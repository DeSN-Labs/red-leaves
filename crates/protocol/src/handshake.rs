//! Deterministic negotiation within an already authenticated libp2p connection.

use libp2p_identity::PeerId;
use prost::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    Error, MAX_CHUNK, MAX_CONTROL_FRAME, MAX_OBJECT_FRAME, NetworkScope, Nonce, Result, canonical,
    wire,
};

pub fn hello(
    role: wire::NodeRole,
    nonce: Nonce,
    capabilities: Vec<wire::Capability>,
) -> Result<wire::Hello> {
    let message = wire::Hello {
        node_role: role as i32,
        supported_versions: vec![1],
        capabilities: capabilities
            .into_iter()
            .map(|capability| capability as i32)
            .collect(),
        nonce: nonce.0.to_vec(),
        max_control_frame: MAX_CONTROL_FRAME as u32,
        max_object_frame: MAX_OBJECT_FRAME as u32,
        max_chunk_payload: MAX_CHUNK as u32,
        max_concurrent_streams: 4,
    };
    validate_hello(&message)?;
    Ok(message)
}

pub fn validate_hello(hello: &wire::Hello) -> Result<()> {
    match wire::NodeRole::try_from(hello.node_role) {
        Ok(wire::NodeRole::Nas | wire::NodeRole::Relay | wire::NodeRole::Discovery) => {}
        _ => return Err(Error::InvalidMessage),
    }
    crate::codec::identifier(&hello.nonce, 32)?;
    if hello.supported_versions.is_empty()
        || hello.supported_versions.len() > 8
        || hello
            .supported_versions
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || hello.supported_versions.contains(&0)
    {
        return Err(Error::InvalidMessage);
    }
    if hello.capabilities.len() > 8 || hello.capabilities.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Error::InvalidMessage);
    }
    for &capability in &hello.capabilities {
        match wire::Capability::try_from(capability) {
            Ok(wire::Capability::Unspecified) | Err(_) => return Err(Error::UnsupportedCapability),
            _ => {}
        }
    }
    if hello.max_control_frame < 1024
        || hello.max_control_frame as usize > MAX_CONTROL_FRAME
        || hello.max_object_frame < 1024
        || hello.max_object_frame as usize > MAX_OBJECT_FRAME
        || hello.max_chunk_payload == 0
        || hello.max_chunk_payload as usize > MAX_CHUNK
        || hello.max_concurrent_streams == 0
        || hello.max_concurrent_streams > 4
    {
        return Err(Error::ResourceExhausted);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Negotiated {
    pub session_id: Nonce,
    pub version: u32,
    pub capabilities: Vec<wire::Capability>,
    pub max_control_frame: u32,
    pub max_object_frame: u32,
    pub max_chunk_payload: u32,
    pub max_concurrent_streams: u32,
}

/// Both participants are sorted by authenticated PeerId, so either side derives the same ID.
/// The transcript binds all advertised parameters to prevent capability/limit ambiguity.
pub fn negotiate(
    scope: &NetworkScope,
    local_peer: PeerId,
    local: &wire::Hello,
    remote_peer: PeerId,
    remote: &wire::Hello,
) -> Result<Negotiated> {
    scope.validate()?;
    validate_hello(local)?;
    validate_hello(remote)?;
    if local_peer == remote_peer || local.nonce == remote.nonce {
        return Err(Error::InvalidMessage);
    }
    if !local.supported_versions.contains(&1) || !remote.supported_versions.contains(&1) {
        return Err(Error::UnsupportedVersion);
    }
    #[derive(Serialize)]
    struct Participant<'a> {
        #[serde(with = "serde_bytes")]
        peer_id: Vec<u8>,
        #[serde(with = "serde_bytes")]
        nonce: &'a [u8],
        role: i32,
        versions: &'a [u32],
        capabilities: &'a [i32],
        limits: [u32; 4],
    }
    #[derive(Serialize)]
    struct Transcript<'a> {
        domain: &'static str,
        scope: &'a NetworkScope,
        participants: Vec<Participant<'a>>,
        selected_version: u32,
    }
    let mut participants =
        [(local_peer, local), (remote_peer, remote)].map(|(peer, hello)| Participant {
            peer_id: peer.to_bytes(),
            nonce: &hello.nonce,
            role: hello.node_role,
            versions: &hello.supported_versions,
            capabilities: &hello.capabilities,
            limits: [
                hello.max_control_frame,
                hello.max_object_frame,
                hello.max_chunk_payload,
                hello.max_concurrent_streams,
            ],
        });
    participants.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    let transcript = canonical::encode(
        &Transcript {
            domain: "red-leaves/session/1",
            scope,
            participants: participants.into(),
            selected_version: 1,
        },
        MAX_CONTROL_FRAME,
    )?;
    Ok(Negotiated {
        session_id: Nonce(Sha256::digest(transcript).into()),
        version: 1,
        capabilities: local
            .capabilities
            .iter()
            .filter(|capability| remote.capabilities.contains(capability))
            .map(|capability| {
                wire::Capability::try_from(*capability).map_err(|_| Error::UnsupportedCapability)
            })
            .collect::<Result<_>>()?,
        max_control_frame: local.max_control_frame.min(remote.max_control_frame),
        max_object_frame: local.max_object_frame.min(remote.max_object_frame),
        max_chunk_payload: local.max_chunk_payload.min(remote.max_chunk_payload),
        max_concurrent_streams: local
            .max_concurrent_streams
            .min(remote.max_concurrent_streams),
    })
}

impl Negotiated {
    /// Check session binding/capabilities after codec validation. User authorization
    /// and lookup of the context in a connection-local table remain mandatory.
    pub fn check_frame(&self, frame: &wire::Frame) -> Result<()> {
        if frame.session_id != self.session_id.0 {
            return Err(Error::Replay);
        }
        use wire::{Capability, Payload};
        let payload = frame.payload.as_ref().ok_or(Error::InvalidMessage)?;
        let content = matches!(
            payload,
            Payload::EventData(_) | Payload::ObjectData(_) | Payload::ChunkData(_)
        );
        let maximum = if content {
            self.max_object_frame
        } else {
            self.max_control_frame
        };
        if frame.encoded_len() > maximum as usize {
            return Err(Error::ResourceExhausted);
        }
        if let Payload::ChunkData(chunk) = payload
            && chunk.data.len() > self.max_chunk_payload as usize
        {
            return Err(Error::ResourceExhausted);
        }
        if let Payload::ChunkGet(chunk) = payload
            && chunk.byte_credit > self.max_chunk_payload
        {
            return Err(Error::ResourceExhausted);
        }
        let needed = match frame.payload.as_ref().ok_or(Error::InvalidMessage)? {
            Payload::Hello(_) | Payload::HelloAck(_) => return Err(Error::InvalidMessage),
            Payload::EventOffer(_)
            | Payload::EventWant(_)
            | Payload::EventReject(_)
            | Payload::EventData(_)
            | Payload::EventCommitted(_) => Some(Capability::PublicEvents),
            Payload::SyncSummary(_) | Payload::SyncRequest(_) | Payload::SyncBatch(_) => {
                Some(Capability::Sync)
            }
            Payload::ObjectGet(_) | Payload::ObjectData(_) => Some(Capability::Objects),
            Payload::ChunkGet(_) | Payload::ChunkData(_) => Some(Capability::Chunks),
            _ => None,
        };
        if needed.is_some_and(|capability| !self.capabilities.contains(&capability)) {
            return Err(Error::UnsupportedCapability);
        }
        Ok(())
    }
}
