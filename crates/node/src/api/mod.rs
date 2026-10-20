//! NAS authentication API shared by Web and native clients.
use crate::identity::{self, CHAIN_ID, CHALLENGE_SECONDS, SESSION_SECONDS};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Config {
    pub listen: String,
    pub domain: String,
    pub uri: String,
    pub peer_id: String,
}
impl Config {
    pub fn from_env() -> Result<Self, identity::Error> {
        let uri =
            std::env::var("RED_LEAVES_AUTH_URI").unwrap_or_else(|_| "http://127.0.0.1:5173".into());
        let parsed: axum::http::Uri = uri.parse()?;
        let host = parsed.host().ok_or("Authentication URI requires a host")?;
        if parsed.scheme_str() != Some("https")
            && !(parsed.scheme_str() == Some("http")
                && matches!(host, "127.0.0.1" | "localhost" | "[::1]"))
        {
            return Err("Authentication URI requires HTTPS outside loopback development".into());
        }
        if parsed.path() != "/" || parsed.query().is_some() {
            return Err("Authentication URI must be an origin without a path or query".into());
        }
        let domain = parsed
            .authority()
            .ok_or("Authentication URI requires an authority")?
            .as_str()
            .to_owned();
        if domain.contains('@') {
            return Err("Authentication URI must not contain credentials".into());
        }
        Ok(Self {
            listen: std::env::var("RED_LEAVES_LISTEN").unwrap_or_else(|_| "127.0.0.1:3000".into()),
            domain,
            uri,
            peer_id: crate::network::load_or_create_peer_id(std::path::Path::new(
                &std::env::var("RED_LEAVES_NODE_KEY_PATH")
                    .unwrap_or_else(|_| "runtime/nas/node.key".into()),
            ))?,
        })
    }
}
#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    config: Config,
    rate: Arc<Mutex<VecDeque<Instant>>>,
    permits: Arc<tokio::sync::Semaphore>,
}
#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}
#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}
impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        eprintln!("Authentication database operation failed");
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication storage unavailable",
        )
    }
}
fn invalid() -> ApiError {
    ApiError(
        StatusCode::UNAUTHORIZED,
        "Invalid or expired authentication",
    )
}
fn internal(_: identity::Error) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Authentication operation failed",
    )
}
fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
fn valid_secret(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn router(pool: SqlitePool, config: Config) -> Router {
    let state = AppState {
        pool,
        config,
        rate: Arc::default(),
        permits: Arc::new(tokio::sync::Semaphore::new(16)),
    };
    Router::new()
        .route("/api/auth/challenge", post(challenge))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/me", get(me))
        .route("/api/nas/info", get(nas_info))
        .route("/api/auth/logout", post(logout))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
        )
        .with_state(state)
}
async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let allowed = if let Ok(mut rate) = state.rate.lock() {
        let current = Instant::now();
        while rate
            .front()
            .is_some_and(|entry| current.duration_since(*entry) >= Duration::from_secs(60))
        {
            rate.pop_front();
        }
        if rate.len() >= 240 {
            false
        } else {
            rate.push_back(current);
            true
        }
    } else {
        false
    };
    let permit = state.permits.try_acquire();
    let mut response = if !allowed || permit.is_err() {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Authentication rate limit exceeded",
        )
        .into_response()
    } else {
        match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await {
            Ok(response) => response,
            Err(_) => ApiError(
                StatusCode::REQUEST_TIMEOUT,
                "Authentication request timed out",
            )
            .into_response(),
        }
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::PRAGMA,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeRequest {
    address: String,
}
#[derive(Serialize)]
struct ChallengeResponse {
    challenge_id: String,
    verifier: String,
    message: String,
    expires_at: i64,
}
async fn challenge(
    State(state): State<AppState>,
    Json(input): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    let address = identity::address(&input.address)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Invalid wallet address"))?;
    let address = format!("0x{}", hex::encode(address));
    let id = identity::secret().map_err(internal)?;
    let verifier = identity::secret().map_err(internal)?;
    let message = identity::message(
        identity::address(&address).map_err(internal)?,
        &state.config.domain,
        &state.config.uri,
        id.clone(),
    )
    .map_err(internal)?
    .to_string();
    let expires_at = now() + CHALLENGE_SECONDS;
    let result = sqlx::query("INSERT INTO auth_challenges (id,address,message,verifier_hash,expires_at) SELECT ?,?,?,?,? WHERE (SELECT COUNT(*) FROM auth_challenges WHERE expires_at > ?) < 1000 AND (SELECT COUNT(*) FROM auth_challenges WHERE address = ? AND expires_at > ?) < 10")
        .bind(&id).bind(&address).bind(&message).bind(identity::hash(&verifier)).bind(expires_at)
        .bind(now()).bind(&address).bind(now()).execute(&state.pool).await?;
    if result.rows_affected() != 1 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pending challenges",
        ));
    }
    Ok(Json(ChallengeResponse {
        challenge_id: id,
        verifier,
        message,
        expires_at,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyRequest {
    challenge_id: String,
    verifier: String,
    signature: String,
}
#[derive(Serialize)]
struct SessionResponse {
    token: String,
    address: String,
    chain_id: u64,
    expires_at: i64,
}
async fn verify(
    State(state): State<AppState>,
    Json(input): Json<VerifyRequest>,
) -> Result<Json<SessionResponse>, ApiError> {
    if !valid_secret(&input.challenge_id) || !valid_secret(&input.verifier) {
        return Err(invalid());
    }
    let binding = identity::hash(&input.verifier);
    let row = sqlx::query("SELECT address,message FROM auth_challenges WHERE id=? AND verifier_hash=? AND expires_at > ?")
        .bind(&input.challenge_id).bind(&binding).bind(now()).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    let message: String = row.try_get("message")?;
    let address: String = row.try_get("address")?;
    identity::verify(
        &message,
        &input.signature,
        &state.config.domain,
        &state.config.uri,
    )
    .await
    .map_err(|_| invalid())?;
    let token = identity::secret().map_err(internal)?;
    let expires_at = now() + SESSION_SECONDS;
    let mut tx = state.pool.begin().await?;
    let consumed = sqlx::query(
        "DELETE FROM auth_challenges WHERE id=? AND verifier_hash=? AND expires_at > ?",
    )
    .bind(&input.challenge_id)
    .bind(binding)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(invalid());
    }
    let inserted = sqlx::query("INSERT INTO auth_sessions (token_hash,address,chain_id,created_at,expires_at) SELECT ?,?,?,?,? WHERE (SELECT COUNT(*) FROM auth_sessions WHERE expires_at > ?) < 10000")
        .bind(identity::hash(&token)).bind(&address).bind(CHAIN_ID as i64).bind(now()).bind(expires_at).bind(now()).execute(&mut *tx).await?;
    if inserted.rows_affected() != 1 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many active sessions",
        ));
    }
    tx.commit().await?;
    Ok(Json(SessionResponse {
        token,
        address,
        chain_id: CHAIN_ID,
        expires_at,
    }))
}
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(invalid)?;
    if !valid_secret(token) {
        return Err(invalid());
    }
    Ok(token)
}
#[derive(Serialize)]
struct UserResponse {
    address: String,
    chain_id: i64,
    expires_at: i64,
}
async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserResponse>, ApiError> {
    let row = sqlx::query("SELECT address,chain_id,expires_at FROM auth_sessions WHERE token_hash=? AND expires_at > ?")
        .bind(identity::hash(bearer(&headers)?)).bind(now()).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    Ok(Json(UserResponse {
        address: row.try_get("address")?,
        chain_id: row.try_get("chain_id")?,
        expires_at: row.try_get("expires_at")?,
    }))
}
#[derive(Serialize)]
struct NasInfo {
    node_type: &'static str,
    peer_id: String,
    p2p_running: bool,
}
async fn nas_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<NasInfo>, ApiError> {
    let _ = me(State(state.clone()), headers).await?;
    Ok(Json(NasInfo {
        node_type: "nas",
        peer_id: state.config.peer_id,
        p2p_running: false,
    }))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM auth_sessions WHERE token_hash=?")
        .bind(identity::hash(bearer(&headers)?))
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn cleanup(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM auth_challenges WHERE expires_at <= ?")
        .bind(now())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= ?")
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[cfg(test)]
mod tests;
