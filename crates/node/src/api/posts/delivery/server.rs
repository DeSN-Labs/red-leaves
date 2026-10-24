use super::*;
use std::{
    collections::HashMap,
    io::{Read, Seek},
    sync::Mutex,
};
async fn serve_content(
    state: &AppState,
    remote: PeerId,
    id: &ContentId,
    viewer: Address,
) -> Result<NetResponse, ApiError> {
    let public = authorize_peer(state, viewer, remote).await?;
    let viewer = wallet(viewer);
    let id = hex::encode(id.as_bytes());
    let row = load(state, &id, &viewer).await?;
    let author: String = row.get("author");
    if profile(state, &author).await?.peer != state.config.peer_id {
        return Err(forbidden());
    }
    let stored = load_object(&row)?;
    let payload = decode_content(service(state)?, &row, &id, &stored)?;
    let key = encryption::random_key().map_err(|_| unavailable())?;
    let aad = aad(&id, &viewer);
    let sealed = encryption::seal(&key, &payload, aad.as_bytes()).map_err(|_| unavailable())?;
    let envelope = encryption::wrap(&public, &key, aad.as_bytes()).map_err(|_| unavailable())?;
    Ok(NetResponse::Content {
        sealed,
        envelope,
        signature: stored.signature,
    })
}
#[derive(Clone)]
struct MediaGrant {
    post_id: String,
    viewer: String,
    revision: i64,
    peer: PeerId,
    path: String,
    size: u64,
    expires: i64,
}
type PeerRates = HashMap<PeerId, (i64, usize, usize)>;
#[derive(Clone)]
pub(in crate::api::posts) struct Server {
    state: AppState,
    grants: Arc<Mutex<HashMap<String, MediaGrant>>>,
    rate: Arc<Mutex<PeerRates>>,
}
impl Server {
    pub(in crate::api::posts) fn new(state: AppState) -> Self {
        Self {
            state,
            grants: Arc::new(Mutex::new(HashMap::new())),
            rate: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn media_start(
        &self,
        remote: PeerId,
        id: ContentId,
        viewer: Address,
        media_id: String,
    ) -> Result<NetResponse, ApiError> {
        let state = &self.state;
        let public = authorize_peer(state, viewer, remote).await?;
        let viewer = wallet(viewer);
        let id = hex::encode(id.as_bytes());
        let row = load(state, &id, &viewer).await?;
        let author: String = row.get("author");
        if profile(state, &author).await?.peer != state.config.peer_id {
            return Err(forbidden());
        }
        let stored = load_object(&row)?;
        let payload = decode_content(service(state)?, &row, &id, &stored)?;
        let intent = red_leaves_protocol::social::verify_post(
            &payload,
            &bytes(&stored.signature)?,
            cid(&id)?,
            &service(state)?.scope().map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())?;
        let Action::Publish { post } = intent.action else {
            return Err(bad());
        };
        let file = post
            .media
            .iter()
            .find(|m| m.id == media_id)
            .ok_or_else(forbidden)?;
        let media = sqlx::query("SELECT * FROM social_media WHERE id=? AND owner=?")
            .bind(&media_id)
            .bind(&author)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(forbidden)?;
        let (secret, _) = service(state)?
            .keys
            .pair(&author)
            .map_err(|_| unavailable())?;
        let wrapped: KeyEnvelope =
            serde_json::from_str(&media.get::<String, _>("envelope")).map_err(|_| unavailable())?;
        let aad = format!("red-leaves/media/1/{author}/{media_id}");
        let key =
            encryption::unwrap(&secret, &wrapped, aad.as_bytes()).map_err(|_| unavailable())?;
        let envelope =
            encryption::wrap(&public, &key, aad.as_bytes()).map_err(|_| unavailable())?;
        let path: String = media.get("object_path");
        let mut input = std::fs::File::open(&path).map_err(|_| unavailable())?;
        if input.metadata().map_err(|_| unavailable())?.len() != file.size + 28 {
            return Err(unavailable());
        }
        let mut nonce = vec![0; 12];
        input.read_exact(&mut nonce).map_err(|_| unavailable())?;
        let token = identity::secret().map_err(internal)?;
        let revision = super::super::access::revision(state, &id).await?;
        let mut grants = self.grants.lock().map_err(|_| unavailable())?;
        grants.retain(|_, g| g.expires > now());
        if grants.len() >= 64 || grants.values().filter(|g| g.peer == remote).count() >= 2 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many active media downloads",
            ));
        }
        grants.insert(
            token.clone(),
            MediaGrant {
                post_id: id,
                viewer,
                revision,
                peer: remote,
                path,
                size: file.size + 16,
                expires: now() + 120,
            },
        );
        Ok(NetResponse::Media {
            token,
            envelope,
            nonce,
            size: file.size + 16,
        })
    }
    async fn media_chunk(
        &self,
        remote: PeerId,
        token: &str,
        offset: u64,
    ) -> Result<NetResponse, ApiError> {
        let grant = self
            .grants
            .lock()
            .map_err(|_| unavailable())?
            .get(token)
            .cloned()
            .ok_or_else(forbidden)?;
        super::super::access::ensure(&self.state, &grant.post_id, &grant.viewer).await?;
        if super::super::access::revision(&self.state, &grant.post_id).await? != grant.revision {
            return Err(forbidden());
        }
        if grant.peer != remote
            || grant.expires <= now()
            || offset >= grant.size
            || !offset.is_multiple_of(CHUNK as u64)
        {
            return Err(forbidden());
        }
        let mut file = std::fs::File::open(&grant.path).map_err(|_| unavailable())?;
        file.seek(std::io::SeekFrom::Start(12 + offset))
            .map_err(|_| unavailable())?;
        let mut bytes = vec![0; (grant.size - offset).min(CHUNK as u64) as usize];
        file.read_exact(&mut bytes).map_err(|_| unavailable())?;
        if offset + bytes.len() as u64 == grant.size {
            self.grants.lock().map_err(|_| unavailable())?.remove(token);
        }
        Ok(NetResponse::Chunk { bytes })
    }
    pub(in crate::api::posts) async fn request(
        &self,
        remote: PeerId,
        request: NetRequest,
    ) -> Result<NetResponse, ApiError> {
        {
            let mut rate = self.rate.lock().map_err(|_| unavailable())?;
            rate.retain(|_, (time, _, _)| now() - *time < 60);
            if !rate.contains_key(&remote) && rate.len() >= 128 {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "P2P peer rate limit reached",
                ));
            }
            let entry = rate.entry(remote).or_insert((now(), 0, 0));
            let counter = if matches!(request, NetRequest::MediaChunk { .. }) {
                &mut entry.2
            } else {
                &mut entry.1
            };
            if *counter
                >= if matches!(request, NetRequest::MediaChunk { .. }) {
                    1024
                } else {
                    120
                }
            {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "P2P request rate limit reached",
                ));
            }
            *counter += 1;
        }
        let state = &self.state;
        match request {
            NetRequest::Offer { notice, recipient } => {
                receive(state, remote, &notice, &wallet(recipient)).await
            }
            NetRequest::Fetch { id, viewer } => serve_content(state, remote, &id, viewer).await,
            NetRequest::Follow { payload, signature } => {
                receive_follow(state, remote, &payload, &signature).await
            }
            NetRequest::Like { payload, signature } => {
                likes::receive(state, remote, &payload, &signature).await
            }
            NetRequest::Likes { id, viewer } => likes::serve(state, remote, id, viewer).await,
            NetRequest::Access { id, viewer } => {
                authorize_peer(state, viewer, remote).await?;
                let id = hex::encode(id.as_bytes());
                let row = load(state, &id, &wallet(viewer)).await?;
                if profile(state, &row.get::<String, _>("author")).await?.peer
                    != state.config.peer_id
                {
                    return Err(forbidden());
                }
                Ok(NetResponse::Access {
                    revision: super::super::access::revision(state, &id).await? as u64,
                })
            }
            NetRequest::MediaStart {
                id,
                viewer,
                media_id,
            } => self.media_start(remote, id, viewer, media_id).await,
            NetRequest::MediaChunk { token, offset } => {
                self.media_chunk(remote, &token, offset).await
            }
            NetRequest::Sync { offset } => {
                if offset > 224 {
                    return Err(bad());
                }
                let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM social_posts WHERE published=1 AND id IN (SELECT post_id FROM social_post_access WHERE level='public') ORDER BY created_at DESC,id DESC LIMIT 32 OFFSET ?").bind(offset).fetch_all(&state.pool).await?;
                let mut notices = vec![];
                for id in ids {
                    notices.push(notice(state, &id).await?);
                }
                Ok(NetResponse::Notices { notices })
            }
        }
    }
}

