//! Authenticated publishing. All outward delivery is gated by a signed Publish intent.
use super::*;
use axum::extract::{Path, Query};
use futures::{StreamExt, TryStreamExt, stream};
use red_leaves_protocol::{
    Address, ContentId, Nonce, canonical,
    encryption::{self, KeyEnvelope, Sealed},
    social::{Action, Distribution, Intent},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .merge(media::routes())
        .route("/api/posts/{id}/access", get(access::get).post(access::set))
        .route("/api/posts/{id}/likes", get(likes::read))
        .route("/api/posts/{id}/likes/access", post(likes::access))
        .route("/api/content/key", get(public_key))
        .route("/api/social/prepare", post(prepare))
        .route("/api/social/commit", post(commit))
        .route(
            "/api/social/preferences",
            get(preferences).post(save_preferences),
        )
        .route("/api/social/relationships", get(relationships))
        .route("/api/social/events", get(live::events))
        .route(
            "/api/social/blocks",
            get(live::blocks).post(live::save_block),
        )
        .route("/api/posts", get(timeline))
        .route("/api/posts/{id}", get(read_post))
        .route("/api/posts/{id}/delivery", get(live::progress))
        .layer(DefaultBodyLimit::max(128 * 1024))
}
fn bad() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "Invalid content request")
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Content service unavailable. Check NAS keys, registry RPC and contract configuration.",
    )
}
fn forbidden() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Content not found or access denied")
}
fn service(state: &AppState) -> Result<&crate::posts::Service, ApiError> {
    state.config.posts.as_deref().ok_or_else(unavailable)
}
fn address(s: &str) -> Result<Address, ApiError> {
    identity::address(s).map(Address).map_err(|_| bad())
}
fn wallet(a: Address) -> String {
    format!("0x{}", hex::encode(a.0))
}
fn bytes(s: &str) -> Result<Vec<u8>, ApiError> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).map_err(|_| bad())
}
async fn owner(state: &AppState, headers: HeaderMap) -> Result<String, ApiError> {
    Ok(me(State(state.clone()), headers)
        .await?
        .0
        .address
        .to_lowercase())
}
async fn public_key(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let owner = owner(&state, headers).await?;
    let (_, key) = service(&state)?
        .keys
        .pair(&owner)
        .map_err(|_| unavailable())?;
    Ok(Json(
        json!({"public_key":format!("0x{}",hex::encode(key)),"algorithm":"x25519-hpke-v1","custody":"nas"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    action: Action,
}
async fn validate_audience(
    state: &AppState,
    owner: &str,
    post: &red_leaves_protocol::social::PostDraft,
) -> Result<(), ApiError> {
    let query = match post.distribution {
        Distribution::Following => {
            "SELECT target FROM social_follows WHERE owner=? ORDER BY target LIMIT 33"
        }
        Distribution::Followers => {
            "SELECT owner FROM social_follows WHERE target=? ORDER BY owner LIMIT 33"
        }
        Distribution::Friends => {
            "SELECT target FROM social_follows WHERE owner=? AND EXISTS(SELECT 1 FROM social_follows reverse WHERE reverse.owner=social_follows.target AND reverse.target=social_follows.owner) ORDER BY target LIMIT 33"
        }
        _ => return Ok(()),
    };
    let expected: Vec<String> = sqlx::query_scalar(query)
        .bind(owner)
        .fetch_all(&state.pool)
        .await?;
    let mut actual: Vec<String> = post.recipients.iter().copied().map(wallet).collect();
    actual.sort();
    if actual != expected {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Audience changed or exceeds 32 recipients. Refresh relationships and review recipients again.",
        ));
    }
    Ok(())
}
async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Prepare>,
) -> Result<Json<Value>, ApiError> {
    let owner = owner(&state, headers).await?;
    let service = service(&state)?;
    if let Action::Publish { ref post } = input.action {
        post.validate(address(&owner)?).map_err(|_| bad())?;
        validate_audience(&state, &owner, post).await?;
        media::validate_uploads(&state.pool, &owner, &post.media).await?;
    }
    let mut issued_at = now() as u64;
    if let Action::Like { post_id, .. } = &input.action {
        let id = hex::encode(post_id.as_bytes());
        likes::held_author(&state, &id, &owner).await?;
        let previous: Option<i64> = sqlx::query_scalar("SELECT MAX(issued_at) FROM (SELECT issued_at FROM social_like_sync WHERE post_id=? AND owner=? UNION ALL SELECT issued_at FROM social_likes WHERE post_id=? AND owner=?)")
            .bind(&id).bind(&owner).bind(&id).bind(&owner).fetch_one(&state.pool).await?;
        if let Some(previous) = previous {
            issued_at = issued_at.max(previous as u64 + 1);
        }
        if issued_at > now() as u64 + 120 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many like changes. Retry shortly.",
            ));
        }
    }
    if let Action::Follow { target, .. } = &input.action {
        let previous: Option<i64> = sqlx::query_scalar(
            "SELECT issued_at FROM social_follow_versions WHERE owner=? AND target=?",
        )
        .bind(&owner)
        .bind(wallet(*target))
        .fetch_optional(&state.pool)
        .await?;
        if let Some(previous) = previous {
            issued_at = issued_at.max(previous as u64 + 1);
        }
        if issued_at > now() as u64 + 120 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many follow changes. Retry shortly.",
            ));
        }
    }
    let id = identity::secret().map_err(internal)?;
    let intent = Intent {
        scope: service.scope().map_err(|_| unavailable())?,
        actor: address(&owner)?,
        nas_peer_id: state.config.peer_id.clone(),
        nonce: Nonce(bytes(&id)?.try_into().map_err(|_| bad())?),
        issued_at,
        expires_at: now() as u64 + 900,
        action: input.action,
    };
    let payload = intent.encode().map_err(|_| bad())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM social_challenges WHERE expires_at<=?")
        .bind(now())
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_challenges WHERE owner=?")
        .bind(&owner)
        .fetch_one(&mut *tx)
        .await?;
    if count >= 16 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pending content signatures",
        ));
    }
    sqlx::query("INSERT INTO social_challenges(id,owner,payload_hash,expires_at) VALUES(?,?,?,?)")
        .bind(id)
        .bind(&owner)
        .bind(hex::encode(ContentId::of(&payload).as_bytes()))
        .bind(intent.expires_at as i64)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"intent":intent,"payload":format!("0x{}",hex::encode(&payload)),"payload_hash":format!("0x{}",hex::encode(keccak(&payload))),"signing_hash":format!("0x{}",hex::encode(intent.signing_hash().map_err(|_|bad())?))}),
    ))
}
fn keccak(data: &[u8]) -> [u8; 32] {
    red_leaves_protocol::social::payload_hash(data)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPost {
    version: u32,
    signature: String,
    public_payload: Option<Vec<u8>>,
    transport: Option<Sealed>,
    transport_keys: Vec<(String, KeyEnvelope)>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    payload: String,
    signature: String,
}

async fn commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Commit>,
) -> Result<Json<Value>, ApiError> {
    let owner = owner(&state, headers).await?;
    let service = service(&state)?;
    let scope = service.scope().map_err(|_| unavailable())?;
    let payload = Zeroizing::new(bytes(&input.payload)?);
    let signature = bytes(&input.signature)?;
    let intent = Intent::verify(
        &payload,
        &signature,
        &scope,
        &state.config.peer_id,
        now() as u64,
    )
    .map_err(|_| bad())?;
    if wallet(intent.actor) != owner {
        return Err(forbidden());
    }
    let id = hex::encode(intent.id().map_err(|_| bad())?.as_bytes());
    if let Some(row) = sqlx::query("SELECT author FROM social_posts WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?
    {
        if row.get::<String, _>("author") == owner {
            return post_status(&state, &id).await;
        }
        return Err(forbidden());
    }
    // Precheck before expensive RPC/encryption; final one-use consumption is transactional.
    let challenge = hex::encode(intent.nonce.0);
    let expected = hex::encode(ContentId::of(&payload).as_bytes());
    let exists:i64=sqlx::query_scalar("SELECT COUNT(*) FROM social_challenges WHERE id=? AND owner=? AND payload_hash=? AND expires_at>?").bind(&challenge).bind(&owner).bind(&expected).bind(now()).fetch_one(&state.pool).await?;
    if exists != 1 {
        return Err(bad());
    }
    if matches!(intent.action, Action::Follow { .. } | Action::Like { .. }) {
        let registered = service
            .chain
            .as_ref()
            .ok_or_else(unavailable)?
            .profile(intent.actor)
            .await
            .map_err(|_| unavailable())?;
        if registered.peer != state.config.peer_id {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Social changes require the NAS registered to this wallet",
            ));
        }
    }
    let like_author = if let Action::Like { post_id, .. } = &intent.action {
        Some(likes::held_author(&state, &hex::encode(post_id.as_bytes()), &owner).await?)
    } else {
        None
    };
    let mut post_record = None;
    if let Action::Publish { ref post } = intent.action {
        media::validate_uploads(&state.pool, &owner, &post.media).await?;
        let chain = service.chain.as_ref().ok_or_else(unavailable)?;
        let author = chain
            .profile(intent.actor)
            .await
            .map_err(|_| unavailable())?;
        let (_, local_public) = service.keys.pair(&owner).map_err(|_| unavailable())?;
        if author.peer != state.config.peer_id || author.public_key != local_public {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Register this NAS PeerId and its NAS-held encryption public key before publishing",
            ));
        }
        let aad = format!("red-leaves/post/1/{id}");
        let mut stored = StoredPost {
            version: 2,
            signature: input.signature.clone(),
            public_payload: None,
            transport: None,
            transport_keys: vec![],
        };
        if post.distribution != Distribution::Broadcast {
            let key = encryption::random_key().map_err(|_| unavailable())?;
            stored.transport =
                Some(encryption::seal(&key, &payload, aad.as_bytes()).map_err(|_| unavailable())?);
            stored.transport_keys.push((
                owner.clone(),
                encryption::wrap(&local_public, &key, format!("{aad}/{owner}").as_bytes())
                    .map_err(|_| bad())?,
            ));
            let profiles = stream::iter(post.recipients.iter().copied())
                .map(|target| async move {
                    chain
                        .profile(target)
                        .await
                        .map(|profile| (target, profile))
                        .map_err(|_| unavailable())
                })
                .buffered(4)
                .try_collect::<Vec<_>>()
                .await?;
            for (target, profile) in profiles {
                let target = wallet(target);
                if profile.peer == state.config.peer_id
                    && service.keys.pair(&target).map_err(|_| unavailable())?.1
                        != profile.public_key
                {
                    return Err(ApiError(
                        StatusCode::CONFLICT,
                        "A recipient must update their on-chain key to the key held by this NAS",
                    ));
                }
                stored.transport_keys.push((
                    target.clone(),
                    encryption::wrap(
                        &profile.public_key,
                        &key,
                        format!("{aad}/{target}").as_bytes(),
                    )
                    .map_err(|_| bad())?,
                ));
            }
        } else {
            stored.public_payload = Some(payload.to_vec());
        }
        let path = write_object(service, &id, &stored).map_err(|_| unavailable())?;
        post_record = Some(path);
    }
    let mut tx = state.pool.begin().await?;
    let consumed = sqlx::query(
        "DELETE FROM social_challenges WHERE id=? AND owner=? AND payload_hash=? AND expires_at>?",
    )
    .bind(challenge)
    .bind(&owner)
    .bind(expected)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(bad());
    }
    match &intent.action {
        Action::Like { post_id, enabled } => {
            let post_id = hex::encode(post_id.as_bytes());
            let result = sqlx::query("INSERT INTO social_like_sync(post_id,owner,author,enabled,issued_at,payload,signature) SELECT ?,?,?,?,?,?,? WHERE (SELECT COUNT(*) FROM social_like_sync)<10000 OR EXISTS(SELECT 1 FROM social_like_sync WHERE post_id=? AND owner=?) ON CONFLICT(post_id,owner) DO UPDATE SET enabled=excluded.enabled,issued_at=excluded.issued_at,payload=excluded.payload,signature=excluded.signature,state='pending',attempts=0,next_attempt=0 WHERE social_like_sync.issued_at < excluded.issued_at")
                .bind(&post_id).bind(&owner).bind(like_author.ok_or_else(bad)?).bind(enabled).bind(intent.issued_at as i64).bind(&input.payload).bind(&input.signature).bind(&post_id).bind(&owner).execute(&mut *tx).await?;
            if result.rows_affected() != 1 {
                let exists: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM social_like_sync WHERE post_id=? AND owner=?",
                )
                .bind(&post_id)
                .bind(&owner)
                .fetch_one(&mut *tx)
                .await?;
                return Err(if exists == 1 {
                    ApiError(
                        StatusCode::CONFLICT,
                        "A newer like change already exists. Review and sign again.",
                    )
                } else {
                    ApiError(
                        StatusCode::TOO_MANY_REQUESTS,
                        "Like synchronization limit reached",
                    )
                });
            }
        }
        Action::Follow { target, enabled } => {
            let previous: Option<i64> = sqlx::query_scalar(
                "SELECT issued_at FROM social_follow_versions WHERE owner=? AND target=?",
            )
            .bind(&owner)
            .bind(wallet(*target))
            .fetch_optional(&mut *tx)
            .await?;
            if previous.is_some_and(|v| v >= intent.issued_at as i64) {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "A newer follow change already exists. Review and sign again.",
                ));
            }
            sqlx::query("INSERT INTO social_follow_versions(owner,target,issued_at,nonce) VALUES(?,?,?,?) ON CONFLICT(owner,target) DO UPDATE SET issued_at=excluded.issued_at,nonce=excluded.nonce")
                .bind(&owner).bind(wallet(*target)).bind(intent.issued_at as i64).bind(hex::encode(intent.nonce.0)).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO social_follow_sync(owner,target,payload,signature,issued_at,nonce) VALUES(?,?,?,?,?,?) ON CONFLICT(owner,target) DO UPDATE SET payload=excluded.payload,signature=excluded.signature,issued_at=excluded.issued_at,nonce=excluded.nonce,state='pending',attempts=0,next_attempt=0")
                .bind(&owner).bind(wallet(*target)).bind(&input.payload).bind(format!("0x{}",hex::encode(&signature))).bind(intent.issued_at as i64).bind(hex::encode(intent.nonce.0)).execute(&mut *tx).await?;
            if *enabled {
                sqlx::query("INSERT OR IGNORE INTO social_follows(owner,target) VALUES(?,?)")
                    .bind(&owner)
                    .bind(wallet(*target))
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("DELETE FROM social_follows WHERE owner=? AND target=?")
                    .bind(&owner)
                    .bind(wallet(*target))
                    .execute(&mut *tx)
                    .await?;
            }
        }
        Action::Publish { post } => {
            let path = post_record.ok_or_else(unavailable)?;
            sqlx::query("INSERT INTO social_posts(id,author,title,distribution,created_at,published,object_path,audience_scope) VALUES(?,?,?,?,?,1,?,?)")
                .bind(&id).bind(&owner).bind(if post.distribution!=Distribution::Broadcast {"Encrypted post"}else{&post.title}).bind(if post.distribution==Distribution::Broadcast {"broadcast"}else{"directed"}).bind(now()).bind(path).bind(serde_json::to_value(&post.distribution).map_err(|_|bad())?.as_str().ok_or_else(bad)?).execute(&mut *tx).await?;
            let level = match post.distribution {
                Distribution::Broadcast => "public",
                Distribution::Following => "following",
                Distribution::Followers => "followers",
                _ => "directed",
            };
            sqlx::query("INSERT INTO social_post_access(post_id,level) VALUES(?,?)")
                .bind(&id)
                .bind(level)
                .execute(&mut *tx)
                .await?;
            if level == "directed" {
                for recipient in &post.recipients {
                    sqlx::query(
                        "INSERT INTO social_post_access_recipients(post_id,viewer) VALUES(?,?)",
                    )
                    .bind(&id)
                    .bind(wallet(*recipient))
                    .execute(&mut *tx)
                    .await?;
                }
            }
            for (position, file) in post.media.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO social_post_media(post_id,media_id,position) VALUES(?,?,?)",
                )
                .bind(&id)
                .bind(&file.id)
                .bind(position as i64)
                .execute(&mut *tx)
                .await?;
            }
            for target in &post.recipients {
                sqlx::query("INSERT INTO social_recipients(post_id,recipient) VALUES(?,?)")
                    .bind(&id)
                    .bind(wallet(*target))
                    .execute(&mut *tx)
                    .await?;
            }
            queue(&mut tx, &id, post).await?;
        }
    }
    tx.commit().await?;
    service.runtime.changed(&owner);
    service.runtime.wake.notify_one();
    if matches!(intent.action, Action::Publish { .. }) {
        post_status(&state, &id).await
    } else {
        Ok(Json(json!({"status":"saved"})))
    }
}

