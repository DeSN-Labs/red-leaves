use k256::ecdsa::SigningKey;
use red_leaves_protocol::{
    Address, Error, NetworkScope, Nonce, encryption,
    social::{Action, Distribution, Intent, PostDraft},
};
use sha3::{Digest, Keccak256};
fn action() -> (Intent, SigningKey) {
    let key = SigningKey::from_bytes((&[4; 32]).into()).unwrap();
    let public = key.verifying_key().to_encoded_point(false);
    let owner = Address::from_slice(&Keccak256::digest(&public.as_bytes()[1..])[12..]).unwrap();
    (
        Intent {
            scope: NetworkScope::new(84532, Address([7; 20])).unwrap(),
            actor: owner,
            nas_peer_id: libp2p_identity::Keypair::generate_ed25519()
                .public()
                .to_peer_id()
                .to_string(),
            nonce: Nonce([1; 32]),
            issued_at: 1000,
            expires_at: 1100,
            action: Action::Publish {
                post: PostDraft {
                    title: "Title".into(),
                    media: vec![],
                    body: "Secret text".into(),
                    price_wei: "0".into(),
                    distribution: Distribution::Broadcast,
                    recipients: vec![],
                },
            },
        },
        key,
    )
}
fn signature(action: &Intent, key: &SigningKey) -> Vec<u8> {
    let (signature, recovery) = key
        .sign_prehash_recoverable(&action.signing_hash().unwrap())
        .unwrap();
    [signature.to_bytes().as_slice(), &[recovery.to_byte() + 27]].concat()
}
#[test]
fn publication_signature_binds_action_scope_audience_price_and_expiry() {
    let (intent, key) = action();
    let signature = signature(&intent, &key);
    let encoded = intent.encode().unwrap();
    assert_eq!(
        Intent::verify(
            &encoded,
            &signature,
            &intent.scope,
            &intent.nas_peer_id,
            1001
        )
        .unwrap(),
        intent
    );
    assert_eq!(
        Intent::verify(
            &encoded,
            &signature,
            &intent.scope,
            &intent.nas_peer_id,
            1100
        ),
        Err(Error::Expired)
    );
    assert!(Intent::verify(&encoded, &signature, &intent.scope, "another-nas", 1001).is_err());
    let mut changed = intent.clone();
    let Action::Publish { ref mut post } = changed.action else {
        unreachable!()
    };
    post.price_wei = "1".into();
    assert!(
        Intent::verify(
            &changed.encode().unwrap(),
            &signature,
            &intent.scope,
            &intent.nas_peer_id,
            1001
        )
        .is_err()
    );
}
#[test]
fn recipients_and_prices_have_unambiguous_bounds() {
    let (intent, _) = action();
    let Action::Publish { mut post } = intent.action else {
        unreachable!()
    };
    for price in ["01", "-1", "0.1", ""] {
        post.price_wei = price.into();
        assert!(post.validate(intent.actor).is_err());
    }
    post.price_wei = "1".into();
    post.distribution = Distribution::Directed;
    assert!(post.validate(intent.actor).is_err());
    post.recipients = vec![Address([1; 20]), Address([1; 20])];
    assert!(post.validate(intent.actor).is_err());
}
#[test]
fn hpke_roundtrip_rejects_wrong_recipient_and_context() {
    let (secret, public) = encryption::key_pair().unwrap();
    let (other, _) = encryption::key_pair().unwrap();
    let key = encryption::random_key().unwrap();
    let envelope = encryption::wrap(&public, &key, b"post/recipient").unwrap();
    assert_eq!(
        *encryption::unwrap(&secret, &envelope, b"post/recipient").unwrap(),
        *key
    );
    assert!(encryption::unwrap(&other, &envelope, b"post/recipient").is_err());
    assert!(encryption::unwrap(&secret, &envelope, b"other-post/recipient").is_err());
}
#[test]
fn symmetric_encryption_authenticates_ciphertext_and_context() {
    let key = encryption::random_key().unwrap();
    let mut sealed = encryption::seal(&key, b"secret", b"post/id").unwrap();
    assert_eq!(
        &**encryption::open(&key, &sealed, b"post/id").unwrap(),
        b"secret"
    );
    assert!(encryption::open(&key, &sealed, b"other/id").is_err());
    sealed.ciphertext[0] ^= 1;
    assert!(encryption::open(&key, &sealed, b"post/id").is_err());
}
#[test]
fn paid_layer_remains_locked_after_transport_decryption() {
    let paid_key = encryption::random_key().unwrap();
    let transport = encryption::random_key().unwrap();
    let paid = encryption::seal(&paid_key, b"Paid plaintext", b"paid/post").unwrap();
    let outer = encryption::seal(&transport, &paid.ciphertext, b"transport/post").unwrap();
    let delivered = encryption::open(&transport, &outer, b"transport/post").unwrap();
    assert_ne!(&**delivered, b"Paid plaintext");
    assert_eq!(&**delivered, &paid.ciphertext);
    assert!(encryption::open(&transport, &paid, b"paid/post").is_err());
    assert_eq!(
        &**encryption::open(&paid_key, &paid, b"paid/post").unwrap(),
        b"Paid plaintext"
    );
}

