//! Durable scope delivery, current registry binding checks, and permission-gated remote reads.
use super::*;
use crate::network::transport::Handle;
use libp2p::PeerId;
use red_leaves_protocol::delivery::{
    CHUNK, Notice, Request as NetRequest, Response as NetResponse,
};

fn network(state: &AppState) -> Result<Handle, ApiError> {
    service(state)?.runtime.network().ok_or_else(unavailable)
}
fn cid(id: &str) -> Result<ContentId, ApiError> {
    ContentId::from_slice(&bytes(id)?).map_err(|_| bad())
}
fn peer(value: &str) -> Result<PeerId, ApiError> {
    value.parse().map_err(|_| bad())
}
fn aad(id: &str, viewer: &str) -> String {
    format!("red-leaves/delivery/1/{id}/{viewer}")
}
async fn profile(
    state: &AppState,
    owner: &str,
) -> Result<crate::posts::registry::Profile, ApiError> {
    service(state)?
        .chain
        .as_ref()
        .ok_or_else(unavailable)?
        .profile(address(owner)?)
        .await
        .map_err(|_| unavailable())
}
pub(super) async fn authorize_peer(
    state: &AppState,
    who: Address,
    remote: PeerId,
) -> Result<Vec<u8>, ApiError> {
    let profile = profile(state, &wallet(who)).await?;
    if peer(&profile.peer)? != remote {
        return Err(forbidden());
    }
    Ok(profile.public_key)
}
async fn local_owners(state: &AppState) -> Result<Vec<String>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT owner FROM social_accounts UNION SELECT address FROM auth_sessions LIMIT 1000",
    )
    .fetch_all(&state.pool)
    .await?)
}
async fn eligible(
    state: &AppState,
    viewer: &str,
    notice: &Notice,
    directed: bool,
) -> Result<bool, ApiError> {
    if viewer == wallet(notice.author) {
        return Ok(false);
    }
    let blocked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_blocks WHERE owner=? AND target=?")
            .bind(viewer)
            .bind(wallet(notice.author))
            .fetch_one(&state.pool)
            .await?;
    if blocked > 0 {
        return Ok(false);
    }
    let prefs = get_preferences(state, viewer).await?;
    let follows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_follows WHERE owner=? AND target=?")
            .bind(viewer)
            .bind(wallet(notice.author))
            .fetch_one(&state.pool)
            .await?;
    let follower: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_follows WHERE owner=? AND target=?")
            .bind(wallet(notice.author))
            .bind(viewer)
            .fetch_one(&state.pool)
            .await?;
    Ok(
        (prefs.broadcasts && notice.distribution == Distribution::Broadcast)
            || (prefs.following && follows > 0)
            || (prefs.followers && follower > 0)
            || (prefs.direct && directed),
    )
}
async fn notice(state: &AppState, id: &str) -> Result<Notice, ApiError> {
    let row = sqlx::query("SELECT * FROM social_posts WHERE id=? AND published=1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(forbidden)?;
    let distribution: Distribution = serde_json::from_value(json!(
        row.get::<Option<String>, _>("audience_scope")
            .unwrap_or(row.get("distribution"))
    ))
    .map_err(|_| unavailable())?;
    let free = row.get::<String, _>("price_wei") == "0";
    Ok(Notice {
        scope: service(state)?.scope().map_err(|_| unavailable())?,
        id: cid(id)?,
        author: address(&row.get::<String, _>("author"))?,
        source_peer: state.config.peer_id.clone(),
        distribution: distribution.clone(),
        price_wei: row.get("price_wei"),
        market: if free {
            None
        } else {
            row.get::<Option<String>, _>("market")
                .as_deref()
                .map(address)
                .transpose()?
        },
        created_at: row.get::<i64, _>("created_at") as u64,
        title: if free && distribution == Distribution::Broadcast {
            row.get("title")
        } else {
            String::new()
        },
    })
}
async fn check_notice(state: &AppState, source: PeerId, notice: &Notice) -> Result<(), ApiError> {
    notice
        .validate(
            &service(state)?.scope().map_err(|_| unavailable())?,
            now() as u64,
        )
        .map_err(|_| bad())?;
    if peer(&notice.source_peer)? != source {
        return Err(forbidden());
    }
    authorize_peer(state, notice.author, source).await?;
    if notice.price_wei != "0" {
        let chain = service(state)?.chain.as_ref().ok_or_else(unavailable)?;
        if notice.market != chain.market {
            return Err(forbidden());
        }
        let (author, price, _) = chain
            .listing(
                notice.author,
                digest_id(&hex::encode(notice.id.as_bytes()))?,
            )
            .await
            .map_err(|_| unavailable())?;
        // A paid notice is not a verified plaintext post. The author's confirmed listing
        // authorizes advertising this ID/price; verify the wallet's full intent on unlock.
        if author != notice.author || price.to_string() != notice.price_wei {
            return Err(forbidden());
        }
    }
    Ok(())
}
fn open_content(
    state: &AppState,
    viewer: &str,
    notice: &Notice,
    response: &NetResponse,
) -> Result<Intent, ApiError> {
    let NetResponse::Content {
        sealed,
        envelope,
        signature,
        ..
    } = response
    else {
        return Err(unavailable());
    };
    let (secret, _) = service(state)?
        .keys
        .pair(viewer)
        .map_err(|_| unavailable())?;
    let id = hex::encode(notice.id.as_bytes());
    let aad = aad(&id, viewer);
    let key = encryption::unwrap(&secret, envelope, aad.as_bytes()).map_err(|_| forbidden())?;
    let payload = encryption::open(&key, sealed, aad.as_bytes()).map_err(|_| forbidden())?;
    let intent: Intent = canonical::decode(&payload, 32 * 1024).map_err(|_| bad())?;
    intent
        .verify_signature(&bytes(signature)?)
        .map_err(|_| forbidden())?;
    intent
        .scope
        .ensure_matches(&notice.scope)
        .map_err(|_| forbidden())?;
    let Action::Publish { post } = &intent.action else {
        return Err(bad());
    };
    if intent.id().map_err(|_| bad())? != notice.id
        || intent.actor != notice.author
        || intent.nas_peer_id != notice.source_peer
        || post.price_wei != notice.price_wei
        || post.distribution != notice.distribution
        || (post.distribution != Distribution::Broadcast
            && !post.recipients.contains(&address(viewer)?))
    {
        return Err(forbidden());
    }
    Ok(intent)
}
async fn save_inbox(
    state: &AppState,
    viewer: &str,
    notice: &Notice,
    object: Option<String>,
) -> Result<(), ApiError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_inbox WHERE recipient=?")
        .bind(viewer)
        .fetch_one(&state.pool)
        .await?;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_inbox WHERE post_id=? AND recipient=?")
            .bind(hex::encode(notice.id.as_bytes()))
            .bind(viewer)
            .fetch_one(&state.pool)
            .await?;
    if count >= 10_000 && exists == 0 {
        return Err(ApiError(
            StatusCode::INSUFFICIENT_STORAGE,
            "Inbox quota reached",
        ));
    }
    sqlx::query("INSERT INTO social_inbox(post_id,recipient,author,source_peer,notice,received_at,created_at,object_path) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(post_id,recipient) DO UPDATE SET object_path=COALESCE(excluded.object_path,social_inbox.object_path)")
        .bind(hex::encode(notice.id.as_bytes())).bind(viewer).bind(wallet(notice.author)).bind(&notice.source_peer).bind(serde_json::to_string(notice).map_err(|_|bad())?).bind(now()).bind(notice.created_at as i64).bind(object).execute(&state.pool).await?;
    service(state)?.runtime.changed(viewer);
    Ok(())
}
async fn receive(
    state: &AppState,
    source: PeerId,
    notice: &Notice,
    viewer: &str,
) -> Result<NetResponse, ApiError> {
    let owners = local_owners(state).await?;
    if !owners.iter().any(|o| o == viewer)
        || !eligible(
            state,
            viewer,
            notice,
            notice.distribution != Distribution::Broadcast,
        )
        .await?
    {
        return Ok(NetResponse::Ignored);
    }
    check_notice(state, source, notice).await?;
    let p = profile(state, viewer).await?;
    if p.peer != state.config.peer_id
        || p.public_key
            != service(state)?
                .keys
                .pair(viewer)
                .map_err(|_| unavailable())?
                .1
    {
        return Err(forbidden());
    }
    let id = hex::encode(notice.id.as_bytes());
    let previous =
        sqlx::query("SELECT object_path FROM social_inbox WHERE post_id=? AND recipient=?")
            .bind(&id)
            .bind(viewer)
            .fetch_optional(&state.pool)
            .await?;
    if let Some(row) = previous {
        return Ok(NetResponse::Saved {
            body: row.get::<Option<String>, _>("object_path").is_some(),
        });
    }
    let object = if notice.price_wei == "0" {
        let response = network(state)?
            .request(
                source,
                NetRequest::Fetch {
                    id: notice.id,
                    viewer: address(viewer)?,
                },
            )
            .await
            .map_err(|_| unavailable())?;
        let intent = open_content(state, viewer, notice, &response)?;
        if let Action::Publish { post } = &intent.action
            && post.distribution != Distribution::Broadcast
            && !post.recipients.contains(&address(viewer)?)
        {
            return Err(forbidden());
        }
        Some(write_object(service(state)?, &id, &response).map_err(|_| unavailable())?)
    } else {
        None
    };
    // Preferences can change while an RPC or content fetch is in flight.
    if !eligible(
        state,
        viewer,
        notice,
        notice.distribution != Distribution::Broadcast,
    )
    .await?
    {
        return Ok(NetResponse::Ignored);
    }
    let body = object.is_some();
    save_inbox(state, viewer, notice, object).await?;
    Ok(NetResponse::Saved { body })
}