async fn receive_follow(
    state: &AppState,
    remote: PeerId,
    payload: &[u8],
    signature: &[u8],
) -> Result<NetResponse, ApiError> {
    if payload.len() > 32 * 1024 {
        return Err(bad());
    }
    let intent: Intent = canonical::decode(payload, 32 * 1024).map_err(|_| bad())?;
    intent
        .verify_signature(signature)
        .map_err(|_| forbidden())?;
    intent
        .scope
        .ensure_matches(&service(state)?.scope().map_err(|_| unavailable())?)
        .map_err(|_| forbidden())?;
    let Action::Follow { target, enabled } = intent.action else {
        return Err(bad());
    };
    if intent.issued_at > now() as u64 + 120 || intent.nas_peer_id != remote.to_string() {
        return Err(bad());
    }
    authorize_peer(state, intent.actor, remote).await?;
    if profile(state, &wallet(target)).await?.peer != state.config.peer_id {
        return Err(forbidden());
    }
    let owner = wallet(intent.actor);
    let target = wallet(target);
    let nonce = hex::encode(intent.nonce.0);
    let mut tx = state.pool.begin().await?;
    let previous = sqlx::query(
        "SELECT issued_at,nonce FROM social_follow_versions WHERE owner=? AND target=?",
    )
    .bind(&owner)
    .bind(&target)
    .fetch_optional(&mut *tx)
    .await?;
    if previous.is_some_and(|r| {
        (r.get::<i64, _>("issued_at"), r.get::<String, _>("nonce"))
            >= (intent.issued_at as i64, nonce.clone())
    }) {
        return Ok(NetResponse::Ignored);
    }
    sqlx::query("INSERT INTO social_follow_versions(owner,target,issued_at,nonce) VALUES(?,?,?,?) ON CONFLICT(owner,target) DO UPDATE SET issued_at=excluded.issued_at,nonce=excluded.nonce")
        .bind(&owner).bind(&target).bind(intent.issued_at as i64).bind(&nonce).execute(&mut *tx).await?;
    if enabled {
        sqlx::query("INSERT OR IGNORE INTO social_follows(owner,target) VALUES(?,?)")
            .bind(&owner)
            .bind(&target)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM social_follows WHERE owner=? AND target=?")
            .bind(&owner)
            .bind(&target)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    service(state)?.runtime.changed(&target);
    Ok(NetResponse::Saved { body: false })
}
