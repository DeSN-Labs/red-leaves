//! Protobuf DTOs. Field numbers are part of wire version 1; never reuse them.
//! Use `codec` to frame and validate network input instead of raw `prost::decode`.

use prost::{Enumeration, Message, Oneof};

#[derive(Clone, PartialEq, Eq, Message)]
pub struct Scope {
    #[prost(string, tag = "1")]
    pub protocol_family: String,
    #[prost(uint32, tag = "2")]
    pub network_version: u32,
    #[prost(uint64, tag = "3")]
    pub chain_id: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub registry_contract: Vec<u8>,
}

impl From<&crate::NetworkScope> for Scope {
    fn from(scope: &crate::NetworkScope) -> Self {
        Self {
            protocol_family: scope.protocol_family.clone(),
            network_version: scope.network_version,
            chain_id: scope.chain_id,
            registry_contract: scope.registry_contract.0.to_vec(),
        }
    }
}

impl TryFrom<&Scope> for crate::NetworkScope {
    type Error = crate::Error;
    fn try_from(scope: &Scope) -> crate::Result<Self> {
        let value = Self {
            protocol_family: scope.protocol_family.clone(),
            network_version: scope.network_version,
            chain_id: scope.chain_id,
            registry_contract: crate::Address::from_slice(&scope.registry_contract)?,
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Frame {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(message, optional, tag = "2")]
    pub scope: Option<Scope>,
    #[prost(bytes = "vec", tag = "3")]
    pub session_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub context_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub request_id: Vec<u8>,
    #[prost(
        oneof = "Payload",
        tags = "10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29"
    )]
    pub payload: Option<Payload>,
}

