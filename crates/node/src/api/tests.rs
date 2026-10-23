use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use k256::ecdsa::SigningKey;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tower::ServiceExt;

#[tokio::test]
async fn request_logs_include_status_without_private_input() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }
    let (app, _) = setup().await;
    let capture = Capture(Arc::default());
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(capture.clone())
        .with_max_level(tracing::Level::TRACE)
        .finish();
    async {
        let (status, _) = call(
            &app,
            "POST",
            "/api/auth/challenge?private_query=SECRET_QUERY",
            json!({"address":"SECRET_BODY"}),
            Some("SECRET_TOKEN"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let request = Request::builder()
            .uri("/unknown/SECRET_PATH?key=SECRET_QUERY")
            .header("authorization", "Bearer SECRET_TOKEN")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    .with_subscriber(subscriber)
    .await;
    let text = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        !text.contains("SECRET"),
        "Private input reached the request log"
    );
    let events: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completed: Vec<_> = events
        .iter()
        .filter(|event| event["fields"]["message"] == "HTTP request completed")
        .collect();
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0]["fields"]["status"], 400);
    assert_eq!(completed[0]["span"]["route"], "/api/auth/challenge");
    assert_eq!(completed[1]["span"]["route"], "unmatched");
    assert!(completed[0]["span"]["request_id"].is_number());
    assert!(completed[0]["fields"]["headers_ms"].is_number());
}

