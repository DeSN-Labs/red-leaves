use super::*;
use axum::response::sse::{Event, Sse};
use futures::stream;
use std::convert::Infallible;

struct Watcher {
    service: Arc<crate::posts::Service>,
    owner: String,
}
impl Drop for Watcher {
    fn drop(&mut self) {
        if let Ok(mut watchers) = self.service.runtime.watchers.lock()
            && let Some(count) = watchers.get_mut(&self.owner)
        {
            *count -= 1;
            if *count == 0 {
                watchers.remove(&self.owner);
            }
        }
    }
}
pub(super) async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let token_hash = identity::hash(bearer(&headers)?);
    let viewer = owner(&state, headers).await?;
    let service = state.config.posts.clone().ok_or_else(unavailable)?;
    {
        let mut watchers = service.runtime.watchers.lock().map_err(|_| unavailable())?;
        if watchers.values().sum::<usize>() >= 128
            || watchers.get(&viewer).copied().unwrap_or(0) >= 4
        {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many live subscriptions",
            ));
        }
        *watchers.entry(viewer.clone()).or_default() += 1;
    }
    let receiver = service.runtime.changes.subscribe();
    let guard = Watcher {
        service,
        owner: viewer.clone(),
    };
    let ticks = tokio::time::interval(Duration::from_secs(10));
    let stream = stream::unfold(
        (state, viewer, token_hash, receiver, ticks, guard, true),
        |(state, viewer, token_hash, mut receiver, mut ticks, guard, mut first)| async move {
            loop {
                if guard
                    .service
                    .runtime
                    .stopping
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return None;
                }
                let event = tokio::select! {
                    _=guard.service.runtime.shutdown.notified()=>return None,
                    _=ticks.tick()=>{
                        let active=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM auth_sessions WHERE token_hash=? AND address=? AND expires_at>?").bind(&token_hash).bind(&viewer).bind(now()).fetch_one(&state.pool).await.ok()?;
                        if active!=1{return None;}
                        if first {first=false;Event::default().event("sync").data("{}")}else{Event::default().comment("keepalive")}
                    }
                    changed=receiver.recv()=>{
                        match changed {
                            Ok(owner) if owner==viewer=>{
                                let active=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM auth_sessions WHERE token_hash=? AND address=? AND expires_at>?").bind(&token_hash).bind(&viewer).bind(now()).fetch_one(&state.pool).await.ok()?;
                                if active!=1{return None;}
                                Event::default().event("update").data("{}")
                            },
                            Ok(_)=>continue,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>Event::default().event("sync").data("{}"),
                            Err(_)=>return None,
                        }
                    }
                };
                return Some((
                    Ok(event),
                    (state, viewer, token_hash, receiver, ticks, guard, first),
                ));
            }
        },
    );
    Ok(Sse::new(stream))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Block {
    target: String,
    blocked: bool,
}
pub(super) async fn blocks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let blocked: Vec<String> = sqlx::query_scalar(
        "SELECT target FROM social_blocks WHERE owner=? ORDER BY target LIMIT 1000",
    )
    .bind(viewer)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"blocked":blocked})))
}
pub(super) async fn save_block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Block>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let target = wallet(address(&input.target)?);
    if target == viewer || address(&target)?.0 == [0; 20] {
        return Err(bad());
    }
    if input.blocked {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_blocks WHERE owner=?")
            .bind(&viewer)
            .fetch_one(&state.pool)
            .await?;
        if count >= 1000 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Block list limit reached",
            ));
        }
        sqlx::query("INSERT OR IGNORE INTO social_blocks(owner,target) VALUES(?,?)")
            .bind(&viewer)
            .bind(target)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM social_blocks WHERE owner=? AND target=?")
            .bind(&viewer)
            .bind(target)
            .execute(&state.pool)
            .await?;
    }
    service(&state)?.runtime.changed(&viewer);
    Ok(Json(json!({"status":"saved"})))
}
pub(super) async fn progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let row = load(&state, &id, &viewer).await?;
    if row.get::<String, _>("author") != viewer {
        return Err(forbidden());
    }
    let rows=sqlx::query("SELECT target,state,attempts,next_attempt,last_error FROM social_outbox WHERE post_id=? ORDER BY target").bind(id).fetch_all(&state.pool).await?;
    let targets:Vec<Value>=rows.iter().map(|r|json!({"target":r.get::<String,_>("target"),"state":r.get::<String,_>("state"),"attempts":r.get::<i64,_>("attempts"),"next_attempt":r.get::<i64,_>("next_attempt"),"last_error":r.get::<Option<String>,_>("last_error")})).collect();
    Ok(Json(json!({"targets":targets})))
}
