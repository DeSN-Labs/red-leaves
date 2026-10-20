use libp2p_core::SignedEnvelope;
use libp2p_identity::Keypair;
use multiaddr::Multiaddr;
use prost::Message;
use red_leaves_protocol::{
    Address, ContentId, Error, MAX_CHUNK, MAX_CONTROL_FRAME, MAX_OBJECT, MAX_ROUTE, NetworkScope,
    Nonce, canonical,
    codec::{Channel, Codec},
    event::{self, EventType, PublicEvent},
    file::{FileManifest, HashPage},
    handshake, receipt,
    route::{self, RelayRoute, RouteRecord},
    wire::{self, Frame, Payload},
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

fn scope() -> NetworkScope {
    NetworkScope::new(84532, Address([7; 20])).unwrap()
}

fn frame(payload: Payload) -> Frame {
    let pre_session = matches!(payload, Payload::Hello(_) | Payload::HelloAck(_));
    Frame {
        version: 1,
        scope: Some((&scope()).into()),
        session_id: if pre_session { vec![] } else { vec![1; 32] },
        context_id: if pre_session
            || matches!(
                payload,
                Payload::OpenUserContext(_)
                    | Payload::Ping(_)
                    | Payload::Pong(_)
                    | Payload::Status(_)
            ) {
            vec![]
        } else {
            vec![2; 32]
        },
        request_id: vec![3; 16],
        payload: Some(payload),
    }
}

fn hello(nonce: u8) -> wire::Hello {
    handshake::hello(
        wire::NodeRole::Nas,
        Nonce([nonce; 32]),
        vec![
            wire::Capability::PublicEvents,
            wire::Capability::Sync,
            wire::Capability::Objects,
            wire::Capability::Chunks,
        ],
    )
    .unwrap()
}

fn key() -> k256::ecdsa::SigningKey {
    k256::ecdsa::SigningKey::from_bytes((&[1; 32]).into()).unwrap()
}
fn owner() -> Address {
    let point = key().verifying_key().to_encoded_point(false);
    Address::from_slice(&Keccak256::digest(&point.as_bytes()[1..])[12..]).unwrap()
}
fn post() -> PublicEvent {
    PublicEvent {
        scope: scope(),
        event_type: EventType::Post,
        actor_owner: owner(),
        event_nonce: Nonce([9; 32]),
        created_at: 1_790_000_000,
        subject_ref: None,
        target_owner: None,
        body: b"Hello from NAS".to_vec(),
        attachments: vec![],
    }
}
fn sign(payload: &PublicEvent) -> wire::SignedEvent {
    let (signature, recovery) = key()
        .sign_prehash_recoverable(&payload.signing_hash().unwrap())
        .unwrap();
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte() + 27);
    wire::SignedEvent {
        canonical_payload: payload.canonical_payload().unwrap(),
        signature_algorithm: wire::SignatureAlgorithm::EoaEip712 as i32,
        signature,
    }
}

#[test]
fn scope_rejects_wrong_chain_contract_and_zero_configuration() {
    assert_eq!(
        NetworkScope::new(0, Address([1; 20])),
        Err(Error::InvalidMessage)
    );
    assert_eq!(
        NetworkScope::new(1, Address([0; 20])),
        Err(Error::InvalidMessage)
    );
    let mut other = scope();
    other.chain_id += 1;
    assert_eq!(other.ensure_matches(&scope()), Err(Error::NetworkMismatch));
    other = scope();
    other.registry_contract = Address([8; 20]);
    assert_eq!(other.ensure_matches(&scope()), Err(Error::NetworkMismatch));
}

