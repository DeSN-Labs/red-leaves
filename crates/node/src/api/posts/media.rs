//! Files remain private uploads until a signed post references them. Media reads use post permissions.
use super::*;
use axum::{
    body::Bytes,
    extract::Request,
    middleware::{self, Next},
};
use red_leaves_protocol::social::Media;
use std::{fs, io::Write};

const MAX_FILE: usize = 64 * 1024 * 1024;
static TRANSFERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/media", get(pending).post(upload))
        .route("/api/media/{id}", get(preview))
        .route("/api/media/{id}/discard", post(discard))
        .route("/api/posts/{post_id}/media/{id}", get(read))
        .layer(DefaultBodyLimit::max(MAX_FILE))
        .layer(middleware::from_fn(transfer_limit))
}
async fn transfer_limit(request: Request, next: Next) -> Response {
    let Ok(_permit) = TRANSFERS.try_acquire() else {
        return ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many media transfers. Try again shortly.",
        )
        .into_response();
    };
    next.run(request).await
}
fn metadata(row: &sqlx::sqlite::SqliteRow) -> Media {
    Media {
        id: row.get("id"),
        mime: row.get("mime"),
        size: row.get::<i64, _>("size") as u64,
        digest: row.get("digest"),
    }
}
pub(super) async fn validate_uploads(
    pool: &SqlitePool,
    owner: &str,
    files: &[Media],
) -> Result<(), ApiError> {
    for file in files {
        let row = sqlx::query("SELECT * FROM social_media WHERE id=? AND owner=?")
            .bind(&file.id)
            .bind(owner)
            .fetch_optional(pool)
            .await?
            .ok_or_else(forbidden)?;
        if metadata(&row) != *file {
            return Err(bad());
        }
    }
    Ok(())
}
fn matches_type(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "video/mp4" => bytes.get(4..8) == Some(b"ftyp"),
        "video/webm" => bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        _ => false,
    }
}
async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Media>, ApiError> {
    let owner = owner(&state, headers.clone()).await?;
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(bad)?;
    let file = Media {
        id: identity::secret().map_err(internal)?,
        mime: mime.into(),
        size: body.len() as u64,
        digest: hex::encode(ContentId::of(&body).as_bytes()),
    };
    file.validate().map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "Use JPEG, PNG, WebP or GIF images up to 10 MiB, or MP4/WebM video up to 64 MiB.",
        )
    })?;
    if !matches_type(mime, &body) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "File content does not match its media type",
        ));
    }
    // Reserve quota transactionally; concurrent uploads cannot bypass the per-owner bound.
    let mut tx = state.pool.begin().await?;
    let reserved: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size),0) FROM social_media WHERE owner=?")
            .bind(&owner)
            .fetch_one(&mut *tx)
            .await?;
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_media m WHERE owner=? AND NOT EXISTS(SELECT 1 FROM social_post_media p WHERE p.media_id=m.id)")
        .bind(&owner).fetch_one(&mut *tx).await?;
    if reserved + body.len() as i64 > 512 * 1024 * 1024 || pending >= 32 {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Media quota reached (512 MiB per wallet or 32 unpublished files). Remove unused uploads.",
        ));
    }
    let service = service(&state)?;
    let (_, public) = service.keys.pair(&owner).map_err(|_| unavailable())?;
    let key = encryption::random_key().map_err(|_| unavailable())?;
    let aad = format!("red-leaves/media/1/{owner}/{}", file.id);
    let sealed = encryption::seal(&key, &body, aad.as_bytes()).map_err(|_| unavailable())?;
    let envelope = encryption::wrap(&public, &key, aad.as_bytes()).map_err(|_| unavailable())?;
    fs::create_dir_all(&service.objects).map_err(|_| unavailable())?;
    let path = service.objects.join(format!("media-{}.bin", file.id));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(&path).map_err(|_| unavailable())?;
    output
        .write_all(&sealed.nonce)
        .and_then(|_| output.write_all(&sealed.ciphertext))
        .and_then(|_| output.sync_all())
        .map_err(|_| unavailable())?;
    #[cfg(unix)]
    fs::File::open(&service.objects)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| unavailable())?;
    sqlx::query("INSERT INTO social_media(id,owner,mime,size,digest,object_path,envelope,created_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&file.id).bind(&owner).bind(&file.mime).bind(file.size as i64).bind(&file.digest).bind(path.to_str().ok_or_else(unavailable)?)
        .bind(serde_json::to_string(&envelope).map_err(|_| unavailable())?).bind(now()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(file))
}
async fn media_row(state: &AppState, id: &str) -> Result<sqlx::sqlite::SqliteRow, ApiError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    sqlx::query("SELECT * FROM social_media WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(forbidden)
}
async fn pending(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let rows = sqlx::query("SELECT * FROM social_media m WHERE owner=? AND NOT EXISTS(SELECT 1 FROM social_post_media p WHERE p.media_id=m.id) ORDER BY created_at DESC,id DESC LIMIT 32")
        .bind(viewer).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"media":rows.iter().map(metadata).collect::<Vec<_>>()}),
    ))
}
fn response(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> Result<Response, ApiError> {
    let file = metadata(row);
    file.validate().map_err(|_| unavailable())?;
    let owner: String = row.get("owner");
    let path: String = row.get("object_path");
    if fs::metadata(&path).map_err(|_| unavailable())?.len() != file.size + 28 {
        return Err(unavailable());
    }
    let bytes = fs::read(path).map_err(|_| unavailable())?;
    if bytes.len() as u64 != file.size + 28 {
        return Err(unavailable());
    }
    let sealed = Sealed {
        nonce: bytes[..12].to_vec(),
        ciphertext: bytes[12..].to_vec(),
    };
    let envelope: KeyEnvelope =
        serde_json::from_str(&row.get::<String, _>("envelope")).map_err(|_| unavailable())?;
    let (secret, _) = service(state)?
        .keys
        .pair(&owner)
        .map_err(|_| unavailable())?;
    let aad = format!("red-leaves/media/1/{owner}/{}", file.id);
    let key = encryption::unwrap(&secret, &envelope, aad.as_bytes()).map_err(|_| unavailable())?;
    let plaintext = encryption::open(&key, &sealed, aad.as_bytes()).map_err(|_| unavailable())?;
    if hex::encode(ContentId::of(&plaintext).as_bytes()) != file.digest {
        return Err(unavailable());
    }
    Ok((
        [
            (header::CONTENT_TYPE, file.mime.as_str()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_DISPOSITION, "inline"),
        ],
        plaintext.to_vec(),
    )
        .into_response())
}
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let viewer = owner(&state, headers).await?;
    let row = media_row(&state, &id).await?;
    if row.get::<String, _>("owner") != viewer {
        return Err(forbidden());
    }
    response(&state, &row)
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((post_id, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let viewer = owner(&state, headers).await?;
    if delivery::remote_row(&state, &post_id, &viewer)
        .await?
        .is_some()
    {
        return delivery::remote_media(&state, &post_id, &viewer, &id).await;
    }
    let post = load(&state, &post_id, &viewer).await?;
    let author: String = post.get("author");
    let stored = load_object(&post)?;
    let payload = decode_content(service(&state)?, &post, &post_id, &stored)?;
    let intent = red_leaves_protocol::social::verify_post(
        &payload,
        &bytes(&stored.signature)?,
        ContentId::from_slice(&bytes(&post_id)?).map_err(|_| bad())?,
        &service(&state)?.scope().map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    let Action::Publish { post } = intent.action else {
        return Err(unavailable());
    };
    let row = media_row(&state, &id).await?;
    if row.get::<String, _>("owner") != author || !post.media.contains(&metadata(&row)) {
        return Err(forbidden());
    }
    response(&state, &row)
}
async fn discard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    let row = media_row(&state, &id).await?;
    if row.get::<String, _>("owner") != viewer {
        return Err(forbidden());
    }
    // Referenced files remain part of a signed publication.
    let deleted = sqlx::query("DELETE FROM social_media WHERE id=? AND owner=? AND NOT EXISTS(SELECT 1 FROM social_post_media WHERE media_id=?)")
        .bind(&id).bind(&viewer).bind(&id).execute(&state.pool).await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Published media cannot be removed",
        ));
    }
    fs::remove_file(row.get::<String, _>("object_path")).map_err(|_| unavailable())?;
    Ok(Json(json!({"status":"removed"})))
}
