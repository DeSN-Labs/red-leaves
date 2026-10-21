//! Immutable public events signed directly by an EOA using EIP-712.
//! Device grants, private content and historical revocation decisions are not inferred.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::{Address, ContentId, Error, MAX_OBJECT, NetworkScope, Nonce, Result, canonical, wire};

pub const DOMAIN_NAME: &str = "RedLeaves";
pub const DOMAIN_VERSION: &str = "1";
pub const EVENT_TYPE: &str = "Event(bytes32 payloadHash)";
const ID_DOMAIN: &[u8] = b"red-leaves/event/1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Post,
    Follow,
    Like,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicEvent {
    pub scope: NetworkScope,
    pub event_type: EventType,
    pub actor_owner: Address,
    pub event_nonce: Nonce,
    /// Author-claimed display time, never proof of authorization or acceptance time.
    pub created_at: u64,
    pub subject_ref: Option<ContentId>,
    pub target_owner: Option<Address>,
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    pub attachments: Vec<ContentId>,
}

impl PublicEvent {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        crate::codec::identifier(&self.actor_owner.0, 20)?;
        crate::codec::identifier(&self.event_nonce.0, 32)?;
        if self.created_at == 0 || self.body.len() > MAX_OBJECT || self.attachments.len() > 32 {
            return Err(Error::ResourceExhausted);
        }
        for id in self.attachments.iter().chain(self.subject_ref.iter()) {
            ContentId::from_slice(id.as_bytes())?;
        }
        let mut ids = std::collections::HashSet::new();
        if self.attachments.iter().any(|id| !ids.insert(id)) {
            return Err(Error::InvalidMessage);
        }
        let valid = match self.event_type {
            EventType::Post => {
                self.subject_ref.is_none()
                    && self.target_owner.is_none()
                    && (!self.body.is_empty() || !self.attachments.is_empty())
            }
            EventType::Follow => {
                self.subject_ref.is_none()
                    && self.body.is_empty()
                    && self.attachments.is_empty()
                    && self
                        .target_owner
                        .is_some_and(|owner| owner != self.actor_owner && owner.0 != [0; 20])
            }
            EventType::Like => {
                self.subject_ref.is_some()
                    && self.target_owner.is_none()
                    && self.body.is_empty()
                    && self.attachments.is_empty()
            }
        };
        if !valid {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }

    pub fn canonical_payload(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical::encode(self, MAX_OBJECT)
    }

    /// Digest passed to EIP-712 EOA signing; do not personal-sign this digest.
    pub fn signing_hash(&self) -> Result<[u8; 32]> {
        Ok(typed_hash(
            &self.scope,
            &self.canonical_payload()?,
            EVENT_TYPE,
        ))
    }

    pub fn event_id(&self) -> Result<ContentId> {
        Ok(event_id(&self.canonical_payload()?))
    }
}

/// Signature verification does not imply the actor is followed or allowed to send.
pub struct VerifiedEvent {
    payload: PublicEvent,
    event_id: ContentId,
}

impl VerifiedEvent {
    pub fn payload(&self) -> &PublicEvent {
        &self.payload
    }
    pub fn event_id(&self) -> ContentId {
        self.event_id
    }

    /// Use in a recipient- and network-scoped uniqueness constraint, not as a packet counter.
    pub fn semantic_key(&self) -> SemanticKey {
        match self.payload.event_type {
            EventType::Post => SemanticKey::Post(self.event_id),
            EventType::Follow => {
                SemanticKey::Follow(self.payload.actor_owner, self.payload.target_owner)
            }
            EventType::Like => {
                SemanticKey::Like(self.payload.actor_owner, self.payload.subject_ref)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SemanticKey {
    Post(ContentId),
    Follow(Address, Option<Address>),
    Like(Address, Option<ContentId>),
}

pub fn validate_envelope(event: &wire::SignedEvent) -> Result<()> {
    if event.signature_algorithm != wire::SignatureAlgorithm::EoaEip712 as i32 {
        return Err(Error::UnsupportedCapability);
    }
    if event.canonical_payload.is_empty() || event.encoded_len() > MAX_OBJECT {
        return Err(Error::ResourceExhausted);
    }
    if event.signature.len() != 65 {
        return Err(Error::InvalidSignature);
    }
    Ok(())
}

pub fn verify(event: &wire::SignedEvent, expected_scope: &NetworkScope) -> Result<VerifiedEvent> {
    validate_envelope(event)?;
    let payload: PublicEvent = canonical::decode(&event.canonical_payload, MAX_OBJECT)?;
    payload.validate()?;
    payload.scope.ensure_matches(expected_scope)?;
    verify_eoa(
        typed_hash(&payload.scope, &event.canonical_payload, EVENT_TYPE),
        &event.signature,
        payload.actor_owner,
    )?;
    Ok(VerifiedEvent {
        payload,
        event_id: event_id(&event.canonical_payload),
    })
}

fn event_id(payload: &[u8]) -> ContentId {
    let mut bytes = Vec::with_capacity(ID_DOMAIN.len() + payload.len());
    bytes.extend_from_slice(ID_DOMAIN);
    bytes.extend_from_slice(payload);
    ContentId::of(&bytes)
}

pub(crate) fn typed_hash(scope: &NetworkScope, payload: &[u8], type_signature: &str) -> [u8; 32] {
    let mut domain = Vec::with_capacity(160);
    domain.extend_from_slice(&Keccak256::digest(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    ));
    domain.extend_from_slice(&Keccak256::digest(DOMAIN_NAME.as_bytes()));
    domain.extend_from_slice(&Keccak256::digest(DOMAIN_VERSION.as_bytes()));
    domain.extend_from_slice(&[0; 24]);
    domain.extend_from_slice(&scope.chain_id.to_be_bytes());
    domain.extend_from_slice(&[0; 12]);
    domain.extend_from_slice(&scope.registry_contract.0);
    let mut event = Vec::with_capacity(64);
    event.extend_from_slice(&Keccak256::digest(type_signature.as_bytes()));
    event.extend_from_slice(&Keccak256::digest(payload));
    let mut typed = Vec::with_capacity(66);
    typed.extend_from_slice(&[0x19, 0x01]);
    typed.extend_from_slice(&Keccak256::digest(domain));
    typed.extend_from_slice(&Keccak256::digest(event));
    Keccak256::digest(typed).into()
}

pub(crate) fn verify_eoa(digest: [u8; 32], signature_bytes: &[u8], owner: Address) -> Result<()> {
    if signature_bytes.len() != 65 {
        return Err(Error::InvalidSignature);
    }
    let signature =
        Signature::from_slice(&signature_bytes[..64]).map_err(|_| Error::InvalidSignature)?;
    if signature.normalize_s().is_some() {
        return Err(Error::InvalidSignature);
    }
    let recovery = match signature_bytes[64] {
        0 | 27 => 0,
        1 | 28 => 1,
        _ => return Err(Error::InvalidSignature),
    };
    let recovery = RecoveryId::try_from(recovery).map_err(|_| Error::InvalidSignature)?;
    let public = VerifyingKey::recover_from_prehash(&digest, &signature, recovery)
        .map_err(|_| Error::InvalidSignature)?;
    let point = public.to_encoded_point(false);
    let address = Keccak256::digest(&point.as_bytes()[1..]);
    if address[12..] != owner.0 {
        return Err(Error::InvalidSignature);
    }
    Ok(())
}