pub(super) async fn remote_row(
    state: &AppState,
    id: &str,
    viewer: &str,
) -> Result<Option<sqlx::sqlite::SqliteRow>, ApiError> {
    let _ = cid(id)?;
    Ok(sqlx::query("SELECT i.* FROM social_inbox i WHERE post_id=? AND recipient=? AND source_peer!=? AND NOT EXISTS(SELECT 1 FROM social_blocks b WHERE b.owner=i.recipient AND b.target=i.author)")
        .bind(id).bind(viewer).bind(&state.config.peer_id).fetch_optional(&state.pool).await?)
}
fn row_notice(row: &sqlx::sqlite::SqliteRow) -> Result<Notice, ApiError> {
    serde_json::from_str(&row.get::<String, _>("notice")).map_err(|_| unavailable())
}
fn cached_content(
    state: &AppState,
    viewer: &str,
    row: &sqlx::sqlite::SqliteRow,
) -> Result<(Intent, NetResponse), ApiError> {
    let path: String = row
        .get::<Option<String>, _>("object_path")
        .ok_or_else(unavailable)?;
    if std::fs::metadata(&path).map_err(|_| unavailable())?.len() > 256 * 1024 {
        return Err(unavailable());
    }
    let response: NetResponse =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
    let intent = open_content(state, viewer, &row_notice(row)?, &response)?;
    Ok((intent, response))
}
pub(super) async fn remote_read(
    state: &AppState,
    id: &str,
    viewer: &str,
    unlock: bool,
) -> Result<Json<Value>, ApiError> {
    let row = remote_row(state, id, viewer).await?.ok_or_else(forbidden)?;
    let notice = row_notice(&row)?;
    if notice.price_wei != "0" && !unlock {
        return Ok(Json(
            json!({"locked":true,"id":id,"title":"Encrypted post","price_wei":notice.price_wei,"market":notice.market.map(wallet),"author":wallet(notice.author)}),
        ));
    }
    if unlock && notice.price_wei != "0" {
        let chain = service(state)?.chain.as_ref().ok_or_else(unavailable)?;
        if notice.market != chain.market
            || !chain
                .has_paid(notice.author, digest_id(id)?, address(viewer)?)
                .await
                .map_err(|_| unavailable())?
        {
            return Err(ApiError(
                StatusCode::PAYMENT_REQUIRED,
                "Payment has not been confirmed on Base Sepolia",
            ));
        }
    }
    let (intent, response) = if row.get::<Option<String>, _>("object_path").is_some() {
        cached_content(state, viewer, &row)?
    } else {
        let response = network(state)?
            .request(
                peer(&notice.source_peer)?,
                NetRequest::Fetch {
                    id: notice.id,
                    viewer: address(viewer)?,
                },
            )
            .await
            .map_err(|_| unavailable())?;
        let intent = open_content(state, viewer, &notice, &response)?;
        let path = write_object(service(state)?, id, &response).map_err(|_| unavailable())?;
        save_inbox(state, viewer, &notice, Some(path)).await?;
        (intent, response)
    };
    let NetResponse::Content {
        signature,
        paid_key,
        ..
    } = response
    else {
        return Err(unavailable());
    };
    Ok(Json(
        json!({"locked":false,"id":id,"intent":intent,"signature":signature,"paid_key_envelope":paid_key}),
    ))
}
pub(super) async fn remote_purchase(
    state: &AppState,
    id: &str,
    viewer: &str,
) -> Result<Json<Value>, ApiError> {
    let row = remote_row(state, id, viewer).await?.ok_or_else(forbidden)?;
    let notice = row_notice(&row)?;
    let chain = service(state)?.chain.as_ref().ok_or_else(unavailable)?;
    if notice.price_wei == "0" || notice.market != chain.market {
        return Err(bad());
    }
    let own = profile(state, viewer).await?;
    encryption::wrap(
        &own.public_key,
        &[0; 32],
        b"red-leaves/pre-purchase-key-check/1",
    )
    .map_err(|_| bad())?;
    let paid = chain
        .has_paid(notice.author, digest_id(id)?, address(viewer)?)
        .await
        .map_err(|_| unavailable())?;
    Ok(Json(
        json!({"paid":paid,"market":notice.market.map(wallet),"price_wei":notice.price_wei,"author":wallet(notice.author)}),
    ))
}