#[derive(Clone, PartialEq, Oneof)]
pub enum Payload {
    #[prost(message, tag = "10")]
    Hello(Hello),
    #[prost(message, tag = "11")]
    HelloAck(Hello),
    #[prost(message, tag = "12")]
    OpenUserContext(OpenUserContext),
    #[prost(message, tag = "13")]
    ContextAccepted(ContextAccepted),
    #[prost(message, tag = "14")]
    EventOffer(EventOffer),
    #[prost(message, tag = "15")]
    EventWant(EventRef),
    #[prost(message, tag = "16")]
    EventReject(EventReject),
    #[prost(message, tag = "17")]
    EventData(SignedEvent),
    #[prost(message, tag = "18")]
    EventCommitted(SignedReceipt),
    #[prost(message, tag = "19")]
    SyncSummary(SyncSummary),
    #[prost(message, tag = "20")]
    SyncRequest(SyncRequest),
    #[prost(message, tag = "21")]
    SyncBatch(SyncBatch),
    #[prost(message, tag = "22")]
    ObjectGet(EventRef),
    #[prost(message, tag = "23")]
    ObjectData(ObjectData),
    #[prost(message, tag = "24")]
    ChunkGet(ChunkGet),
    #[prost(message, tag = "25")]
    ChunkData(ChunkData),
    #[prost(message, tag = "26")]
    Cancel(Cancel),
    #[prost(bytes, tag = "27")]
    Ping(Vec<u8>),
    #[prost(message, tag = "28")]
    Status(Status),
    #[prost(bytes, tag = "29")]
    Pong(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum NodeRole {
    Unspecified = 0,
    Nas = 1,
    Relay = 2,
    Discovery = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum Capability {
    Unspecified = 0,
    PublicEvents = 1,
    Sync = 2,
    Objects = 3,
    Chunks = 4,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct Hello {
    #[prost(enumeration = "NodeRole", tag = "1")]
    pub node_role: i32,
    #[prost(uint32, repeated, tag = "2")]
    pub supported_versions: Vec<u32>,
    #[prost(enumeration = "Capability", repeated, tag = "3")]
    pub capabilities: Vec<i32>,
    #[prost(bytes = "vec", tag = "4")]
    pub nonce: Vec<u8>,
    #[prost(uint32, tag = "5")]
    pub max_control_frame: u32,
    #[prost(uint32, tag = "6")]
    pub max_object_frame: u32,
    #[prost(uint32, tag = "7")]
    pub max_chunk_payload: u32,
    #[prost(uint32, tag = "8")]
    pub max_concurrent_streams: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct OpenUserContext {
    #[prost(bytes = "vec", tag = "1")]
    pub acting_owner: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub target_owner: Vec<u8>,
    /// Opaque wallet-signed grant; format and revocation policy are not frozen.
    #[prost(bytes = "vec", tag = "3")]
    pub grant: Vec<u8>,
    #[prost(string, repeated, tag = "4")]
    pub requested_capabilities: Vec<String>,
    #[prost(bytes = "vec", tag = "5")]
    pub proof_signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ContextAccepted {
    #[prost(string, repeated, tag = "1")]
    pub granted_capabilities: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct EventOffer {
    #[prost(bytes = "vec", tag = "1")]
    pub event_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub actor_owner: Vec<u8>,
    #[prost(enumeration = "EventKind", tag = "3")]
    pub event_kind: i32,
    #[prost(uint32, tag = "4")]
    pub encoded_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum EventKind {
    Unspecified = 0,
    Post = 1,
    Follow = 2,
    Like = 3,
}

#[derive(Clone, PartialEq, Message)]
pub struct EventRef {
    #[prost(bytes = "vec", tag = "1")]
    pub id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct EventReject {
    #[prost(bytes = "vec", tag = "1")]
    pub event_id: Vec<u8>,
    #[prost(enumeration = "ErrorCode", tag = "2")]
    pub code: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum SignatureAlgorithm {
    Unspecified = 0,
    EoaEip712 = 1,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignedEvent {
    #[prost(bytes = "vec", tag = "1")]
    pub canonical_payload: Vec<u8>,
    #[prost(enumeration = "SignatureAlgorithm", tag = "2")]
    pub signature_algorithm: i32,
    #[prost(bytes = "vec", tag = "3")]
    pub signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignedReceipt {
    /// Standard libp2p SignedEnvelope signed by the user's service key.
    #[prost(bytes = "vec", tag = "1")]
    pub envelope: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SyncSummary {
    #[prost(bytes = "vec", tag = "1")]
    pub stream_id: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub high_water: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct SyncRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub stream_id: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub after_cursor: u64,
    #[prost(uint32, tag = "3")]
    pub limit: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct SyncBatch {
    #[prost(bytes = "vec", tag = "1")]
    pub stream_id: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub event_ids: Vec<Vec<u8>>,
    #[prost(uint64, tag = "3")]
    pub next_cursor: u64,
    #[prost(bool, tag = "4")]
    pub has_more: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct ObjectData {
    #[prost(bytes = "vec", tag = "1")]
    pub id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ChunkGet {
    #[prost(bytes = "vec", tag = "1")]
    pub object_id: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub index: u32,
    #[prost(uint32, tag = "3")]
    pub byte_credit: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ChunkData {
    #[prost(bytes = "vec", tag = "1")]
    pub object_id: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub index: u32,
    #[prost(bytes = "vec", tag = "3")]
    pub data: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub chunk_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Cancel {
    #[prost(bytes = "vec", tag = "1")]
    pub request_id: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum ErrorCode {
    Unspecified = 0,
    UnsupportedVersion = 1,
    UnsupportedCapability = 2,
    NetworkMismatch = 3,
    InvalidMessage = 4,
    InvalidSignature = 5,
    ReplayDetected = 6,
    AuthRequired = 7,
    AuthExpired = 8,
    AuthRevoked = 9,
    UserNotRegistered = 10,
    BindingMismatch = 11,
    ChainUnavailable = 12,
    NotFoundOrForbidden = 13,
    RejectedByPolicy = 14,
    ResourceExhausted = 15,
    NoRoute = 16,
    RelayUnavailable = 17,
    RelayLimitReached = 18,
    RelayPaymentUnsupported = 19,
    DeadlineExceeded = 20,
    StorageUnavailable = 21,
    SyncGap = 22,
    IntegrityFailed = 23,
}

#[derive(Clone, PartialEq, Message)]
pub struct Status {
    #[prost(enumeration = "ErrorCode", tag = "1")]
    pub code: i32,
    /// Only use bounded static English descriptions, never echo user input.
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(uint32, optional, tag = "3")]
    pub retry_after_seconds: Option<u32>,
}
