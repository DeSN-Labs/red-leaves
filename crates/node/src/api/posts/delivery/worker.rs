use super::*;
use crate::network::transport::Incoming;
use std::collections::HashMap;
use tokio::sync::mpsc;
async fn deliver_local(
    state: &AppState,
    notice: &Notice,
    target: &str,
) -> Result<NetResponse, ApiError> {
    if !eligible(
        state,
        target,
        notice,
        notice.distribution != Distribution::Broadcast,
    )
    .await?
    {
        return Ok(NetResponse::Ignored);
    }
    let local = local_owners(state).await?;
    if !local.iter().any(|o| o == target) {
        return Ok(NetResponse::Ignored);
    }
    save_inbox(state, target, notice, None).await?;
    Ok(NetResponse::Saved {
        body: notice.price_wei == "0",
    })
}
async fn deliver_post(state: &AppState, id: &str, target: &str) -> Result<&'static str, ApiError> {
    let n = notice(state, id).await?;
    if target == "broadcast" {
        for owner in local_owners(state).await? {
            deliver_local(state, &n, &owner).await?;
        }
        network(state)?
            .publish(n)
            .await
            .map_err(|_| unavailable())?;
        return Ok("announced");
    }
    let p = profile(state, target).await?;
    let response = if p.peer == state.config.peer_id {
        deliver_local(state, &n, target).await?
    } else {
        network(state)?
            .request(
                peer(&p.peer)?,
                NetRequest::Offer {
                    notice: n,
                    recipient: address(target)?,
                },
            )
            .await
            .map_err(|_| unavailable())?
    };
    match response {
        NetResponse::Saved { body: true } => Ok("delivered"),
        NetResponse::Saved { body: false } => Ok("notified"),
        NetResponse::Ignored => Ok("filtered"),
        _ => Err(unavailable()),
    }
}
async fn process_post(state: AppState, id: String, target: String, attempts: i64) {
    let result = deliver_post(&state, &id, &target).await;
    let (status, next, error) = match result {
        Ok("announced") => ("announced", now() + 120, None),
        Ok(status) => (status, 0, None),
        Err(_) => (
            "retrying",
            now() + (2i64.pow(attempts.min(8) as u32) * 2).min(300),
            Some("Target unavailable; delivery will retry"),
        ),
    };
    if error.is_some() {
        if attempts == 0 {
            tracing::warn!(post_id=%id,attempt=attempts+1,next_attempt=next,"Post delivery is waiting for a reachable target");
        } else {
            tracing::debug!(post_id=%id,attempt=attempts+1,next_attempt=next,"Post delivery retry scheduled");
        }
    } else {
        tracing::info!(post_id=%id,delivery_state=status,attempt=attempts+1,"Post delivery progress updated");
    }
    if sqlx::query("UPDATE social_outbox SET state=?,next_attempt=?,attempts=attempts+1,last_error=? WHERE post_id=? AND target=?")
        .bind(status).bind(next).bind(error).bind(&id).bind(&target).execute(&state.pool).await.is_err(){tracing::error!("Unable to persist post delivery progress");}
    if let Ok(n) = notice(&state, &id).await
        && let Ok(service) = service(&state)
    {
        service.runtime.changed(&wallet(n.author));
    }
}
async fn process_follow(
    state: AppState,
    owner: String,
    target: String,
    payload: String,
    signature: String,
    attempts: i64,
) {
    let result = async {
        let p = profile(&state, &target).await?;
        if p.peer != state.config.peer_id {
            let result = network(&state)?
                .request(
                    peer(&p.peer)?,
                    NetRequest::Follow {
                        payload: bytes(&payload)?,
                        signature: bytes(&signature)?,
                    },
                )
                .await
                .map_err(|_| unavailable())?;
            if !matches!(result, NetResponse::Saved { .. } | NetResponse::Ignored) {
                return Err(unavailable());
            }
        }
        Ok::<_, ApiError>(())
    }
    .await;
    if result.is_err() {
        tracing::debug!(
            attempt = attempts + 1,
            "Follow synchronization retry scheduled"
        );
    } else {
        tracing::debug!("Follow synchronization completed");
    }
    if sqlx::query("UPDATE social_follow_sync SET state=?,next_attempt=?,attempts=attempts+1 WHERE owner=? AND target=? AND payload=?")
        .bind(if result.is_ok(){"delivered"}else{"retrying"}).bind(now()+(2i64.pow(attempts.min(8) as u32)*2).min(300)).bind(owner).bind(target).bind(payload).execute(&state.pool).await.is_err(){tracing::error!("Unable to persist follow synchronization progress");}
}
async fn sync_public(state: AppState, remote: PeerId) {
    let Ok(handle) = network(&state) else {
        return;
    };
    for offset in (0..256).step_by(32) {
        let Ok(NetResponse::Notices { notices }) =
            handle.request(remote, NetRequest::Sync { offset }).await
        else {
            return;
        };
        if notices.is_empty() {
            break;
        }
        for notice in notices {
            if notice.source_peer != remote.to_string()
                || notice.distribution != Distribution::Broadcast
            {
                continue;
            }
            let Ok(owners) = local_owners(&state).await else {
                return;
            };
            for viewer in owners {
                let _ = receive(&state, remote, &notice, &viewer).await;
            }
        }
    }
}
pub(in crate::api) fn start(
    state: AppState,
    net: Option<(Handle, mpsc::Receiver<Incoming>)>,
) -> tokio::task::JoinHandle<()> {
    let (handle, mut incoming) = match net {
        Some((handle, incoming)) => (Some(handle), incoming),
        None => (None, mpsc::channel(1).1),
    };
    if let Ok(service) = service(&state)
        && let Ok(mut slot) = service.runtime.network.lock()
    {
        *slot = handle;
    }
    tokio::spawn(async move {
        let server = Server::new(state.clone());
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut work = tokio::task::JoinSet::new();
        let mut inbound = tokio::task::JoinSet::new();
        let mut inflight = std::collections::HashSet::new();
        let mut sync_times: HashMap<PeerId, i64> = HashMap::new();
        loop {
            tokio::select! {
                _=tick.tick()=>{
                    if work.len()>=4{continue;}
                    let rows=sqlx::query("SELECT post_id,target,attempts FROM social_outbox WHERE state IN ('pending_network','retrying','announced') AND next_attempt<=? ORDER BY next_attempt,post_id LIMIT 8").bind(now()).fetch_all(&state.pool).await;
                    if let Ok(rows)=rows{for row in rows{if work.len()>=4{break;}let id:String=row.get("post_id");let target:String=row.get("target");let key=format!("post/{id}/{target}");if !inflight.insert(key.clone()){continue;}let state=state.clone();work.spawn(async move{process_post(state,id,target,row.get("attempts")).await;key});}}else{tracing::error!("Unable to load pending post deliveries");}
                    if work.len()<4 {
                        let rows=sqlx::query("SELECT * FROM social_follow_sync WHERE state IN ('pending','retrying') AND next_attempt<=? ORDER BY next_attempt LIMIT 4").bind(now()).fetch_all(&state.pool).await;
                        if let Ok(rows)=rows{for row in rows{if work.len()>=4{break;}let owner:String=row.get("owner");let target:String=row.get("target");let key=format!("follow/{owner}/{target}");if !inflight.insert(key.clone()){continue;}let state=state.clone();work.spawn(async move{process_follow(state,owner,target,row.get("payload"),row.get("signature"),row.get("attempts")).await;key});}}else{tracing::error!("Unable to load pending follow synchronization");}
                    }
                }
                Some(result)=work.join_next()=>{match result{Ok(key)=>{inflight.remove(&key);},Err(_)=>{tracing::error!("Delivery worker task failed");}}},
                Some(result)=inbound.join_next()=>{if result.is_err(){tracing::error!("Inbound delivery task failed");}},
                Some(event)=incoming.recv()=>{
                    if inbound.len()>=8{if let Incoming::Request{reply,..}=event{let _=reply.send(NetResponse::Error{code:"busy".into()});}continue;}
                    match event {
                        Incoming::Request{peer,request,reply}=>{let server=server.clone();inbound.spawn(async move{let result=tokio::time::timeout(Duration::from_secs(30),server.request(peer,request)).await;let response=match result{Ok(Ok(response))=>response,Ok(Err(ApiError(code,_)))=>NetResponse::Error{code:code.as_u16().to_string()},Err(_)=>NetResponse::Error{code:"timeout".into()}};let _=reply.send(response);});}
                        Incoming::Broadcast{source,notice}=>{if notice.distribution!=Distribution::Broadcast{continue;}let state=state.clone();inbound.spawn(async move{if let Ok(owners)=local_owners(&state).await{for owner in owners{let _=receive(&state,source,&notice,&owner).await;}}});}
                        Incoming::Connected(remote)=>{if sync_times.get(&remote).is_some_and(|time|now()-time<60){continue;}sync_times.retain(|_,time|now()-*time<120);if sync_times.len()>=128{continue;}sync_times.insert(remote,now());let state=state.clone();inbound.spawn(async move{sync_public(state,remote).await;});}
                    }
                }
            }
        }
    })
}