pub(super) async fn remote_media(
    state: &AppState,
    post_id: &str,
    viewer: &str,
    media_id: &str,
) -> Result<Response, ApiError> {
    let readable = remote_read(state, post_id, viewer, true).await?;
    let intent: Intent =
        serde_json::from_value(readable.0["intent"].clone()).map_err(|_| unavailable())?;
    let Action::Publish { post } = &intent.action else {
        return Err(bad());
    };
    let media = post
        .media
        .iter()
        .find(|file| file.id == media_id)
        .ok_or_else(forbidden)?;
    let source = peer(&intent.nas_peer_id)?;
    let handle = network(state)?;
    let response = handle
        .request(
            source,
            NetRequest::MediaStart {
                id: cid(post_id)?,
                viewer: address(viewer)?,
                media_id: media_id.into(),
            },
        )
        .await
        .map_err(|_| unavailable())?;
    let NetResponse::Media {
        token,
        envelope,
        nonce,
        size,
    } = response
    else {
        return Err(unavailable());
    };
    if size != media.size + 16 || nonce.len() != 12 || token.len() != 64 {
        return Err(bad());
    }
    let mut ciphertext = Vec::with_capacity(size as usize);
    for offset in (0..size).step_by(CHUNK) {
        let response = handle
            .request(
                source,
                NetRequest::MediaChunk {
                    token: token.clone(),
                    offset,
                },
            )
            .await
            .map_err(|_| unavailable())?;
        let NetResponse::Chunk { bytes } = response else {
            return Err(unavailable());
        };
        if bytes.len() != (size - offset).min(CHUNK as u64) as usize {
            return Err(bad());
        }
        ciphertext.extend_from_slice(&bytes);
    }
    let aad = format!("red-leaves/media/1/{}/{media_id}", wallet(intent.actor));
    let (secret, _) = service(state)?
        .keys
        .pair(viewer)
        .map_err(|_| unavailable())?;
    let key = encryption::unwrap(&secret, &envelope, aad.as_bytes()).map_err(|_| unavailable())?;
    let plaintext = encryption::open(&key, &Sealed { nonce, ciphertext }, aad.as_bytes())
        .map_err(|_| unavailable())?;
    if hex::encode(ContentId::of(&plaintext).as_bytes()) != media.digest {
        return Err(unavailable());
    }
    Ok((
        [
            (header::CONTENT_TYPE, media.mime.as_str()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        plaintext.to_vec(),
    )
        .into_response())
}

mod server;
mod worker;
use server::Server;
pub(in crate::api) use worker::start;