fn write_object(
    service: &crate::posts::Service,
    id: &str,
    stored: &impl Serialize,
) -> Result<String, identity::Error> {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
    };
    fs::create_dir_all(&service.objects)?;
    let path = service
        .objects
        .join(format!("{id}-{}.json", identity::secret()?));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&serde_json::to_vec(stored)?)?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::File::open(&service.objects)?.sync_all()?;
    Ok(path.to_str().ok_or("Invalid content path")?.into())
}
async fn queue(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    post: &red_leaves_protocol::social::PostDraft,
) -> Result<(), ApiError> {
    let targets = if post.distribution == Distribution::Broadcast {
        vec!["broadcast".into()]
    } else {
        post.recipients.iter().map(|a| wallet(*a)).collect()
    };
    for target in targets {
        sqlx::query("INSERT OR IGNORE INTO social_outbox(post_id,target) VALUES(?,?)")
            .bind(id)
            .bind(target)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
async fn post_status(state: &AppState, id: &str) -> Result<Json<Value>, ApiError> {
    let row = sqlx::query("SELECT author FROM social_posts WHERE id=?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(
        json!({"id":id,"status":"saved_pending_network","author":row.get::<String,_>("author")}),
    ))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    broadcasts: bool,
    following: bool,
    followers: bool,
    direct: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            broadcasts: false,
            following: true,
            followers: false,
            direct: true,
        }
    }
}
async fn get_preferences(state: &AppState, owner: &str) -> Result<Preferences, ApiError> {
    let row = sqlx::query(
        "SELECT broadcasts,following,followers,direct FROM social_preferences WHERE owner=?",
    )
    .bind(owner)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row
        .map(|r| Preferences {
            broadcasts: r.get("broadcasts"),
            following: r.get("following"),
            followers: r.get("followers"),
            direct: r.get("direct"),
        })
        .unwrap_or_default())
}
async fn preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Preferences>, ApiError> {
    let owner = owner(&state, headers).await?;
    Ok(Json(get_preferences(&state, &owner).await?))
}
async fn save_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Preferences>,
) -> Result<Json<Preferences>, ApiError> {
    let owner = owner(&state, headers).await?;
    sqlx::query("INSERT INTO social_preferences(owner,broadcasts,following,followers,direct) VALUES(?,?,?,?,?) ON CONFLICT(owner) DO UPDATE SET broadcasts=excluded.broadcasts,following=excluded.following,followers=excluded.followers,direct=excluded.direct")
        .bind(owner).bind(input.broadcasts).bind(input.following).bind(input.followers).bind(input.direct).execute(&state.pool).await?;
    Ok(Json(input))
}
async fn relationships(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let owner = owner(&state, headers).await?;
    let following: Vec<String> = sqlx::query_scalar(
        "SELECT target FROM social_follows WHERE owner=? ORDER BY target LIMIT 1000",
    )
    .bind(&owner)
    .fetch_all(&state.pool)
    .await?;
    let followers: Vec<String> = sqlx::query_scalar(
        "SELECT owner FROM social_follows WHERE target=? ORDER BY owner LIMIT 1000",
    )
    .bind(&owner)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"following":following,"followers":followers,"scope":"local_nas"}),
    ))
}
#[derive(Deserialize)]
struct Timeline {
    #[serde(default)]
    mine: bool,
    #[serde(default)]
    offset: u32,
}
async fn timeline(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Timeline>,
) -> Result<Json<Value>, ApiError> {
    let owner = owner(&state, headers).await?;
    let prefs = get_preferences(&state, &owner).await?;
    let rows=sqlx::query("WITH available AS (
 SELECT p.id,p.author,p.title,p.audience_scope,p.created_at,p.published,0 AS remote
 FROM social_posts p LEFT JOIN social_post_access a ON a.post_id=p.id WHERE
 ((? AND p.author=?) OR (NOT ? AND p.published=1 AND
 ((? AND a.level='public') OR (? AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=? AND f.target=p.author)) OR
 (? AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=p.author AND f.target=?)) OR
 (? AND (EXISTS(SELECT 1 FROM social_post_access_recipients r WHERE r.post_id=p.id AND r.viewer=?) OR EXISTS(SELECT 1 FROM social_inbox i WHERE i.post_id=p.id AND i.recipient=? AND json_extract(i.notice,'$.distribution')!='broadcast')))) AND
 NOT EXISTS(SELECT 1 FROM social_blocks b WHERE b.owner=? AND b.target=p.author)))
 UNION ALL
 SELECT i.post_id,i.author,COALESCE(NULLIF(json_extract(i.notice,'$.title'),''),'Encrypted post'),json_extract(i.notice,'$.distribution'),i.created_at,1,1
 FROM social_inbox i WHERE NOT ? AND i.recipient=? AND i.source_peer!=? AND
 NOT EXISTS(SELECT 1 FROM social_blocks b WHERE b.owner=i.recipient AND b.target=i.author) AND
 ((? AND json_extract(i.notice,'$.distribution')='broadcast') OR
 (? AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=i.recipient AND f.target=i.author)) OR
 (? AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=i.author AND f.target=i.recipient)) OR
 (? AND json_extract(i.notice,'$.distribution')!='broadcast'))
 ) SELECT * FROM available ORDER BY created_at DESC,id DESC LIMIT 50 OFFSET ?")
        .bind(query.mine).bind(&owner).bind(query.mine)
        .bind(prefs.broadcasts).bind(prefs.following).bind(&owner).bind(prefs.followers).bind(&owner).bind(prefs.direct).bind(&owner).bind(&owner).bind(&owner)
        .bind(query.mine).bind(&owner).bind(&state.config.peer_id).bind(prefs.broadcasts).bind(prefs.following).bind(prefs.followers).bind(prefs.direct)
        .bind(query.offset.min(10000)).fetch_all(&state.pool).await?;
    let mut posts = Vec::new();
    let mut unavailable_posts = 0;
    for row in &rows {
        let id: String = row.get("id");
        let remote: bool = row.get("remote");
        let content = if remote {
            delivery::remote_read(&state, &id, &owner).await
        } else {
            match load(&state, &id, &owner).await {
                Ok(full) => readable(&state, &full, &id).await,
                Err(error) => Err(error),
            }
        };
        let content = match content {
            Ok(content) => content,
            Err(ApiError(StatusCode::NOT_FOUND | StatusCode::FORBIDDEN, _)) => continue,
            Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
                | StatusCode::REQUEST_TIMEOUT,
                _,
            )) if remote => {
                unavailable_posts += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let preview = content.0["intent"]["action"]["post"].clone();
        let mut delivery = serde_json::Map::new();
        if query.mine {
            let states = sqlx::query(
                "SELECT state,COUNT(*) AS count FROM social_outbox WHERE post_id=? GROUP BY state",
            )
            .bind(&id)
            .fetch_all(&state.pool)
            .await?;
            for status in states {
                delivery.insert(
                    status.get::<String, _>("state"),
                    json!(status.get::<i64, _>("count")),
                );
            }
        }
        posts.push(json!({"id":id,"author":row.get::<String, _>("author"),"title":row.get::<String, _>("title"),"distribution":row.get::<String, _>("audience_scope"),"created_at":row.get::<i64, _>("created_at"),"published":row.get::<bool, _>("published"),"preview":preview,"delivery":delivery}));
    }
    Ok(Json(
        json!({"posts":posts,"unavailable_posts":unavailable_posts,"scope":"nas_inbox","network_delivery":if service(&state)?.runtime.network().is_some_and(|h|h.status.running.load(std::sync::atomic::Ordering::Relaxed)){"running"}else{"local_only"}}),
    ))
}

