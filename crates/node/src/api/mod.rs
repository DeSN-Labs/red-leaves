//! NAS authentication and content API shared by Web and native clients.
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
mod posts;

pub fn start_delivery(
    pool: SqlitePool,
    config: Config,
    network: Option<(
        crate::network::transport::Handle,
        tokio::sync::mpsc::Receiver<crate::network::transport::Incoming>,
    )>,
) -> tokio::task::JoinHandle<()> {
    posts::delivery::start(app_state(pool, config), network)
}

#[derive(Clone)]
pub struct Config {
    pub posts: Option<Arc<crate::posts::Service>>,
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
            posts: Some(crate::posts::Service::from_env()?),
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
        tracing::error!("NAS database operation failed");
        Self(StatusCode::SERVICE_UNAVAILABLE, "NAS storage unavailable")
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
    let origin: axum::http::HeaderValue = config
        .uri
        .trim_end_matches('/')
        .parse()
        .expect("Authentication URI must be a valid header value");
    let state = app_state(pool, config);
    Router::new()
        .route("/api/auth/challenge", post(challenge))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/me", get(me))
        .route("/api/nas/info", get(nas_info))
        .route("/api/auth/logout", post(logout))
        .merge(posts::routes())
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origin)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                    axum::http::HeaderName::from_static("x-red-leaves-client"),
                ]),
        )
        .with_state(state)
}
fn app_state(pool: SqlitePool, config: Config) -> AppState {
    AppState {
        pool,
        config,
        rate: Arc::default(),
        permits: Arc::new(tokio::sync::Semaphore::new(16)),
    }
}
async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    use tracing::Instrument;
    static REQUEST_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let request_id = REQUEST_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|path| path.as_str())
        .unwrap_or("unmatched")
        .to_owned();
    let span = tracing::info_span!("http_request", request_id, method, route);
    async move {
        let start = Instant::now();
        tracing::debug!("HTTP request received");
        let mut response = guard_response(state, request, next).await;
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        response.headers_mut().insert(
            header::PRAGMA,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        let status = response.status().as_u16();
        let headers_ms = start.elapsed().as_millis() as u64;
        if response.status().is_server_error() {
            tracing::error!(status, headers_ms, "HTTP request completed");
        } else {
            tracing::info!(status, headers_ms, "HTTP request completed");
        }
        response
    }
    .instrument(span)
    .await
}
async fn guard_response(state: AppState, request: Request, next: Next) -> Response {
    let headers = request.headers();
    // A custom header forces cross-origin preflight. Same-origin GET requests may omit Origin.
    let cookie_request = web_client(headers)
        || (!headers.contains_key(header::AUTHORIZATION) && headers.contains_key(header::COOKIE));
    if cookie_request
        && (!web_client(headers)
            || headers.get(header::ORIGIN).is_some_and(|origin| {
                origin.to_str().ok() != Some(state.config.uri.trim_end_matches('/'))
            }))
    {
        return ApiError(StatusCode::FORBIDDEN, "Untrusted browser request").into_response();
    }
    let content_request = ["/api/posts", "/api/social/", "/api/content/", "/api/media"]
        .iter()
        .any(|prefix| request.uri().path().starts_with(prefix));
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
    if !allowed || permit.is_err() {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Authentication rate limit exceeded",
        )
        .into_response()
    } else {
        match tokio::time::timeout(
            Duration::from_secs(if content_request { 60 } else { 10 }),
            next.run(request),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => ApiError(
                StatusCode::REQUEST_TIMEOUT,
                "Authentication request timed out",
            )
            .into_response(),
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    address: String,
    chain_id: u64,
    expires_at: i64,
}
async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<VerifyRequest>,
) -> Result<Response, ApiError> {
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
    sqlx::query("INSERT OR IGNORE INTO social_accounts(owner) VALUES(?)")
        .bind(&address)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let browser = web_client(&headers);
    let cookie = browser.then(|| session_cookie(&state.config, &token, SESSION_SECONDS));
    let mut response = Json(SessionResponse {
        token: if browser { None } else { Some(token) },
        address,
        chain_id: CHAIN_ID,
        expires_at,
    })
    .into_response();
    if let Some(cookie) = cookie {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    Ok(response)
}
fn web_client(headers: &HeaderMap) -> bool {
    headers
        .get("x-red-leaves-client")
        .is_some_and(|value| value == "web")
}
fn session_cookie(config: &Config, token: &str, max_age: i64) -> axum::http::HeaderValue {
    let secure = if config.uri.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "red_leaves_session={token}; Path=/api; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    )
    .parse()
    .expect("Session cookie contains only validated values")
}
fn session_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let token = if headers.contains_key(header::AUTHORIZATION) {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or_else(invalid)?
    } else {
        if !web_client(headers) {
            return Err(invalid());
        }
        let mut cookies = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .flat_map(|h| h.split(';'))
            .filter_map(|entry| entry.trim().strip_prefix("red_leaves_session="));
        let token = cookies.next().ok_or_else(invalid)?;
        if cookies.next().is_some() {
            return Err(invalid());
        }
        token
    };
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
        .bind(identity::hash(session_token(&headers)?)).bind(now()).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
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
    p2p_peers: usize,
    p2p_addresses: Vec<String>,
}
async fn nas_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<NasInfo>, ApiError> {
    let _ = me(State(state.clone()), headers).await?;
    let net = state
        .config
        .posts
        .as_ref()
        .and_then(|s| s.runtime.network());
    Ok(Json(NasInfo {
        p2p_peers: net.as_ref().map_or(0, |h| {
            h.status
                .connected
                .load(std::sync::atomic::Ordering::Relaxed)
        }),
        p2p_addresses: net
            .as_ref()
            .and_then(|h| h.status.listeners.lock().ok().map(|v| v.clone()))
            .unwrap_or_default(),
        node_type: "nas",
        peer_id: state.config.peer_id,
        p2p_running: state
            .config
            .posts
            .as_ref()
            .and_then(|s| s.runtime.network())
            .is_some_and(|h| h.status.running.load(std::sync::atomic::Ordering::Relaxed)),
    }))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    match session_token(&headers) {
        Ok(token) => {
            sqlx::query("DELETE FROM auth_sessions WHERE token_hash=?")
                .bind(identity::hash(token))
                .execute(&state.pool)
                .await?;
        }
        Err(_) if web_client(&headers) => {}
        Err(error) => return Err(error),
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if web_client(&headers) {
        response
            .headers_mut()
            .insert(header::SET_COOKIE, session_cookie(&state.config, "", 0));
    }
    Ok(response)
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
    sqlx::query("DELETE FROM social_challenges WHERE expires_at <= ?")
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[cfg(test)]
mod tests;