#[test]
fn sha256_multihash_has_standard_prefix_and_detects_corruption() {
    let id = ContentId::of(b"abc");
    assert_eq!(
        hex::encode(id.as_bytes()),
        "1220ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(ContentId::from_slice(&[0; 34]).is_err());
    assert_eq!(id.verify(b"abd"), Err(Error::IntegrityFailed));
}

#[test]
fn protobuf_framing_handles_partial_and_concatenated_input() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    let frame = frame(Payload::Hello(hello(1)));
    let encoded = codec.encode(&frame).unwrap();
    for end in 0..encoded.len() {
        assert_eq!(codec.decode(&encoded[..end]), Err(Error::Incomplete));
    }
    let joined = [encoded.clone(), encoded.clone()].concat();
    let (decoded, used) = codec.decode(&joined).unwrap();
    assert_eq!(decoded, frame);
    assert_eq!(used, encoded.len());
    assert_eq!(codec.decode(&joined[used..]).unwrap().0, frame);
}

#[test]
fn length_prefix_rejects_oversize_overflow_and_nonminimal_values_before_body() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    assert_eq!(
        codec.decode_length(&[0x81, 0x80, 0x04]),
        Err(Error::ResourceExhausted)
    );
    assert_eq!(
        codec.decode_length(&[0xff, 0xff, 0xff, 0xff, 0x10]),
        Err(Error::InvalidMessage)
    );
    assert_eq!(codec.decode_length(&[0x81, 0]), Err(Error::NonCanonical));
    assert_eq!(codec.decode_length(&[0]), Err(Error::InvalidMessage));
    assert_eq!(codec.decode_length(&[0x80]), Ok(None));
    assert_eq!(codec.decode_length(&[0xff; 6]), Err(Error::InvalidMessage));
}

#[test]
fn malformed_protobuf_and_unknown_payload_are_rejected() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    assert!(codec.decode(&[1, 0xff]).is_err());
    let mut unknown = frame(Payload::Ping(vec![1; 32]));
    unknown.payload = None;
    assert_eq!(
        codec.decode(&unknown.encode_length_delimited_to_vec()),
        Err(Error::UnsupportedCapability)
    );
}

#[test]
fn duplicate_headers_multiple_payloads_and_unknown_semantic_fields_are_rejected() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    let original = frame(Payload::Ping(vec![4; 32])).encode_to_vec();
    for extra in [
        vec![8, 1],
        [vec![0xda, 1, 32], vec![5; 32]].concat(),
        vec![0xf0, 1, 1],
    ] {
        let raw = [original.clone(), extra].concat();
        let mut framed = Vec::new();
        prost::encode_length_delimiter(raw.len(), &mut framed).unwrap();
        framed.extend_from_slice(&raw);
        assert!(codec.decode(&framed).is_err());
    }
}

#[test]
fn public_event_matches_independent_eip712_reference_vector() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("vectors/public-event.json")).unwrap();
    let payload = post();
    assert_eq!(
        hex::encode(payload.canonical_payload().unwrap()),
        vector["payload"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(payload.signing_hash().unwrap()),
        vector["hash"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(payload.event_id().unwrap().as_bytes()),
        vector["event_id"].as_str().unwrap()
    );
    let event = wire::SignedEvent {
        canonical_payload: hex::decode(vector["payload"].as_str().unwrap()).unwrap(),
        signature_algorithm: wire::SignatureAlgorithm::EoaEip712 as i32,
        signature: hex::decode(vector["signature"].as_str().unwrap()).unwrap(),
    };
    event::verify(&event, &scope()).unwrap();
}

#[test]
fn protobuf_frame_matches_protoc_reference_vector() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    let encoded = codec.encode(&frame(Payload::Ping(vec![4; 32]))).unwrap();
    let (length, prefix) = codec.decode_length(&encoded).unwrap().unwrap();
    let reference = hex::decode(include_str!("vectors/ping-frame.hex").trim()).unwrap();
    assert_eq!(&encoded[prefix..prefix + length], reference);
}