#[test]
fn media_formats_are_signed_and_legacy_text_encoding_remains_compatible() {
    use red_leaves_protocol::social::Media;
    let (mut intent, key) = action();
    let legacy = intent.encode().unwrap();
    let json = serde_json::to_value(&intent).unwrap();
    assert!(json["action"]["post"].get("media").is_none());
    assert_eq!(
        red_leaves_protocol::canonical::decode::<Intent>(&legacy, 32768).unwrap(),
        intent
    );
    let signature = signature(&intent, &key);
    let Action::Publish { ref mut post } = intent.action else {
        unreachable!()
    };
    let image = Media {
        id: "ab".repeat(32),
        mime: "image/png".into(),
        size: 100,
        digest: format!("1220{}", "cd".repeat(32)),
    };
    post.media = vec![image.clone()];
    post.body.clear();
    assert!(post.validate(intent.actor).is_ok()); // title + images
    post.body = "Optional caption".into();
    assert!(post.validate(intent.actor).is_ok());
    post.media[0].mime = "video/mp4".into();
    assert!(post.validate(intent.actor).is_ok()); // title + text + video
    post.body.clear();
    assert!(post.validate(intent.actor).is_ok()); // title + video
    post.media.push(image);
    assert!(post.validate(intent.actor).is_err()); // mixed media is not a supported format
    post.media.pop();
    assert!(intent.verify_signature(&signature).is_err());
    let Action::Publish { ref mut post } = intent.action else {
        unreachable!()
    };
    post.media.clear();
    assert!(post.validate(intent.actor).is_err()); // title alone
}

#[test]
fn delivery_notices_reject_private_titles_wrong_scope_and_unbounded_fields() {
    use red_leaves_protocol::delivery::Notice;
    let (intent, _) = action();
    let mut notice = Notice {
        scope: intent.scope.clone(),
        id: intent.id().unwrap(),
        author: intent.actor,
        source_peer: intent.nas_peer_id.clone(),
        distribution: Distribution::Broadcast,
        price_wei: "0".into(),
        market: None,
        created_at: 1000,
        title: "Public title".into(),
    };
    assert!(notice.validate(&intent.scope, 1001).is_ok());
    notice.distribution = Distribution::Directed;
    assert!(notice.validate(&intent.scope, 1001).is_err());
    notice.title.clear();
    assert!(notice.validate(&intent.scope, 1001).is_ok());
    notice.price_wei = "100".into();
    assert!(notice.validate(&intent.scope, 1001).is_err());
    notice.market = Some(Address([8; 20]));
    assert!(notice.validate(&intent.scope, 1001).is_ok());
    notice.title = "Encrypted title must not leak".into();
    assert!(notice.validate(&intent.scope, 1001).is_err());
    notice.title.clear();
    notice.created_at = 1200;
    assert!(notice.validate(&intent.scope, 1001).is_err());
}
