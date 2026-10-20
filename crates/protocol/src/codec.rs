//! Bounded unsigned-varint framing. Incomplete input is retained by the caller.

use prost::Message;

use crate::{
    ContentId, Error, MAX_CHUNK, MAX_CHUNK_FRAME, MAX_CONTROL_FRAME, MAX_OBJECT, MAX_OBJECT_FRAME,
    MAX_ROUTE, MAX_SYNC_PAGE, NetworkScope, Result, WIRE_VERSION,
    wire::{self, Frame, Payload},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Control,
    Content,
}

pub struct Codec {
    scope: NetworkScope,
    channel: Channel,
    max_frame_bytes: usize,
}

impl Codec {
    pub fn new(scope: NetworkScope, channel: Channel) -> Result<Self> {
        let max_frame_bytes = match channel {
            Channel::Control => MAX_CONTROL_FRAME,
            Channel::Content => MAX_OBJECT_FRAME,
        };
        scope.validate()?;
        Ok(Self {
            scope,
            channel,
            max_frame_bytes,
        })
    }

    /// Apply a smaller negotiated bound; callers load runtime policy from node config.
    pub fn with_max_frame_bytes(mut self, bound: usize) -> Result<Self> {
        if bound == 0 || bound > self.max_frame_bytes {
            return Err(Error::ResourceExhausted);
        }
        self.max_frame_bytes = bound;
        Ok(self)
    }

    /// Read only the prefix before allocating the declared payload buffer.
    /// Returns None for a partial prefix; rejects oversize lengths immediately.
    pub fn decode_length(&self, input: &[u8]) -> Result<Option<(usize, usize)>> {
        let mut length = 0u32;
        for (index, &byte) in input.iter().take(5).enumerate() {
            if index == 4 && byte > 15 {
                return Err(Error::InvalidMessage);
            }
            length |= u32::from(byte & 127) << (index * 7);
            if byte & 128 == 0 {
                if index > 0 && byte == 0 {
                    return Err(Error::NonCanonical);
                }
                if length == 0 {
                    return Err(Error::InvalidMessage);
                }
                if length as usize > self.max_frame_bytes {
                    return Err(Error::ResourceExhausted);
                }
                return Ok(Some((length as usize, index + 1)));
            }
        }
        if input.len() >= 5 {
            Err(Error::InvalidMessage)
        } else {
            Ok(None)
        }
    }

    pub fn encode(&self, frame: &Frame) -> Result<Vec<u8>> {
        self.validate(frame)?;
        let length = frame.encoded_len();
        if length > self.max_frame_bytes {
            return Err(Error::ResourceExhausted);
        }
        Ok(frame.encode_length_delimited_to_vec())
    }

    /// Decode one frame and report consumed bytes, allowing multiple frames per buffer.
    /// This validates structure and integrity, not signatures or user permissions.
    pub fn decode(&self, input: &[u8]) -> Result<(Frame, usize)> {
        let (length, prefix) = self.decode_length(input)?.ok_or(Error::Incomplete)?;
        let end = prefix + length;
        let payload = input.get(prefix..end).ok_or(Error::Incomplete)?;
        validate_frame_fields(payload)?;
        let frame = Frame::decode(payload).map_err(|_| Error::InvalidMessage)?;
        self.validate(&frame)?;
        Ok((frame, end))
    }

