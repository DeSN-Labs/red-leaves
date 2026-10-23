//! Explicit, scoped user actions. A Publish action is both signature and publication confirmation.
use crate::{Address, ContentId, Error, NetworkScope, Nonce, Result, canonical, event};
use serde::{Deserialize, Serialize};

pub const MAX_TEXT: usize = 16 * 1024;
pub const SOCIAL_TYPE: &str = "SocialAction(bytes32 payloadHash)";
pub fn payload_hash(payload: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    Keccak256::digest(payload).into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    Broadcast,
    Following,
    Followers,
    Friends,
    Directed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub id: String,
    pub mime: String,
    pub size: u64,
    /// SHA-256 multihash of the original file, included in the publication signature.
    pub digest: String,
}
impl Media {
    pub fn validate(&self) -> Result<()> {
        let image = matches!(
            self.mime.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        );
        let video = matches!(self.mime.as_str(), "video/mp4" | "video/webm");
        if (!image && !video)
            || self.size == 0
            || self.size
                > if image {
                    10 * 1024 * 1024
                } else {
                    64 * 1024 * 1024
                }
            || self.id.len() != 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || self.digest.len() != 68
            || !self.digest.starts_with("1220")
            || !self
                .digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostDraft {
    pub title: String,
    pub body: String,
    pub distribution: Distribution,
    pub recipients: Vec<Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<Media>,
}
impl PostDraft {
    pub fn validate(&self, author: Address) -> Result<()> {
        if self.title.trim().is_empty()
            || self.title.len() > 256
            || (self.body.trim().is_empty() && self.media.is_empty())
            || self.body.len() > MAX_TEXT
            || self.recipients.len() > 32
        {
            return Err(Error::InvalidMessage);
        }
        if self.media.len() > 9 {
            return Err(Error::InvalidMessage);
        }
        let mut files = std::collections::HashSet::new();
        for media in &self.media {
            media.validate()?;
            if !files.insert(&media.id)
                || (media.mime.starts_with("video/") && self.media.len() != 1)
            {
                return Err(Error::InvalidMessage);
            }
        }
        if (self.distribution == Distribution::Broadcast) != self.recipients.is_empty() {
            return Err(Error::InvalidMessage);
        }
        let mut seen = std::collections::HashSet::new();
        for recipient in &self.recipients {
            if recipient.0 == [0; 20] || *recipient == author || !seen.insert(recipient) {
                return Err(Error::InvalidMessage);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Publish { post: PostDraft },
    Follow { target: Address, enabled: bool },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub scope: NetworkScope,
    pub actor: Address,
    pub nas_peer_id: String,
    pub nonce: Nonce,
    pub issued_at: u64,
    pub expires_at: u64,
    pub action: Action,
}
impl Intent {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.actor.0 == [0; 20]
            || self.nonce.0 == [0; 32]
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > 900
            || self.nas_peer_id.len() > 128
            || self.nas_peer_id.parse::<libp2p_identity::PeerId>().is_err()
        {
            return Err(Error::InvalidMessage);
        }
        match &self.action {
            Action::Publish { post } => post.validate(self.actor)?,
            Action::Follow { target, .. } if target.0 == [0; 20] || *target == self.actor => {
                return Err(Error::InvalidMessage);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical::encode(self, 32 * 1024)
    }
    pub fn signing_hash(&self) -> Result<[u8; 32]> {
        Ok(event::typed_hash(&self.scope, &self.encode()?, SOCIAL_TYPE))
    }
    pub fn verify_signature(&self, signature: &[u8]) -> Result<()> {
        self.validate()?;
        event::verify_eoa(self.signing_hash()?, signature, self.actor)
    }
    pub fn id(&self) -> Result<ContentId> {
        Ok(ContentId::of(
            &[b"red-leaves/social/1\0".as_slice(), &self.encode()?].concat(),
        ))
    }
    pub fn verify(
        bytes: &[u8],
        signature: &[u8],
        scope: &NetworkScope,
        nas: &str,
        now: u64,
    ) -> Result<Self> {
        let intent: Self = canonical::decode(bytes, 32 * 1024)?;
        intent.validate()?;
        intent.scope.ensure_matches(scope)?;
        if intent.nas_peer_id != nas {
            return Err(Error::NetworkMismatch);
        }
        if intent.issued_at > now.saturating_add(120) || intent.expires_at <= now {
            return Err(Error::Expired);
        }
        event::verify_eoa(intent.signing_hash()?, signature, intent.actor)?;
        Ok(intent)
    }
}

/// Verify a saved publication using the current schema without reapplying submission expiry.
pub fn verify_post(
    payload: &[u8],
    signature: &[u8],
    expected_id: ContentId,
    scope: &NetworkScope,
) -> Result<Intent> {
    let intent: Intent = canonical::decode(payload, 32 * 1024)?;
    intent.validate()?;
    intent.scope.ensure_matches(scope)?;
    if !matches!(intent.action, Action::Publish { .. }) || intent.id()? != expected_id {
        return Err(Error::InvalidMessage);
    }
    intent.verify_signature(signature)?;
    Ok(intent)
}
