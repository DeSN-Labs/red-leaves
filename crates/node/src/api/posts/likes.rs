//! Authoritative likes stay at the author's NAS; remote statistics are never served from a cache.
use super::*;
use delivery::{authorize_peer, cid, network, peer, profile};
use libp2p::PeerId;
use red_leaves_protocol::delivery::{Request as NetRequest, Response as NetResponse};

pub(super) async fn held_author(
    state: &AppState,
    id: &str,
    viewer: &str,
) -> Result<String, ApiError> {
    let _ = cid(id)?;
    if let Some(row) = delivery::remote_row(state, id, viewer).await? {
        // Validate the signed cached post before routing an interaction by its author.
        let _ = delivery::cached_content(state, viewer, &row)?;
        return Ok(row.get("author"));
    }
    Ok(load(state, id, viewer).await?.get("author"))
}
async fn author_access(state: &AppState, id: &str, viewer: &str) -> Result<String, ApiError> {
    let _ = cid(id)?;
    let author: String =
        sqlx::query_scalar("SELECT author FROM social_posts WHERE id=? AND published=1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(forbidden)?;
    if profile(state, &author).await?.peer != state.config.peer_id {
        return Err(forbidden());
    }
    check_access(state, id, viewer).await?;
    Ok(author)
}
macro_rules! access_sql { () => { concat!("SELECT p.id FROM (",super::access::allowed_sql!(),") p WHERE NOT EXISTS(SELECT 1 FROM social_like_denials d WHERE d.post_id=p.id AND d.viewer=?)") }; }
async fn check_access(state: &AppState, id: &str, viewer: &str) -> Result<(), ApiError> {
    let allowed: i64 = sqlx::query_scalar(concat!("SELECT EXISTS(", access_sql!(), ")"))
        .bind(id)
        .bind(viewer)
        .bind(viewer)
        .fetch_one(&state.pool)
        .await?;
    if allowed != 1 {
        return Err(forbidden());
    }
    Ok(())
}
fn context(id: &str, viewer: &str) -> String {
    format!("red-leaves/likes/1/{id}/{viewer}")
}

pub(super) async fn receive(
    state: &AppState,
    remote: PeerId,
    payload: &[u8],
    signature: &[u8],
) -> Result<NetResponse, ApiError> {
    let intent: Intent = canonical::decode(payload, 32 * 1024).map_err(|_| bad())?;
    intent
        .verify_signature(signature)
        .map_err(|_| forbidden())?;
    intent
        .scope
        .ensure_matches(&service(state)?.scope().map_err(|_| unavailable())?)
        .map_err(|_| forbidden())?;
    if intent.nas_peer_id != remote.to_string() || intent.issued_at > now() as u64 + 120 {
        return Err(bad());
    }
    let Action::Like { post_id, enabled } = intent.action else {
        return Err(bad());
    };
    authorize_peer(state, intent.actor, remote).await?;
    let id = hex::encode(post_id.as_bytes());
    let viewer = wallet(intent.actor);
    let author = author_access(state, &id, &viewer).await?;
    let nonce = hex::encode(intent.nonce.0);
    let mut tx = state.pool.begin().await?;
    // The write and admission bound share a transaction; replacing a wallet's state does not consume another slot.
    let result = sqlx::query(concat!("INSERT INTO social_likes(post_id,owner,enabled,issued_at,nonce,payload,signature)
        SELECT ?,?,?,?,?,?,? WHERE EXISTS(", access_sql!(), ") AND ((SELECT COUNT(*) FROM social_likes WHERE post_id=?) < 10000 OR EXISTS(SELECT 1 FROM social_likes WHERE post_id=? AND owner=?))
        ON CONFLICT(post_id,owner) DO UPDATE SET enabled=excluded.enabled,issued_at=excluded.issued_at,nonce=excluded.nonce,payload=excluded.payload,signature=excluded.signature
        WHERE (social_likes.issued_at,social_likes.nonce) < (excluded.issued_at,excluded.nonce)"))
        .bind(&id).bind(&viewer).bind(enabled).bind(intent.issued_at as i64).bind(&nonce).bind(format!("0x{}",hex::encode(payload))).bind(format!("0x{}",hex::encode(signature)))
        .bind(&id).bind(&viewer).bind(&viewer).bind(&id).bind(&id).bind(&viewer).execute(&mut *tx).await?;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_likes WHERE post_id=? AND owner=?")
            .bind(&id)
            .bind(&viewer)
            .fetch_one(&mut *tx)
            .await?;
    if exists != 1 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Like limit reached for this post",
        ));
    }
    if result.rows_affected() == 0 {
        let row =
            sqlx::query("SELECT issued_at,nonce FROM social_likes WHERE post_id=? AND owner=?")
                .bind(&id)
                .bind(&viewer)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(forbidden)?;
        if (
            row.get::<i64, _>("issued_at"),
            row.get::<String, _>("nonce"),
        ) < (intent.issued_at as i64, nonce)
        {
            return Err(forbidden());
        }
    }
    tx.commit().await?;
    if result.rows_affected() != 0 {
        service(state)?.runtime.changed(&author);
    }
    Ok(NetResponse::Saved { body: false })
}
async fn summary(state: &AppState, id: &str, viewer: &str) -> Result<Value, ApiError> {
    // Counts and current author policy are read in one SQLite snapshot. Never serve a stale cache.
    let row = sqlx::query(concat!("SELECT (SELECT COUNT(*) FROM social_likes WHERE post_id=? AND enabled=1) AS count, EXISTS(SELECT 1 FROM social_likes WHERE post_id=? AND owner=? AND enabled=1) AS liked FROM (", access_sql!(), ")"))
        .bind(id).bind(id).bind(viewer).bind(id).bind(viewer).bind(viewer).fetch_optional(&state.pool).await?.ok_or_else(forbidden)?;
    Ok(json!({"count":row.get::<i64,_>("count"),"liked":row.get::<bool,_>("liked")}))
}
pub(super) async fn serve(
    state: &AppState,
    remote: PeerId,
    id: ContentId,
    viewer: Address,
) -> Result<NetResponse, ApiError> {
    let public = authorize_peer(state, viewer, remote).await?;
    let viewer = wallet(viewer);
    let id = hex::encode(id.as_bytes());
    author_access(state, &id, &viewer).await?;
    let value = summary(state, &id, &viewer).await?;
    let key = encryption::random_key().map_err(|_| unavailable())?;
    let aad = context(&id, &viewer);
    Ok(NetResponse::Likes {
        sealed: encryption::seal(
            &key,
            &serde_json::to_vec(&value).map_err(|_| unavailable())?,
            aad.as_bytes(),
        )
        .map_err(|_| unavailable())?,
        envelope: encryption::wrap(&public, &key, aad.as_bytes()).map_err(|_| unavailable())?,
    })
}
pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let author = held_author(&state, &id, &viewer).await?;
    let target = profile(&state, &author).await?;
    let mut value = if target.peer == state.config.peer_id {
        author_access(&state, &id, &viewer).await?;
        summary(&state, &id, &viewer).await?
    } else {
        let response = network(&state)?
            .request(
                peer(&target.peer)?,
                NetRequest::Likes {
                    id: cid(&id)?,
                    viewer: address(&viewer)?,
                },
            )
            .await
            .map_err(|_| unavailable())?;
        if matches!(&response, NetResponse::Error { code } if code=="404" || code=="403") {
            return Err(forbidden());
        }
        let NetResponse::Likes { sealed, envelope } = response else {
            return Err(unavailable());
        };
        let (secret, _) = service(&state)?
            .keys
            .pair(&viewer)
            .map_err(|_| unavailable())?;
        let aad = context(&id, &viewer);
        let key =
            encryption::unwrap(&secret, &envelope, aad.as_bytes()).map_err(|_| forbidden())?;
        let bytes = encryption::open(&key, &sealed, aad.as_bytes()).map_err(|_| forbidden())?;
        let result: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if result["count"].as_u64().is_none_or(|count| count > 10000)
            || !result["liked"].is_boolean()
        {
            return Err(unavailable());
        }
        result
    };
    let pending = sqlx::query("SELECT enabled,state FROM social_like_sync WHERE post_id=? AND owner=? AND state!='delivered'").bind(&id).bind(&viewer).fetch_optional(&state.pool).await?;
    value["pending"] = pending.map(|row|json!({"enabled":row.get::<bool,_>("enabled"),"state":row.get::<String,_>("state")})).unwrap_or(Value::Null);
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Access {
    target: String,
    allowed: bool,
}
pub(super) async fn access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Access>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let row = load(&state, &id, &viewer).await?;
    if row.get::<String, _>("author") != viewer
        || profile(&state, &viewer).await?.peer != state.config.peer_id
    {
        return Err(forbidden());
    }
    let target = wallet(address(&input.target)?);
    if target == viewer || address(&target)?.0 == [0; 20] {
        return Err(bad());
    }
    if input.allowed {
        sqlx::query("DELETE FROM social_like_denials WHERE post_id=? AND viewer=?")
            .bind(&id)
            .bind(target)
            .execute(&state.pool)
            .await?;
    } else {
        let saved = sqlx::query("INSERT OR IGNORE INTO social_like_denials(post_id,viewer) SELECT ?,? WHERE (SELECT COUNT(*) FROM social_like_denials WHERE post_id=?)<1000 OR EXISTS(SELECT 1 FROM social_like_denials WHERE post_id=? AND viewer=?)")
            .bind(&id).bind(&target).bind(&id).bind(&id).bind(&target).execute(&state.pool).await?;
        if saved.rows_affected() == 0 {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM social_like_denials WHERE post_id=? AND viewer=?",
            )
            .bind(&id)
            .bind(target)
            .fetch_one(&state.pool)
            .await?;
            if exists == 0 {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Likes access limit reached",
                ));
            }
        }
    }
    Ok(Json(json!({"status":"saved"})))
}
pub(super) async fn process(
    state: AppState,
    post_id: String,
    owner: String,
    author: String,
    payload: String,
    signature: String,
    attempts: i64,
) {
    let result = async {
        let target = profile(&state, &author).await?;
        let response = if target.peer == state.config.peer_id {
            receive(
                &state,
                peer(&state.config.peer_id)?,
                &bytes(&payload)?,
                &bytes(&signature)?,
            )
            .await?
        } else {
            network(&state)?
                .request(
                    peer(&target.peer)?,
                    NetRequest::Like {
                        payload: bytes(&payload)?,
                        signature: bytes(&signature)?,
                    },
                )
                .await
                .map_err(|_| unavailable())?
        };
        match response {
            NetResponse::Saved { .. } | NetResponse::Ignored => Ok("delivered"),
            NetResponse::Error { code } if code == "404" || code == "403" => Ok("denied"),
            _ => Err(unavailable()),
        }
    }
    .await;
    let status = match result {
        Ok(status) => status,
        Err(ApiError(StatusCode::NOT_FOUND | StatusCode::FORBIDDEN, _)) => "denied",
        Err(_) => "retrying",
    };
    if sqlx::query("UPDATE social_like_sync SET state=?,attempts=attempts+1,next_attempt=? WHERE post_id=? AND owner=? AND payload=?")
        .bind(status).bind(now()+(2i64.pow(attempts.min(8) as u32)*2).min(300)).bind(post_id).bind(&owner).bind(payload).execute(&state.pool).await.is_err() { tracing::error!("Unable to persist like synchronization progress"); }
    if let Ok(service) = service(&state) {
        service.runtime.changed(&owner);
    }
}