    pub fn validate(&self, frame: &Frame) -> Result<()> {
        if frame.version != WIRE_VERSION {
            return Err(Error::UnsupportedVersion);
        }
        NetworkScope::try_from(frame.scope.as_ref().ok_or(Error::InvalidMessage)?)?
            .ensure_matches(&self.scope)?;
        identifier(&frame.request_id, 16)?;
        if frame.encoded_len() > self.max_frame_bytes {
            return Err(Error::ResourceExhausted);
        }
        let payload = frame.payload.as_ref().ok_or(Error::UnsupportedCapability)?;
        let is_content = matches!(
            payload,
            Payload::EventData(_) | Payload::ObjectData(_) | Payload::ChunkData(_)
        );
        if is_content != (self.channel == Channel::Content) {
            return Err(Error::InvalidMessage);
        }
        if matches!(payload, Payload::Hello(_) | Payload::HelloAck(_)) {
            if !frame.session_id.is_empty() || !frame.context_id.is_empty() {
                return Err(Error::InvalidMessage);
            }
        } else {
            identifier(&frame.session_id, 32)?;
            match payload {
                Payload::OpenUserContext(_) => {
                    if !frame.context_id.is_empty() {
                        return Err(Error::InvalidMessage);
                    }
                }
                Payload::Ping(_) | Payload::Pong(_) | Payload::Status(_) => {
                    if !frame.context_id.is_empty() {
                        identifier(&frame.context_id, 32)?;
                    }
                }
                _ => identifier(&frame.context_id, 32)?,
            }
        }
        match payload {
            Payload::Hello(hello) | Payload::HelloAck(hello) => {
                crate::handshake::validate_hello(hello)?
            }
            Payload::OpenUserContext(context) => {
                address(&context.acting_owner)?;
                address(&context.target_owner)?;
                bounded(&context.grant, MAX_ROUTE)?;
                bounded(&context.proof_signature, 128)?;
                capabilities(&context.requested_capabilities)?;
            }
            Payload::ContextAccepted(context) => capabilities(&context.granted_capabilities)?,
            Payload::EventOffer(offer) => {
                id(&offer.event_id)?;
                address(&offer.actor_owner)?;
                event_kind(offer.event_kind)?;
                if offer.encoded_size == 0
                    || offer.encoded_size as usize > MAX_OBJECT
                    || frame.encoded_len() > 4096
                {
                    return Err(Error::ResourceExhausted);
                }
            }
            Payload::EventWant(reference) | Payload::ObjectGet(reference) => id(&reference.id)?,
            Payload::EventReject(reject) => {
                id(&reject.event_id)?;
                error_code(reject.code)?;
            }
            Payload::EventData(event) => crate::event::validate_envelope(event)?,
            Payload::EventCommitted(receipt) => bounded(&receipt.envelope, MAX_ROUTE)?,
            Payload::SyncSummary(summary) => identifier(&summary.stream_id, 32)?,
            Payload::SyncRequest(request) => {
                identifier(&request.stream_id, 32)?;
                if request.limit == 0 || request.limit as usize > MAX_SYNC_PAGE {
                    return Err(Error::ResourceExhausted);
                }
            }
            Payload::SyncBatch(batch) => {
                identifier(&batch.stream_id, 32)?;
                if batch.event_ids.len() > MAX_SYNC_PAGE
                    || (batch.event_ids.is_empty() && batch.has_more)
                {
                    return Err(Error::ResourceExhausted);
                }
                let mut seen = std::collections::HashSet::new();
                for event in &batch.event_ids {
                    id(event)?;
                    if !seen.insert(event) {
                        return Err(Error::InvalidMessage);
                    }
                }
            }
            Payload::ObjectData(object) => {
                if object.data.len() > MAX_OBJECT {
                    return Err(Error::ResourceExhausted);
                }
                ContentId::from_slice(&object.id)?.verify(&object.data)?;
            }
            Payload::ChunkGet(request) => {
                id(&request.object_id)?;
                if request.index >= 8192
                    || request.byte_credit == 0
                    || request.byte_credit as usize > MAX_CHUNK
                {
                    return Err(Error::ResourceExhausted);
                }
            }
            Payload::ChunkData(chunk) => {
                id(&chunk.object_id)?;
                bounded(&chunk.data, MAX_CHUNK)?;
                if chunk.index >= 8192 || frame.encoded_len() > MAX_CHUNK_FRAME {
                    return Err(Error::ResourceExhausted);
                }
                ContentId::from_slice(&chunk.chunk_id)?.verify(&chunk.data)?;
                // The caller MUST additionally match chunk_id/index to its verified manifest.
            }
            Payload::Cancel(cancel) => identifier(&cancel.request_id, 16)?,
            Payload::Ping(nonce) | Payload::Pong(nonce) => identifier(nonce, 32)?,
            Payload::Status(status) => {
                error_code(status.code)?;
                if status.message.is_empty()
                    || status.message.len() > 256
                    || !status
                        .message
                        .bytes()
                        .all(|b| b.is_ascii_graphic() || b == b' ')
                {
                    return Err(Error::InvalidMessage);
                }
            }
        }
        Ok(())
    }
}

/// Reject conflicting duplicate headers/oneofs and unknown v1 semantic fields,
/// without imposing a field ordering on otherwise valid Protobuf encoders.
fn validate_frame_fields(mut bytes: &[u8]) -> Result<()> {
    use prost::encoding::{DecodeContext, WireType, decode_key, skip_field};
    let mut seen_headers = 0u8;
    let mut seen_payload = false;
    while !bytes.is_empty() {
        let (tag, kind) = decode_key(&mut bytes).map_err(|_| Error::InvalidMessage)?;
        match tag {
            1..=5 => {
                let mask = 1 << (tag - 1);
                if seen_headers & mask != 0 {
                    return Err(Error::InvalidMessage);
                }
                seen_headers |= mask;
                let expected = if tag == 1 {
                    WireType::Varint
                } else {
                    WireType::LengthDelimited
                };
                if kind != expected {
                    return Err(Error::InvalidMessage);
                }
            }
            10..=29 => {
                if seen_payload || kind != WireType::LengthDelimited {
                    return Err(Error::InvalidMessage);
                }
                seen_payload = true;
            }
            _ => return Err(Error::UnsupportedCapability),
        }
        skip_field(kind, tag, &mut bytes, DecodeContext::default())
            .map_err(|_| Error::InvalidMessage)?;
    }
    Ok(())
}

pub(crate) fn identifier(bytes: &[u8], length: usize) -> Result<()> {
    if bytes.len() != length || bytes.iter().all(|byte| *byte == 0) {
        return Err(Error::InvalidMessage);
    }
    Ok(())
}

fn address(bytes: &[u8]) -> Result<()> {
    identifier(bytes, 20)
}
fn id(bytes: &[u8]) -> Result<()> {
    ContentId::from_slice(bytes).map(|_| ())
}
fn bounded(bytes: &[u8], max: usize) -> Result<()> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(Error::ResourceExhausted);
    }
    Ok(())
}
fn event_kind(kind: i32) -> Result<()> {
    match wire::EventKind::try_from(kind) {
        Ok(wire::EventKind::Post | wire::EventKind::Follow | wire::EventKind::Like) => Ok(()),
        _ => Err(Error::UnsupportedCapability),
    }
}
fn error_code(code: i32) -> Result<()> {
    match wire::ErrorCode::try_from(code) {
        Ok(wire::ErrorCode::Unspecified) | Err(_) => Err(Error::InvalidMessage),
        Ok(_) => Ok(()),
    }
}
fn capabilities(values: &[String]) -> Result<()> {
    if values.is_empty() || values.len() > 8 {
        return Err(Error::ResourceExhausted);
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if !matches!(
            value.as_str(),
            "event.deliver" | "sync.request" | "content.fetch" | "receipt.sign"
        ) {
            return Err(Error::UnsupportedCapability);
        }
        if !seen.insert(value) {
            return Err(Error::InvalidMessage);
        }
    }
    Ok(())
}