async fn setup() -> (Router, SqlitePool) {
    let pool = crate::storage::open("sqlite::memory:").await.unwrap();
    let app = router(
        pool.clone(),
        Config {
            posts: None,
            listen: "127.0.0.1:0".into(),
            domain: "localhost:5173".into(),
            uri: "http://localhost:5173".into(),
            peer_id: libp2p::identity::Keypair::generate_ed25519()
                .public()
                .to_peer_id()
                .to_string(),
        },
    );
    (app, pool)
}
fn key(number: u8) -> SigningKey {
    SigningKey::from_slice(&[number; 32]).unwrap()
}
fn wallet(key: &SigningKey) -> String {
    let public = key.verifying_key().to_encoded_point(false);
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(&public.as_bytes()[1..])[12..])
    )
}
fn signed(challenge: &Value, key: &SigningKey) -> Value {
    let message: siwe::Message = challenge["message"].as_str().unwrap().parse().unwrap();
    let (signature, recovery) = key
        .sign_prehash_recoverable(&message.eip191_hash().unwrap())
        .unwrap();
    let mut bytes = signature.to_bytes().to_vec();
    bytes.push(recovery.to_byte() + 27);
    json!({ "challenge_id": challenge["challenge_id"], "verifier": challenge["verifier"], "signature": format!("0x{}", hex::encode(bytes)) })
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    // Native-client requests intentionally have no Origin or Cookie.
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(!response.headers().contains_key("set-cookie"));
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn challenge_for(app: &Router, key: &SigningKey) -> Value {
    let (status, body) = call(
        app,
        "POST",
        "/api/auth/challenge",
        json!({"address": wallet(key)}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
#[tokio::test]
async fn login_me_logout_and_replay() {
    let (app, pool) = setup().await;
    let key = key(1);
    let challenge = challenge_for(&app, &key).await;
    let proof = signed(&challenge, &key);
    let (status, session) = call(&app, "POST", "/api/auth/verify", proof.clone(), None).await;
    assert_eq!(status, StatusCode::OK);
    let token = session["token"].as_str().unwrap();
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM auth_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, identity::hash(token));
    assert_ne!(stored, token);
    let (status, user) = call(&app, "GET", "/api/auth/me", Value::Null, Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["address"], wallet(&key));
    assert_eq!(
        call(&app, "POST", "/api/auth/verify", proof, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "POST", "/api/auth/logout", Value::Null, Some(token))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/api/auth/me", Value::Null, Some(token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn rejects_invalid_binding_signature_expiry_and_scope() {
    let (app, pool) = setup().await;
    let key = key(2);
    let challenge = challenge_for(&app, &key).await;
    let good = signed(&challenge, &key);
    let mut wrong = good.clone();
    wrong["verifier"] = json!("a".repeat(64));
    assert_eq!(
        call(&app, "POST", "/api/auth/verify", wrong, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/auth/verify",
            signed(&challenge, &self::key(3)),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        identity::verify(
            challenge["message"].as_str().unwrap(),
            good["signature"].as_str().unwrap(),
            "evil.example",
            "http://localhost:5173"
        )
        .await
        .is_err()
    );
    assert!(
        identity::verify(
            challenge["message"].as_str().unwrap(),
            good["signature"].as_str().unwrap(),
            "localhost:5173",
            "https://evil.example"
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE auth_challenges SET expires_at=0")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/auth/verify", good, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    cleanup(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM auth_challenges")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn concurrent_verification_creates_only_one_session() {
    let (app, pool) = setup().await;
    let key = key(4);
    let challenge = challenge_for(&app, &key).await;
    let proof = signed(&challenge, &key);
    let (a, b) = tokio::join!(
        call(&app, "POST", "/api/auth/verify", proof.clone(), None),
        call(&app, "POST", "/api/auth/verify", proof, None)
    );
    assert!(matches!(
        (a.0, b.0),
        (StatusCode::OK, StatusCode::UNAUTHORIZED) | (StatusCode::UNAUTHORIZED, StatusCode::OK)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM auth_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn session_expiry_and_cookie_alone_are_rejected() {
    let (app, pool) = setup().await;
    let key = key(5);
    let challenge = challenge_for(&app, &key).await;
    let (_, session) = call(
        &app,
        "POST",
        "/api/auth/verify",
        signed(&challenge, &key),
        None,
    )
    .await;
    let token = session["token"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("cookie", format!("red_leaves_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    sqlx::query("UPDATE auth_sessions SET expires_at=0")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/api/auth/me", Value::Null, Some(token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn bounds_untrusted_input_and_pending_challenges() {
    let (app, _) = setup().await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/auth/challenge",
            json!({"address":"invalid"}),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/auth/challenge",
            json!({"address":"a".repeat(5000)}),
            None
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let key = key(6);
    for _ in 0..10 {
        challenge_for(&app, &key).await;
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/auth/challenge",
            json!({"address":wallet(&key)}),
            None
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn failed_session_insert_rolls_back_challenge_consumption() {
    let (app, pool) = setup().await;
    let key = key(7);
    let challenge = challenge_for(&app, &key).await;
    let proof = signed(&challenge, &key);
    sqlx::raw_sql("CREATE TRIGGER fail_session BEFORE INSERT ON auth_sessions BEGIN SELECT RAISE(ABORT, 'test failure'); END;")
        .execute(&pool).await.unwrap();
    assert_eq!(
        call(&app, "POST", "/api/auth/verify", proof.clone(), None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    sqlx::query("DROP TRIGGER fail_session")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/auth/verify", proof, None).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn rejects_wrong_chain_and_expired_signed_message() {
    let (app, _) = setup().await;
    let key = key(8);
    let challenge = challenge_for(&app, &key).await;
    let mut message: siwe::Message = challenge["message"].as_str().unwrap().parse().unwrap();
    message.chain_id = 1;
    let mut modified = challenge.clone();
    modified["message"] = json!(message.to_string());
    let proof = signed(&modified, &key);
    assert!(
        identity::verify(
            modified["message"].as_str().unwrap(),
            proof["signature"].as_str().unwrap(),
            "localhost:5173",
            "http://localhost:5173"
        )
        .await
        .is_err()
    );
    message.chain_id = CHAIN_ID;
    message.expiration_time = Some("2020-01-01T00:00:00Z".parse().unwrap());
    modified["message"] = json!(message.to_string());
    let proof = signed(&modified, &key);
    assert!(
        identity::verify(
            modified["message"].as_str().unwrap(),
            proof["signature"].as_str().unwrap(),
            "localhost:5173",
            "http://localhost:5173"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn sqlite_session_survives_reopen() {
    let path = std::env::temp_dir().join(format!(
        "red-leaves-auth-{}.sqlite",
        identity::secret().unwrap()
    ));
    let url = format!("sqlite://{}", path.display());
    let config = Config {
        posts: None,
        listen: "127.0.0.1:0".into(),
        domain: "localhost:5173".into(),
        uri: "http://localhost:5173".into(),
        peer_id: libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
            .to_string(),
    };
    let pool = crate::storage::open(&url).await.unwrap();
    let app = router(pool.clone(), config.clone());
    let key = key(9);
    let challenge = challenge_for(&app, &key).await;
    let (_, session) = call(
        &app,
        "POST",
        "/api/auth/verify",
        signed(&challenge, &key),
        None,
    )
    .await;
    drop(app);
    pool.close().await;
    let reopened = crate::storage::open(&url).await.unwrap();
    let app = router(reopened.clone(), config);
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/auth/me",
            Value::Null,
            session["token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(app);
    reopened.close().await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn authentication_rate_limit_is_bounded() {
    let (app, _) = setup().await;
    for _ in 0..240 {
        assert_eq!(
            call(&app, "GET", "/api/auth/me", Value::Null, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(&app, "GET", "/api/auth/me", Value::Null, None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn cross_origin_preflight_and_authentication_responses() {
    let (app, _) = setup().await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/auth/verify")
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "content-type,authorization",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let headers = response.headers();
    assert_eq!(
        headers["access-control-allow-origin"],
        "http://localhost:5173"
    );
    assert!(
        headers["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .contains("POST")
    );
    let allowed = headers["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .to_lowercase();
    assert!(allowed.contains("content-type") && allowed.contains("authorization"));
    assert_eq!(headers["access-control-allow-credentials"], "true");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("origin", "https://separate-web.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );
    assert_eq!(
        response.headers()["access-control-allow-credentials"],
        "true"
    );
}

#[tokio::test]
async fn nas_identity_requires_session_and_exposes_only_public_fields() {
    let (app, _) = setup().await;
    assert_eq!(
        call(&app, "GET", "/api/nas/info", Value::Null, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let key = key(10);
    let challenge = challenge_for(&app, &key).await;
    let (_, session) = call(
        &app,
        "POST",
        "/api/auth/verify",
        signed(&challenge, &key),
        None,
    )
    .await;
    let (status, info) = call(
        &app,
        "GET",
        "/api/nas/info",
        Value::Null,
        session["token"].as_str(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        info["peer_id"]
            .as_str()
            .unwrap()
            .parse::<libp2p::PeerId>()
            .is_ok()
    );
    assert_eq!(info["p2p_running"], false);
    assert_eq!(info["node_type"], "nas");
    assert_eq!(info.as_object().unwrap().len(), 5);
}

#[tokio::test]
async fn browser_cookie_login_access_csrf_and_logout() {
    let (app, pool) = setup().await;
    let key = key(19);
    let challenge = challenge_for(&app, &key).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/verify")
                .header("content-type", "application/json")
                .header("origin", "http://localhost:5173")
                .header("x-red-leaves-client", "web")
                .body(Body::from(signed(&challenge, &key).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-credentials"],
        "true"
    );
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("; HttpOnly; SameSite=Lax; Max-Age=3600"));
    assert!(cookie.contains("; Path=/api"));
    assert!(!cookie.contains("Domain="));
    assert!(!cookie.contains("; Secure"));
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert!(body.get("token").is_none());
    assert_eq!(body["chain_id"], CHAIN_ID);
    let cookie = cookie.split(';').next().unwrap();
    for (origin, client, path, expected) in [
        (
            Some("http://localhost:5173"),
            true,
            "/api/auth/me",
            StatusCode::OK,
        ),
        (None, true, "/api/nas/info", StatusCode::OK),
        (
            Some("https://attacker.example"),
            true,
            "/api/auth/me",
            StatusCode::FORBIDDEN,
        ),
        (
            Some("http://localhost:5173"),
            false,
            "/api/auth/me",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut request = Request::builder().uri(path).header("cookie", cookie);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if client {
            request = request.header("x-red-leaves-client", "web");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    // An untrusted site cannot revoke a valid session.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", cookie)
                .header("origin", "https://attacker.example")
                .header("x-red-leaves-client", "web")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM auth_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/logout")
                    .header("cookie", cookie)
                    .header("x-red-leaves-client", "web")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("cookie", cookie)
                .header("x-red-leaves-client", "web")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cookie_auth_rejects_expired_ambiguous_and_malformed_credentials() {
    let (app, pool) = setup().await;
    let key = key(20);
    let challenge = challenge_for(&app, &key).await;
    let (_, session) = call(
        &app,
        "POST",
        "/api/auth/verify",
        signed(&challenge, &key),
        None,
    )
    .await;
    let token = session["token"].as_str().unwrap();
    let cookie = format!("red_leaves_session={token}");
    for (cookie, authorization) in [
        (format!("{cookie}; {cookie}"), None),
        ("red_leaves_session=bad".to_owned(), None),
        (cookie.clone(), Some("Basic invalid")),
    ] {
        let mut request = Request::builder()
            .uri("/api/auth/me")
            .header("cookie", cookie)
            .header("x-red-leaves-client", "web");
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    sqlx::query("UPDATE auth_sessions SET expires_at=0")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("cookie", cookie)
                .header("x-red-leaves-client", "web")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn https_session_cookies_are_secure_and_expire_with_matching_attributes() {
    let config = Config {
        posts: None,
        listen: "127.0.0.1:3000".into(),
        domain: "web.example".into(),
        uri: "https://web.example".into(),
        peer_id: "test".into(),
    };
    for (value, max_age) in [("ab".repeat(32), SESSION_SECONDS), (String::new(), 0)] {
        let cookie = session_cookie(&config, &value, max_age);
        let cookie = cookie.to_str().unwrap();
        assert!(cookie.ends_with("; Secure"));
        assert!(cookie.contains("HttpOnly; SameSite=Lax"));
        assert!(cookie.contains(&format!("Max-Age={max_age}")));
    }
}