async fn load(
    state: &AppState,
    id: &str,
    viewer: &str,
) -> Result<sqlx::sqlite::SqliteRow, ApiError> {
    if id.len() != 68 || ContentId::from_slice(&bytes(id)?).is_err() {
        return Err(bad());
    }
    let row = sqlx::query("SELECT * FROM social_posts WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(forbidden)?;
    access::ensure(state, id, viewer).await?;
    Ok(row)
}
fn load_object(row: &sqlx::sqlite::SqliteRow) -> Result<StoredPost, ApiError> {
    let path: String = row.get("object_path");
    let metadata = std::fs::metadata(&path).map_err(|_| unavailable())?;
    if metadata.len() > 256 * 1024 {
        return Err(unavailable());
    }
    let stored: StoredPost =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
    if stored.version != 2 {
        return Err(unavailable());
    }
    Ok(stored)
}

fn decode_content(
    service: &crate::posts::Service,
    row: &sqlx::sqlite::SqliteRow,
    id: &str,
    stored: &StoredPost,
) -> Result<Zeroizing<Vec<u8>>, ApiError> {
    let author: String = row.get("author");
    let (secret, _) = service.keys.pair(&author).map_err(|_| unavailable())?;
    let aad = format!("red-leaves/post/1/{id}");
    let bytes = if let Some(transport) = &stored.transport {
        let envelope = &stored
            .transport_keys
            .iter()
            .find(|(owner, _)| owner == &author)
            .ok_or_else(unavailable)?
            .1;
        let key = encryption::unwrap(&secret, envelope, format!("{aad}/{author}").as_bytes())
            .map_err(|_| unavailable())?;
        encryption::open(&key, transport, aad.as_bytes()).map_err(|_| unavailable())?
    } else if let Some(public) = &stored.public_payload {
        Zeroizing::new(public.clone())
    } else {
        return Err(unavailable());
    };
    let intent = red_leaves_protocol::social::verify_post(
        &bytes,
        &self::bytes(&stored.signature)?,
        ContentId::from_slice(&self::bytes(id)?).map_err(|_| bad())?,
        &service.scope().map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    if wallet(intent.actor) != author {
        return Err(unavailable());
    }
    intent
        .scope
        .ensure_matches(&service.scope().map_err(|_| unavailable())?)
        .map_err(|_| unavailable())?;
    Ok(bytes)
}
#[derive(Deserialize, Default)]
struct ReadPost {
    author: Option<String>,
}
async fn read_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<ReadPost>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    if delivery::remote_row(&state, &id, &viewer).await?.is_some() {
        return delivery::remote_read(&state, &id, &viewer).await;
    }
    let local: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM social_posts WHERE id=?)")
        .bind(&id)
        .fetch_one(&state.pool)
        .await?;
    if !local && let Some(author) = query.author {
        return delivery::open_shared(&state, &id, &viewer, &wallet(address(&author)?)).await;
    }
    let row = load(&state, &id, &viewer).await?;
    readable(&state, &row, &id).await
}
async fn readable(
    state: &AppState,
    row: &sqlx::sqlite::SqliteRow,
    id: &str,
) -> Result<Json<Value>, ApiError> {
    let service = service(state)?;
    let stored = load_object(row)?;
    let payload = decode_content(service, row, id, &stored)?;
    let intent = red_leaves_protocol::social::verify_post(
        &payload,
        &bytes(&stored.signature)?,
        ContentId::from_slice(&bytes(id)?).map_err(|_| bad())?,
        &service.scope().map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    Ok(Json(
        json!({"id":id,"intent":intent,"signature":stored.signature}),
    ))
}

pub(super) mod delivery;
mod live;
mod media;

#[cfg(test)]
mod tests;

mod likes;

mod access;
