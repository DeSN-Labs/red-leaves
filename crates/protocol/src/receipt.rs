//! Final-NAS signed storage claims. A signature is not a physical disk proof.

use libp2p_core::SignedEnvelope;
use libp2p_identity::{Keypair, PeerId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::{Address, ContentId, Error, MAX_ROUTE, NetworkScope, Nonce, Result, canonical};

pub const RECEIPT_DOMAIN: &str = "red-leaves/storage-receipt/1";
pub const RECEIPT_PAYLOAD_TYPE: &[u8] = b"/red-leaves/storage-receipt/1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageReceipt {
    pub scope: NetworkScope,
    pub recipient_owner: Address,
    #[serde(with = "serde_bytes")]
    pub recipient_nas_peer_id: Vec<u8>,
    pub event_id: ContentId,
    pub body_stored: bool,
    pub attachments_stored: bool,
    pub receipt_nonce: Nonce,
    pub committed_at: u64,
    pub service_grant_ref: ContentId,
}

impl StorageReceipt {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        crate::codec::identifier(&self.recipient_owner.0, 20)?;
        crate::codec::identifier(&self.receipt_nonce.0, 32)?;
        PeerId::from_bytes(&self.recipient_nas_peer_id).map_err(|_| Error::InvalidMessage)?;
        ContentId::from_slice(self.event_id.as_bytes())?;
        ContentId::from_slice(self.service_grant_ref.as_bytes())?;
        if !self.body_stored || self.committed_at == 0 {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }

    /// Call only after actual file/database durability, using the user's service key.
    pub fn sign(&self, service_key: &Keypair) -> Result<Vec<u8>> {
        self.validate()?;
        let payload = canonical::encode(self, MAX_ROUTE)?;
        let encoded = SignedEnvelope::new(
            service_key,
            RECEIPT_DOMAIN.into(),
            RECEIPT_PAYLOAD_TYPE.to_vec(),
            payload,
        )
        .map_err(|_| Error::InvalidSignature)?
        .into_protobuf_encoding();
        if encoded.len() > MAX_ROUTE {
            return Err(Error::ResourceExhausted);
        }
        Ok(encoded)
    }
}

/// `service_key` must come from an independently validated, current service grant.
/// This function never interprets a peer's self-declared key as user authorization.
pub fn verify(
    envelope: &[u8],
    scope: &NetworkScope,
    service_key: &PublicKey,
    recipient: Address,
    recipient_nas: PeerId,
    event_id: ContentId,
) -> Result<StorageReceipt> {
    if envelope.is_empty() || envelope.len() > MAX_ROUTE {
        return Err(Error::ResourceExhausted);
    }
    let signed =
        SignedEnvelope::from_protobuf_encoding(envelope).map_err(|_| Error::InvalidMessage)?;
    let (payload, signer) = signed
        .payload_and_signing_key(RECEIPT_DOMAIN.into(), RECEIPT_PAYLOAD_TYPE)
        .map_err(|_| Error::InvalidSignature)?;
    if signer != service_key {
        return Err(Error::InvalidSignature);
    }
    let receipt: StorageReceipt = canonical::decode(payload, MAX_ROUTE)?;
    receipt.validate()?;
    receipt.scope.ensure_matches(scope)?;
    if receipt.recipient_owner != recipient
        || receipt.recipient_nas_peer_id != recipient_nas.to_bytes()
        || receipt.event_id != event_id
    {
        return Err(Error::InvalidMessage);
    }
    Ok(receipt)
}
