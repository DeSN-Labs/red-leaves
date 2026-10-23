use super::*;
use crate::posts::{Service, keys::KeyStore, registry::*};
use alloy_sol_types::SolCall;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use k256::ecdsa::SigningKey;
use std::collections::HashMap;
use tower::ServiceExt;

#[derive(Default)]
struct RpcState {
    profiles: HashMap<Address, (String, Vec<u8>)>,
}
async fn rpc(State(state): State<Arc<RpcState>>, Json(input): Json<Value>) -> Json<Value> {
    let result = match input["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x14a34"),
        "eth_blockNumber" => json!("0x100"),
        "eth_call" => {
            let data = bytes(input["params"][0]["data"].as_str().unwrap()).unwrap();
            let result = if data.starts_with(&getUserCall::SELECTOR) {
                let call = getUserCall::abi_decode(&data).unwrap();
                let address = Address(call.owner.into_array());
                let (peer, key) = state.profiles.get(&address).unwrap();
                getUserCall::abi_encode_returns(&RegistryUser {
                    nasPeerId: peer.clone(),
                    encryptionPublicKey: key.clone().into(),
                    nickname: "User".into(),
                })
            } else {
                panic!("Unexpected contract call");
            };
            json!(format!("0x{}", hex::encode(result)))
        }
        _ => panic!("Unexpected RPC method"),
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}
struct Setup {
    app: Router,
    pool: SqlitePool,
    service: Arc<Service>,
    rpc: Arc<RpcState>,
    task: tokio::task::JoinHandle<()>,
    root: std::path::PathBuf,
    actors: Vec<(SigningKey, String, String)>,
}
impl Drop for Setup {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn setup() -> Setup {
    let root =
        std::env::temp_dir().join(format!("red-leaves-posts-{}", identity::secret().unwrap()));
    let keys = KeyStore::new(root.join("keys"), root.join("master.key"));
    let peer = libp2p::identity::Keypair::generate_ed25519()
        .public()
        .to_peer_id()
        .to_string();
    let pool = crate::storage::open("sqlite::memory:").await.unwrap();
    let mut profiles = HashMap::new();
    let mut actors = vec![];
    for value in 1..=3u8 {
        let key = SigningKey::from_slice(&[value; 32]).unwrap();
        let point = key.verifying_key().to_encoded_point(false);
        let address = Address::from_slice(&keccak(&point.as_bytes()[1..])[12..]).unwrap();
        let owner = wallet(address);
        let token = hex::encode([value; 32]);
        sqlx::query("INSERT INTO auth_sessions(token_hash,address,chain_id,created_at,expires_at) VALUES(?,?,84532,?,?)").bind(identity::hash(&token)).bind(&owner).bind(now()).bind(now()+3600).execute(&pool).await.unwrap();
        profiles.insert(address, (peer.clone(), keys.pair(&owner).unwrap().1));
        actors.push((key, owner, token));
    }
    let rpc_state = Arc::new(RpcState { profiles });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let rpc_app = Router::new()
        .route("/", post(rpc))
        .with_state(rpc_state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, rpc_app).await.unwrap();
    });
    let service = Arc::new(Service {
        runtime: Default::default(),
        keys,
        chain: Some(Chain::new(rpc_url, Address([7; 20])).unwrap()),
        objects: root.join("objects"),
    });
    let app = super::super::router(
        pool.clone(),
        Config {
            listen: "127.0.0.1:0".into(),
            domain: "localhost:5173".into(),
            uri: "http://localhost:5173".into(),
            peer_id: peer,
            posts: Some(service.clone()),
        },
    );
    Setup {
        app,
        pool,
        service,
        rpc: rpc_state,
        task,
        root,
        actors,
    }
}
async fn call(
    setup: &Setup,
    actor: usize,
    method: &str,
    path: &str,
    input: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .uri(path)
        .method(method)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", setup.actors[actor].2))
        .body(Body::from(input.to_string()))
        .unwrap();
    let response = setup.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 256 * 1024).await.unwrap())
        .unwrap_or(Value::Null);
    (status, value)
}
async fn prepare_signed(setup: &Setup, actor: usize, action: Action) -> Value {
    let (status, prepared) = call(
        setup,
        actor,
        "POST",
        "/api/social/prepare",
        json!({"action":action}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{prepared}");
    let payload = bytes(prepared["payload"].as_str().unwrap()).unwrap();
    let intent: Intent = canonical::decode(&payload, 32 * 1024).unwrap();
    let (signature, recovery) = setup.actors[actor]
        .0
        .sign_prehash_recoverable(&intent.signing_hash().unwrap())
        .unwrap();
    json!({"payload":prepared["payload"],"signature":format!("0x{}{}",hex::encode(signature.to_bytes()),hex::encode([recovery.to_byte()+27]))})
}
fn draft(setup: &Setup, directed: bool) -> Action {
    Action::Publish {
        post: red_leaves_protocol::social::PostDraft {
            title: "Test post".into(),
            media: vec![],
            body: "Private content that must not leak".repeat(150),
            distribution: if directed {
                Distribution::Directed
            } else {
                Distribution::Broadcast
            },
            recipients: if directed {
                vec![address(&setup.actors[1].1).unwrap()]
            } else {
                vec![]
            },
        },
    }
}
async fn publish(setup: &Setup, directed: bool) -> Value {
    let proof = prepare_signed(setup, 0, draft(setup, directed)).await;
    let (status, value) = call(setup, 0, "POST", "/api/social/commit", proof).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn prefs(
    setup: &Setup,
    actor: usize,
    broadcasts: bool,
    following: bool,
    followers: bool,
    direct: bool,
) {
    let (status,_)=call(setup,actor,"POST","/api/social/preferences",json!({"broadcasts":broadcasts,"following":following,"followers":followers,"direct":direct})).await;
    assert_eq!(status, StatusCode::OK);
}
async fn feed(setup: &Setup, actor: usize) -> Value {
    let (status, value) = call(setup, actor, "GET", "/api/posts", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    value
}

#[tokio::test]
async fn signed_confirmation_is_required_and_commits_are_idempotent() {
    let setup = setup().await;
    let proof = prepare_signed(&setup, 0, draft(&setup, false)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_posts")
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_ne!(
        call(&setup, 1, "POST", "/api/social/commit", proof.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&setup, 0, "POST", "/api/social/commit", proof.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&setup, 0, "POST", "/api/social/commit", proof).await.0,
        StatusCode::OK
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM social_outbox")
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn directed_content_is_encrypted_and_unrelated_users_cannot_read() {
    let setup = setup().await;
    let result = publish(&setup, true).await;
    let id = result["id"].as_str().unwrap();
    let path: String = sqlx::query_scalar("SELECT object_path FROM social_posts WHERE id=?")
        .bind(id)
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    let stored = std::fs::read_to_string(path).unwrap();
    assert!(!stored.contains("Private content"));
    assert_eq!(
        call(&setup, 2, "GET", &format!("/api/posts/{id}"), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = call(&setup, 1, "GET", &format!("/api/posts/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["intent"]["action"]["post"]["body"]
            .as_str()
            .unwrap()
            .starts_with("Private content")
    );
    assert_eq!(feed(&setup, 1).await["posts"].as_array().unwrap().len(), 1);
    prefs(&setup, 1, true, false, false, false).await;
    assert!(
        feed(&setup, 1).await["posts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn reception_categories_use_real_follow_edges_without_expanding_audience() {
    let setup = setup().await;
    publish(&setup, false).await;
    assert!(
        feed(&setup, 1).await["posts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let proof = prepare_signed(
        &setup,
        1,
        Action::Follow {
            target: address(&setup.actors[0].1).unwrap(),
            enabled: true,
        },
    )
    .await;
    assert_eq!(
        call(&setup, 1, "POST", "/api/social/commit", proof).await.0,
        StatusCode::OK
    );
    assert_eq!(feed(&setup, 1).await["posts"].as_array().unwrap().len(), 1);
    prefs(&setup, 0, false, false, true, false).await;
    // Viewer 0 receives broadcasts by their follower 1, not arbitrary authors.
    let proof = prepare_signed(&setup, 1, draft(&setup, false)).await;
    assert_eq!(
        call(&setup, 1, "POST", "/api/social/commit", proof).await.0,
        StatusCode::OK
    );
    assert_eq!(feed(&setup, 0).await["posts"].as_array().unwrap().len(), 1);
    prefs(&setup, 2, true, true, true, true).await;
    publish(&setup, true).await;
    assert_eq!(feed(&setup, 2).await["posts"].as_array().unwrap().len(), 2);
}
#[tokio::test]
async fn publication_has_no_pricing_and_obsolete_routes_are_removed() {
    let setup = setup().await;
    for price in ["0", "100"] {
        let mut action = serde_json::to_value(draft(&setup, false)).unwrap();
        action["post"]["price_wei"] = json!(price);
        assert_eq!(
            call(
                &setup,
                0,
                "POST",
                "/api/social/prepare",
                json!({"action":action})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let result = publish(&setup, true).await;
    assert_eq!(result["status"], "saved_pending_network");
    assert!(
        result.get("price_wei").is_none()
            && result.get("market").is_none()
            && result.get("encrypted_key").is_none()
    );
    let id = result["id"].as_str().unwrap();
    for (method, route) in [
        ("GET", "listing"),
        ("POST", "activate"),
        ("POST", "unlock"),
        ("POST", "purchase-info"),
    ] {
        assert_eq!(
            call(
                &setup,
                0,
                method,
                &format!("/api/posts/{id}/{route}"),
                json!({})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, read) = call(&setup, 1, "GET", &format!("/api/posts/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(read.get("locked").is_none() && read.get("paid_key_envelope").is_none());
    assert!(read["intent"]["action"]["post"].get("price_wei").is_none());
    let posts = feed(&setup, 1).await;
    assert!(posts["posts"][0].get("price_wei").is_none());
    assert!(read["signature"].is_string());
}
#[tokio::test]
async fn key_files_survive_restart_and_corruption_never_replaces_keys() {
    let setup = setup().await;
    let owner = &setup.actors[0].1;
    let original = setup.service.keys.pair(owner).unwrap().1;
    let reopened = KeyStore::new(setup.root.join("keys"), setup.root.join("master.key"));
    assert_eq!(reopened.pair(owner).unwrap().1, original);
    let file = setup
        .root
        .join("keys")
        .join(format!("{}.key", owner.trim_start_matches("0x")));
    std::fs::write(&file, b"corrupted").unwrap();
    assert!(reopened.pair(owner).is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"corrupted");
}

#[tokio::test]
async fn lost_master_key_does_not_silently_create_an_unusable_replacement() {
    let setup = setup().await;
    let path = setup.root.join("master.key");
    std::fs::remove_file(&path).unwrap();
    assert!(setup.service.keys.pair(&setup.actors[0].1).is_err());
    assert!(!path.exists());
}

#[tokio::test]
async fn broadcast_is_public_and_expired_uncommitted_intents_cannot_publish() {
    let setup = setup().await;
    let proof = prepare_signed(&setup, 0, draft(&setup, false)).await;
    sqlx::query("UPDATE social_challenges SET expires_at=0")
        .execute(&setup.pool)
        .await
        .unwrap();
    assert_eq!(
        call(&setup, 0, "POST", "/api/social/commit", proof).await.0,
        StatusCode::BAD_REQUEST
    );
    let post = publish(&setup, false).await;
    let row = sqlx::query("SELECT * FROM social_posts WHERE id=?")
        .bind(post["id"].as_str().unwrap())
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    let object = load_object(&row).unwrap();
    assert!(object.public_payload.is_some() && object.transport.is_none());
    let plaintext =
        decode_content(&setup.service, &row, post["id"].as_str().unwrap(), &object).unwrap();
    let original: Intent = canonical::decode(&plaintext, 32 * 1024).unwrap();
    assert!(matches!(original.action, Action::Publish { .. }));
}

async fn media_call(
    setup: &Setup,
    actor: usize,
    method: &str,
    path: &str,
    mime: &str,
    body: Vec<u8>,
) -> (StatusCode, Vec<u8>) {
    let response = setup
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .method(method)
                .header("content-type", mime)
                .header("authorization", format!("Bearer {}", setup.actors[actor].2))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}
#[tokio::test]
async fn media_uploads_are_private_signed_encrypted_and_audience_gated() {
    let setup = setup().await;
    // Exceed the JSON route limit to verify that binary uploads use their own bound.
    let image = [b"\x89PNG\r\n\x1a\n".as_slice(), &vec![42; 200_000]].concat();
    let (status, uploaded) =
        media_call(&setup, 0, "POST", "/api/media", "image/png", image.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&uploaded)
    );
    let file: red_leaves_protocol::social::Media = serde_json::from_slice(&uploaded).unwrap();
    assert_eq!(
        media_call(
            &setup,
            1,
            "GET",
            &format!("/api/media/{}", file.id),
            "image/png",
            vec![]
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        media_call(
            &setup,
            0,
            "GET",
            &format!("/api/media/{}", file.id),
            "image/png",
            vec![]
        )
        .await
        .1,
        image
    );
    assert_eq!(
        media_call(&setup, 0, "POST", "/api/media", "video/mp4", image.clone())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let path: String = sqlx::query_scalar("SELECT object_path FROM social_media WHERE id=?")
        .bind(&file.id)
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), image);
    let mut action = draft(&setup, true);
    let Action::Publish { ref mut post } = action else {
        unreachable!()
    };
    post.body.clear();
    post.media = vec![file.clone()];
    let mut forged = action.clone();
    let Action::Publish { ref mut post } = forged else {
        unreachable!()
    };
    post.media[0].size += 1;
    assert_eq!(
        call(
            &setup,
            0,
            "POST",
            "/api/social/prepare",
            json!({"action":forged})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let proof = prepare_signed(&setup, 0, action).await;
    let (status, result) = call(&setup, 0, "POST", "/api/social/commit", proof).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let id = result["id"].as_str().unwrap();
    let path = format!("/api/posts/{id}/media/{}", file.id);
    assert_eq!(
        call(
            &setup,
            0,
            "POST",
            &format!("/api/media/{}/discard", file.id),
            json!({})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(feed(&setup, 1).await["posts"][0]["preview"].is_object());
    assert_eq!(
        media_call(&setup, 1, "GET", &path, "image/png", vec![])
            .await
            .1,
        image
    );
    assert_eq!(
        media_call(&setup, 2, "GET", &path, "image/png", vec![])
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // Audience permission cannot substitute a different upload for the signed reference.
    assert_eq!(
        media_call(
            &setup,
            1,
            "GET",
            &format!("/api/posts/{id}/media/{}", "ee".repeat(32)),
            "image/png",
            vec![]
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut corrupted = std::fs::read(&path_from_media(&setup, &file.id).await).unwrap();
    corrupted[20] ^= 1;
    std::fs::write(path_from_media(&setup, &file.id).await, corrupted).unwrap();
    assert_eq!(
        media_call(&setup, 1, "GET", &path, "image/png", vec![])
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
async fn path_from_media(setup: &Setup, id: &str) -> String {
    sqlx::query_scalar("SELECT object_path FROM social_media WHERE id=?")
        .bind(id)
        .fetch_one(&setup.pool)
        .await
        .unwrap()
}
#[tokio::test]
async fn group_scopes_validate_relationships_and_freeze_the_signed_audience() {
    let setup = setup().await;
    let a = &setup.actors[0].1;
    let b = &setup.actors[1].1;
    let c = &setup.actors[2].1;
    for (owner, target) in [(a, b), (b, a), (c, a)] {
        sqlx::query("INSERT INTO social_follows(owner,target) VALUES(?,?)")
            .bind(owner)
            .bind(target)
            .execute(&setup.pool)
            .await
            .unwrap();
    }
    for (scope, targets) in [
        (Distribution::Following, vec![b]),
        (Distribution::Followers, vec![b, c]),
        (Distribution::Friends, vec![b]),
    ] {
        let mut action = draft(&setup, true);
        let Action::Publish { ref mut post } = action else {
            unreachable!()
        };
        post.distribution = scope.clone();
        post.recipients = targets
            .iter()
            .map(|value| address(value).unwrap())
            .collect();
        let proof = prepare_signed(&setup, 0, action).await;
        let (status, result) = call(&setup, 0, "POST", "/api/social/commit", proof).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let id = result["id"].as_str().unwrap();
        assert_eq!(
            call(&setup, 2, "GET", &format!("/api/posts/{id}"), Value::Null)
                .await
                .0,
            if scope == Distribution::Followers {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
    }
    let mut wrong = draft(&setup, true);
    let Action::Publish { ref mut post } = wrong else {
        unreachable!()
    };
    post.distribution = Distribution::Friends;
    post.recipients.push(address(c).unwrap());
    assert_eq!(
        call(
            &setup,
            0,
            "POST",
            "/api/social/prepare",
            json!({"action":wrong})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut action = draft(&setup, true);
    let Action::Publish { ref mut post } = action else {
        unreachable!()
    };
    post.distribution = Distribution::Following;
    let proof = prepare_signed(&setup, 0, action).await;
    sqlx::query("INSERT INTO social_follows(owner,target) VALUES(?,?)")
        .bind(a)
        .bind(c)
        .execute(&setup.pool)
        .await
        .unwrap();
    let (status, result) = call(&setup, 0, "POST", "/api/social/commit", proof).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(
            &setup,
            2,
            "GET",
            &format!("/api/posts/{}", result["id"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, mine) = call(&setup, 0, "GET", "/api/posts?mine=true", Value::Null).await;
    assert!(
        mine["posts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["distribution"] == "friends")
    );
}
#[tokio::test]
async fn local_delivery_filters_preferences_and_blocks_and_exposes_owner_progress() {
    let setup = setup().await;
    let config = Config {
        posts: Some(setup.service.clone()),
        listen: "127.0.0.1:0".into(),
        domain: "localhost:5173".into(),
        uri: "http://localhost:5173".into(),
        peer_id: setup.rpc.profiles.values().next().unwrap().0.clone(),
    };
    let task = super::super::start_delivery(setup.pool.clone(), config, None);
    let result = publish(&setup, true).await;
    let id = result["id"].as_str().unwrap();
    for _ in 0..40 {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM social_inbox WHERE post_id=? AND recipient=?")
                .bind(id)
                .bind(&setup.actors[1].1)
                .fetch_one(&setup.pool)
                .await
                .unwrap();
        if count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, progress) = call(
        &setup,
        0,
        "GET",
        &format!("/api/posts/{id}/delivery"),
        Value::Null,
    )
    .await;
    assert_eq!(progress["targets"][0]["state"], "delivered");
    assert_eq!(
        call(
            &setup,
            1,
            "GET",
            &format!("/api/posts/{id}/delivery"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, _) = call(
        &setup,
        1,
        "POST",
        "/api/social/blocks",
        json!({"target":setup.actors[0].1,"blocked":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let blocked = publish(&setup, true).await;
    let blocked_id = blocked["id"].as_str().unwrap();
    for _ in 0..40 {
        let (_, value) = call(
            &setup,
            0,
            "GET",
            &format!("/api/posts/{blocked_id}/delivery"),
            Value::Null,
        )
        .await;
        if value["targets"][0]["state"] == "filtered" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        feed(&setup, 1).await["posts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM social_inbox WHERE post_id=? AND recipient=?")
            .bind(blocked_id)
            .bind(&setup.actors[1].1)
            .fetch_one(&setup.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    task.abort();
}
#[tokio::test]
async fn live_events_are_bearer_scoped_bounded_and_release_connections() {
    let setup = setup().await;
    let response = setup
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/social/events")
                .header("authorization", format!("Bearer {}", setup.actors[1].2))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let mut stream = response.into_body().into_data_stream();
    let first = stream.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event: sync"));
    setup.service.runtime.changed(&setup.actors[2].1);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), stream.next())
            .await
            .is_err()
    );
    setup.service.runtime.changed(&setup.actors[1].1);
    let update = stream.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&update).contains("event: update"));
    drop(stream);
    assert!(setup.service.runtime.watchers.lock().unwrap().is_empty());
    assert_eq!(
        call(
            &setup,
            0,
            "POST",
            "/api/social/blocks",
            json!({"target":setup.actors[0].1,"blocked":true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn network_requests_cannot_impersonate_a_wallets_registered_nas() {
    let setup = setup().await;
    let peer = setup.rpc.profiles.values().next().unwrap().0.clone();
    let config = Config {
        posts: Some(setup.service.clone()),
        listen: "127.0.0.1:0".into(),
        domain: "localhost:5173".into(),
        uri: "http://localhost:5173".into(),
        peer_id: peer.clone(),
    };
    let state = super::super::app_state(setup.pool.clone(), config);
    let viewer = address(&setup.actors[1].1).unwrap();
    assert!(
        delivery::authorize_peer(&state, viewer, peer.parse().unwrap())
            .await
            .is_ok()
    );
    assert_eq!(
        delivery::authorize_peer(&state, viewer, libp2p::PeerId::random())
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn post_objects_reject_obsolete_versions_and_removed_fields() {
    let setup = setup().await;
    let result = publish(&setup, false).await;
    let id = result["id"].as_str().unwrap();
    let path: String = sqlx::query_scalar("SELECT object_path FROM social_posts WHERE id=?")
        .bind(id)
        .fetch_one(&setup.pool)
        .await
        .unwrap();
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut obsolete = original.clone();
    obsolete["version"] = json!(1);
    let mut removed = original.clone();
    removed["paid_key"] = Value::Null;
    for object in [obsolete, removed] {
        std::fs::write(&path, serde_json::to_vec(&object).unwrap()).unwrap();
        assert_eq!(
            call(&setup, 1, "GET", &format!("/api/posts/{id}"), Value::Null)
                .await
                .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(
        call(&setup, 1, "GET", &format!("/api/posts/{id}"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}