#[test]
fn high_s_and_invalid_recovery_signatures_are_rejected() {
    let mut event = sign(&post());
    event.signature[64] = 35;
    assert!(matches!(
        event::verify(&event, &scope()),
        Err(Error::InvalidSignature)
    ));
    event.signature[64] = 27;
    event.signature[32..64].copy_from_slice(
        &hex::decode("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140").unwrap(),
    );
    assert!(matches!(
        event::verify(&event, &scope()),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn malformed_cbor_inputs_are_bounded_and_truncated_payloads_fail() {
    for byte in 0u8..=255 {
        let _ = canonical::decode::<Small>(&[byte], 100);
    }
    let encoded = post().canonical_payload().unwrap();
    for length in 0..encoded.len() {
        assert!(canonical::decode::<PublicEvent>(&encoded[..length], MAX_OBJECT).is_err());
    }
}

#[test]
fn frame_enforces_network_version_channel_and_identifier_phase() {
    let control = Codec::new(scope(), Channel::Control).unwrap();
    let content = Codec::new(scope(), Channel::Content).unwrap();
    let mut message = frame(Payload::Ping(vec![4; 32]));
    message.version = 2;
    assert_eq!(control.encode(&message), Err(Error::UnsupportedVersion));
    message.version = 1;
    message.scope.as_mut().unwrap().chain_id += 1;
    assert_eq!(control.encode(&message), Err(Error::NetworkMismatch));
    message.scope = Some((&scope()).into());
    message.session_id.clear();
    assert_eq!(control.encode(&message), Err(Error::InvalidMessage));
    let hello = frame(Payload::Hello(hello(1)));
    assert_eq!(content.encode(&hello), Err(Error::InvalidMessage));
    let mut hello = hello;
    hello.session_id = vec![1; 32];
    assert_eq!(control.encode(&hello), Err(Error::InvalidMessage));
    let mut want = frame(Payload::EventWant(wire::EventRef {
        id: ContentId::of(b"x").as_bytes().to_vec(),
    }));
    want.context_id.clear();
    assert_eq!(control.encode(&want), Err(Error::InvalidMessage));
}

#[test]
fn offer_sync_and_error_fields_are_bounded() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    let mut offer = wire::EventOffer {
        event_id: ContentId::of(b"x").as_bytes().to_vec(),
        actor_owner: vec![1; 20],
        event_kind: wire::EventKind::Post as i32,
        encoded_size: 256,
    };
    assert!(
        codec
            .encode(&frame(Payload::EventOffer(offer.clone())))
            .is_ok()
    );
    offer.encoded_size = MAX_OBJECT as u32 + 1;
    assert_eq!(
        codec.encode(&frame(Payload::EventOffer(offer))),
        Err(Error::ResourceExhausted)
    );
    let request = wire::SyncRequest {
        stream_id: vec![1; 32],
        after_cursor: 0,
        limit: 101,
    };
    assert_eq!(
        codec.encode(&frame(Payload::SyncRequest(request))),
        Err(Error::ResourceExhausted)
    );
    let batch = wire::SyncBatch {
        stream_id: vec![1; 32],
        event_ids: vec![ContentId::of(b"x").as_bytes().to_vec(); 2],
        next_cursor: 2,
        has_more: false,
    };
    assert_eq!(
        codec.encode(&frame(Payload::SyncBatch(batch))),
        Err(Error::InvalidMessage)
    );
    for text in ["错误", "Line\nbreak", ""] {
        let status = wire::Status {
            code: wire::ErrorCode::InvalidMessage as i32,
            message: text.into(),
            retry_after_seconds: None,
        };
        assert_eq!(
            codec.encode(&frame(Payload::Status(status))),
            Err(Error::InvalidMessage)
        );
    }
}

#[test]
fn content_integrity_and_payload_size_are_enforced() {
    let codec = Codec::new(scope(), Channel::Content).unwrap();
    let mut object = wire::ObjectData {
        id: ContentId::of(b"payload").as_bytes().to_vec(),
        data: b"payload".to_vec(),
    };
    assert!(
        codec
            .encode(&frame(Payload::ObjectData(object.clone())))
            .is_ok()
    );
    object.data[0] ^= 1;
    assert_eq!(
        codec.encode(&frame(Payload::ObjectData(object))),
        Err(Error::IntegrityFailed)
    );
    let chunk = wire::ChunkData {
        object_id: ContentId::of(b"file").as_bytes().to_vec(),
        index: 0,
        data: vec![1; MAX_CHUNK + 1],
        chunk_id: ContentId::of(b"chunk").as_bytes().to_vec(),
    };
    assert_eq!(
        codec.encode(&frame(Payload::ChunkData(chunk))),
        Err(Error::ResourceExhausted)
    );
    let codec = Codec::new(scope(), Channel::Control)
        .unwrap()
        .with_max_frame_bytes(64)
        .unwrap();
    assert_eq!(
        codec.encode(&frame(Payload::Hello(hello(1)))),
        Err(Error::ResourceExhausted)
    );
}

#[test]
fn context_messages_do_not_accept_device_or_wallet_powers_as_service_capabilities() {
    let codec = Codec::new(scope(), Channel::Control).unwrap();
    let mut context = wire::OpenUserContext {
        acting_owner: vec![1; 20],
        target_owner: vec![2; 20],
        grant: vec![3],
        requested_capabilities: vec!["event.deliver".into()],
        proof_signature: vec![4; 64],
    };
    assert!(
        codec
            .encode(&frame(Payload::OpenUserContext(context.clone())))
            .is_ok()
    );
    context.requested_capabilities = vec!["post.sign".into()];
    assert_eq!(
        codec.encode(&frame(Payload::OpenUserContext(context))),
        Err(Error::UnsupportedCapability)
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Small {
    a: u64,
    z: bool,
}

#[test]
fn canonical_cbor_matches_golden_bytes_and_rejects_ambiguous_values() {
    assert_eq!(
        canonical::encode(&Small { a: 1, z: true }, 100).unwrap(),
        hex::decode("a2616101617af5").unwrap()
    );
    assert_eq!(
        canonical::decode::<Small>(&hex::decode("a2616101617af5").unwrap(), 100).unwrap(),
        Small { a: 1, z: true }
    );
    for bytes in [
        "a2617af5616101",       // unsorted map
        "a261611801617af5",     // overlong integer
        "a3616101616101617af5", // duplicate key
        "bf616101617af5ff",     // indefinite map
        "a2616120617af5",       // negative integer
        "a26161c001617af5",     // tag
        "a26161f93c00617af5",   // float
        "a2616101617af500",     // trailing object
        "a3616101617af5617800", // unknown signed field
        "9bffffffffffffffff",   // malicious array allocation
    ] {
        assert!(
            canonical::decode::<Small>(&hex::decode(bytes).unwrap(), 100).is_err(),
            "{bytes}"
        );
    }
    assert_eq!(
        canonical::encode(&Small { a: 1, z: true }, 1),
        Err(Error::ResourceExhausted)
    );
    let deeply_nested = [vec![0x81; 30], vec![0]].concat();
    assert_eq!(
        canonical::decode::<Small>(&deeply_nested, 100),
        Err(Error::ResourceExhausted)
    );
}

#[test]
fn signed_public_event_roundtrips_and_signature_does_not_change_event_id() {
    let payload = post();
    let signed = sign(&payload);
    let verified = event::verify(&signed, &scope()).unwrap();
    assert_eq!(verified.payload(), &payload);
    assert_eq!(verified.event_id(), payload.event_id().unwrap());
    let mut alternate_v = signed.clone();
    alternate_v.signature[64] -= 27;
    assert_eq!(
        event::verify(&alternate_v, &scope()).unwrap().event_id(),
        verified.event_id()
    );
    assert_eq!(
        hex::encode(owner().0),
        "1a642f0e3c3af545e7acbd38b07251b3990914f1"
    );
}

#[test]
fn public_event_rejects_tampering_wrong_network_and_unknown_signature_mode() {
    let signed = sign(&post());
    let mut corrupt = signed.clone();
    corrupt.signature[4] ^= 1;
    assert!(event::verify(&corrupt, &scope()).is_err());
    let mut other = scope();
    other.chain_id += 1;
    assert!(matches!(
        event::verify(&signed, &other),
        Err(Error::NetworkMismatch)
    ));
    let mut unsupported = signed.clone();
    unsupported.signature_algorithm = 999;
    assert!(matches!(
        event::verify(&unsupported, &scope()),
        Err(Error::UnsupportedCapability)
    ));
    let mut changed = post();
    changed.body.push(b'!');
    let mut mismatch = signed;
    mismatch.canonical_payload = changed.canonical_payload().unwrap();
    assert!(matches!(
        event::verify(&mismatch, &scope()),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn event_semantics_and_replay_keys_do_not_depend_on_packet_count() {
    let mut payload = post();
    payload.event_type = EventType::Like;
    assert_eq!(payload.validate(), Err(Error::InvalidMessage));
    payload.body.clear();
    payload.subject_ref = Some(ContentId::of(b"post"));
    let first = event::verify(&sign(&payload), &scope()).unwrap();
    payload.event_nonce = Nonce([8; 32]);
    let second = event::verify(&sign(&payload), &scope()).unwrap();
    assert_ne!(first.event_id(), second.event_id());
    assert_eq!(first.semantic_key(), second.semantic_key());
    payload.event_type = EventType::Follow;
    payload.subject_ref = None;
    payload.target_owner = Some(payload.actor_owner);
    assert_eq!(payload.validate(), Err(Error::InvalidMessage));
}

fn route_record(node: &Keypair, relay: &Keypair) -> RouteRecord {
    let relay_peer = relay.public().to_peer_id();
    let subject = node.public().to_peer_id();
    let address: Multiaddr =
        format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay_peer}/p2p-circuit/p2p/{subject}")
            .parse()
            .unwrap();
    RouteRecord {
        scope: scope(),
        subject_peer_id: subject.to_bytes(),
        revision: 1,
        issued_at: 1000,
        expires_at: 1500,
        relay_routes: vec![RelayRoute {
            relay_peer_id: relay_peer.to_bytes(),
            circuit_multiaddr: address.to_vec(),
            reservation_expires_at: 1800,
        }],
    }
}

#[test]
fn signed_relay_routes_validate_node_identity_scope_lifetime_and_revision() {
    let node = Keypair::generate_ed25519();
    let relay = Keypair::generate_ed25519();
    let mut record = route_record(&node, &relay);
    let first = route::verify(
        &record.sign(&node, 1000).unwrap(),
        &scope(),
        node.public().to_peer_id(),
        1100,
    )
    .unwrap();
    let checkpoint = first.checkpoint(None).unwrap();
    assert!(first.checkpoint(Some(&checkpoint)).is_ok());
    record.revision = 2;
    let second = route::verify(
        &record.sign(&node, 1000).unwrap(),
        &scope(),
        node.public().to_peer_id(),
        1100,
    )
    .unwrap();
    let checkpoint = second.checkpoint(Some(&checkpoint)).unwrap();
    assert!(matches!(
        first.checkpoint(Some(&checkpoint)),
        Err(Error::Replay)
    ));
    record.expires_at += 1;
    let conflict = route::verify(
        &record.sign(&node, 1000).unwrap(),
        &scope(),
        node.public().to_peer_id(),
        1100,
    )
    .unwrap();
    assert!(matches!(
        conflict.checkpoint(Some(&checkpoint)),
        Err(Error::Conflict)
    ));
    assert!(checkpoint.retain_until >= record.expires_at - 1 + route::CLOCK_SKEW);
}

#[test]
fn route_rejects_wrong_signer_expired_records_wrong_scope_and_envelope_domain() {
    let node = Keypair::generate_ed25519();
    let relay = Keypair::generate_ed25519();
    let record = route_record(&node, &relay);
    assert!(matches!(
        record.sign(&relay, 1000),
        Err(Error::InvalidSignature)
    ));
    let encoded = record.sign(&node, 1000).unwrap();
    assert!(matches!(
        route::verify(&encoded, &scope(), node.public().to_peer_id(), 1620),
        Err(Error::Expired)
    ));
    assert!(matches!(
        route::verify(&encoded, &scope(), relay.public().to_peer_id(), 1100),
        Err(Error::InvalidSignature)
    ));
    let mut other = scope();
    other.registry_contract = Address([8; 20]);
    assert!(matches!(
        route::verify(&encoded, &other, node.public().to_peer_id(), 1100),
        Err(Error::NetworkMismatch)
    ));
    let wrong_domain = SignedEnvelope::new(
        &node,
        "other".into(),
        route::ROUTE_PAYLOAD_TYPE.to_vec(),
        canonical::encode(&record, MAX_ROUTE).unwrap(),
    )
    .unwrap()
    .into_protobuf_encoding();
    assert!(matches!(
        route::verify(&wrong_domain, &scope(), node.public().to_peer_id(), 1100),
        Err(Error::InvalidSignature)
    ));
    assert!(matches!(
        route::verify(
            &vec![0; MAX_ROUTE + 1],
            &scope(),
            node.public().to_peer_id(),
            1100
        ),
        Err(Error::ResourceExhausted)
    ));
    assert_ne!(
        route::discovery_key(&scope(), node.public().to_peer_id()).unwrap(),
        route::discovery_key(&other, node.public().to_peer_id()).unwrap()
    );
}

#[test]
fn route_rejects_private_addresses_extra_hops_and_reservation_mismatch() {
    let node = Keypair::generate_ed25519();
    let relay = Keypair::generate_ed25519();
    let mut record = route_record(&node, &relay);
    let subject = node.public().to_peer_id();
    let relay_peer = relay.public().to_peer_id();
    for host in [
        "/ip4/127.0.0.1",
        "/ip4/192.168.1.1",
        "/ip4/169.254.169.254",
        "/ip4/100.64.0.1",
        "/ip6/::1",
        "/dns4/nas.local",
        "/dns4/localhost",
    ] {
        record.relay_routes[0].circuit_multiaddr =
            format!("{host}/tcp/4001/p2p/{relay_peer}/p2p-circuit/p2p/{subject}")
                .parse::<Multiaddr>()
                .unwrap()
                .to_vec();
        assert!(record.validate(&scope(), 1100).is_err(), "{host}");
    }
    record = route_record(&node, &relay);
    record.relay_routes[0].reservation_expires_at = 1499;
    assert!(record.validate(&scope(), 1100).is_err());
    record = route_record(&node, &relay);
    record.relay_routes[0].circuit_multiaddr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay_peer}/p2p-circuit/p2p/{relay_peer}/p2p-circuit/p2p/{subject}").parse::<Multiaddr>().unwrap().to_vec();
    assert!(record.validate(&scope(), 1100).is_err());
}

#[test]
fn handshake_is_symmetric_and_binds_limits_nonces_and_peers() {
    let a = Keypair::generate_ed25519().public().to_peer_id();
    let b = Keypair::generate_ed25519().public().to_peer_id();
    let left = hello(1);
    let mut right = hello(2);
    right.max_concurrent_streams = 2;
    let one = handshake::negotiate(&scope(), a, &left, b, &right).unwrap();
    let two = handshake::negotiate(&scope(), b, &right, a, &left).unwrap();
    assert_eq!(one.session_id, two.session_id);
    assert_eq!(one.max_concurrent_streams, 2);
    right.max_concurrent_streams = 1;
    assert_ne!(
        one.session_id,
        handshake::negotiate(&scope(), a, &left, b, &right)
            .unwrap()
            .session_id
    );
    let mut ping = frame(Payload::Ping(vec![3; 32]));
    ping.session_id = one.session_id.0.to_vec();
    assert!(one.check_frame(&ping).is_ok());
    ping.session_id[0] ^= 1;
    assert_eq!(one.check_frame(&ping), Err(Error::Replay));
    assert!(handshake::negotiate(&scope(), a, &left, a, &right).is_err());
}

#[test]
fn handshake_rejects_incompatible_versions_capabilities_and_resource_claims() {
    let a = Keypair::generate_ed25519().public().to_peer_id();
    let b = Keypair::generate_ed25519().public().to_peer_id();
    let mut remote = hello(2);
    remote.supported_versions = vec![2];
    assert!(matches!(
        handshake::negotiate(&scope(), a, &hello(1), b, &remote),
        Err(Error::UnsupportedVersion)
    ));
    remote = hello(2);
    remote.capabilities.push(999);
    assert_eq!(
        handshake::validate_hello(&remote),
        Err(Error::UnsupportedCapability)
    );
    remote = hello(2);
    remote.max_control_frame = MAX_CONTROL_FRAME as u32 + 1;
    assert_eq!(
        handshake::validate_hello(&remote),
        Err(Error::ResourceExhausted)
    );
    remote = hello(2);
    remote.capabilities.clear();
    let negotiated = handshake::negotiate(&scope(), a, &hello(1), b, &remote).unwrap();
    let mut offer = frame(Payload::EventWant(wire::EventRef {
        id: ContentId::of(b"x").as_bytes().to_vec(),
    }));
    offer.session_id = negotiated.session_id.0.to_vec();
    assert_eq!(
        negotiated.check_frame(&offer),
        Err(Error::UnsupportedCapability)
    );
}

fn manifest_fixture() -> (FileManifest, Vec<u8>, Vec<u8>) {
    let bytes = vec![4; MAX_CHUNK + 17];
    let object = ContentId::of(&bytes);
    let page = HashPage {
        object_id: object,
        page_index: 0,
        chunk_ids: vec![
            ContentId::of(&bytes[..MAX_CHUNK]),
            ContentId::of(&bytes[MAX_CHUNK..]),
        ],
    }
    .encode()
    .unwrap();
    let manifest = FileManifest {
        object_id: object,
        total_length: bytes.len() as u64,
        chunk_size: MAX_CHUNK as u32,
        chunk_count: 2,
        hash_page_ids: vec![ContentId::of(&page)],
        media_type: "application/octet-stream".into(),
    };
    (manifest, bytes, page)
}

#[test]
fn manifests_validate_paged_hashes_exact_chunk_sizes_and_whole_file_hash() {
    let (manifest, bytes, page) = manifest_fixture();
    assert_eq!(
        FileManifest::decode(&manifest.encode().unwrap()).unwrap(),
        manifest
    );
    manifest
        .verify_chunk(0, &bytes[..MAX_CHUNK], &page)
        .unwrap();
    manifest
        .verify_chunk(1, &bytes[MAX_CHUNK..], &page)
        .unwrap();
    manifest.object_id.verify(&bytes).unwrap();
    assert_eq!(
        manifest.verify_chunk(2, b"x", &page),
        Err(Error::InvalidMessage)
    );
    assert_eq!(
        manifest.verify_chunk(1, b"wrong", &page),
        Err(Error::IntegrityFailed)
    );
    let mut corrupt = page.clone();
    corrupt[0] ^= 1;
    assert!(manifest.verify_page(0, &corrupt).is_err());
    let mut oversized = manifest.clone();
    oversized.total_length = red_leaves_protocol::file::MAX_FILE_BYTES + 1;
    assert_eq!(oversized.validate(), Err(Error::ResourceExhausted));
    let mut wrong_count = manifest;
    wrong_count.chunk_count += 1;
    assert_eq!(wrong_count.validate(), Err(Error::ResourceExhausted));
}

#[test]
fn smaller_negotiated_chunks_and_untrusted_hash_page_sizes_are_bounded() {
    let bytes = b"small file";
    let object_id = ContentId::of(bytes);
    let page = HashPage {
        object_id,
        page_index: 0,
        chunk_ids: bytes.chunks(4).map(ContentId::of).collect(),
    }
    .encode()
    .unwrap();
    let mut manifest = FileManifest {
        object_id,
        total_length: bytes.len() as u64,
        chunk_size: 4,
        chunk_count: 3,
        hash_page_ids: vec![ContentId::of(&page)],
        media_type: "application/octet-stream".into(),
    };
    manifest.verify_chunk(2, &bytes[8..], &page).unwrap();
    assert_eq!(
        manifest.verify_page(0, &vec![0; MAX_OBJECT + 1]),
        Err(Error::ResourceExhausted)
    );
    manifest.chunk_size = 0;
    assert_eq!(manifest.validate(), Err(Error::ResourceExhausted));
}

fn storage_receipt(recipient: Address, nas: &Keypair, event: ContentId) -> receipt::StorageReceipt {
    receipt::StorageReceipt {
        scope: scope(),
        recipient_owner: recipient,
        recipient_nas_peer_id: nas.public().to_peer_id().to_bytes(),
        event_id: event,
        body_stored: true,
        attachments_stored: false,
        receipt_nonce: Nonce([4; 32]),
        committed_at: 1100,
        service_grant_ref: ContentId::of(b"validated service grant"),
    }
}

#[test]
fn receipts_bind_service_signer_recipient_node_event_and_saved_parts() {
    let nas = Keypair::generate_ed25519();
    let service = Keypair::generate_ed25519();
    let event_id = ContentId::of(b"event");
    let receipt = storage_receipt(owner(), &nas, event_id);
    let encoded = receipt.sign(&service).unwrap();
    let verified = receipt::verify(
        &encoded,
        &scope(),
        &service.public(),
        owner(),
        nas.public().to_peer_id(),
        event_id,
    )
    .unwrap();
    assert!(verified.body_stored);
    assert!(!verified.attachments_stored);
    assert!(
        receipt::verify(
            &encoded,
            &scope(),
            &nas.public(),
            owner(),
            nas.public().to_peer_id(),
            event_id
        )
        .is_err()
    );
    assert!(
        receipt::verify(
            &encoded,
            &scope(),
            &service.public(),
            Address([2; 20]),
            nas.public().to_peer_id(),
            event_id
        )
        .is_err()
    );
    assert!(
        receipt::verify(
            &encoded,
            &scope(),
            &service.public(),
            owner(),
            service.public().to_peer_id(),
            event_id
        )
        .is_err()
    );
    assert!(
        receipt::verify(
            &encoded,
            &scope(),
            &service.public(),
            owner(),
            nas.public().to_peer_id(),
            ContentId::of(b"other event")
        )
        .is_err()
    );
}

#[test]
fn two_nas_exchange_public_event_and_final_receipt_through_opaque_bytes() {
    // In-memory protocol interop, not an actual libp2p/NAT/relay integration test.
    let a = Keypair::generate_ed25519();
    let b = Keypair::generate_ed25519();
    let service_b = Keypair::generate_ed25519();
    let session = handshake::negotiate(
        &scope(),
        a.public().to_peer_id(),
        &hello(1),
        b.public().to_peer_id(),
        &hello(2),
    )
    .unwrap();
    let receiver = Address([6; 20]);
    let signed = sign(&post());
    let mut data = frame(Payload::EventData(signed));
    data.session_id = session.session_id.0.to_vec();
    let content = Codec::new(scope(), Channel::Content).unwrap();
    let opaque_bytes = content.encode(&data).unwrap();
    let (received, _) = content.decode(&opaque_bytes).unwrap();
    session.check_frame(&received).unwrap();
    let Payload::EventData(event) = received.payload.unwrap() else {
        panic!("Expected event data");
    };
    let verified = event::verify(&event, &scope()).unwrap();
    // A real node checks policy and commits files + DB before constructing this claim.
    let claim = storage_receipt(receiver, &b, verified.event_id())
        .sign(&service_b)
        .unwrap();
    let mut ack = frame(Payload::EventCommitted(wire::SignedReceipt {
        envelope: claim,
    }));
    ack.session_id = session.session_id.0.to_vec();
    let control = Codec::new(scope(), Channel::Control).unwrap();
    let (received_ack, _) = control.decode(&control.encode(&ack).unwrap()).unwrap();
    session.check_frame(&received_ack).unwrap();
    let Payload::EventCommitted(receipt) = received_ack.payload.unwrap() else {
        panic!("Expected storage receipt");
    };
    receipt::verify(
        &receipt.envelope,
        &scope(),
        &service_b.public(),
        receiver,
        b.public().to_peer_id(),
        verified.event_id(),
    )
    .unwrap();
}
