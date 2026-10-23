//! Bounded application delivery v2. It does not replace the lower-level wire protocol.
use crate::{
    Address, ContentId, Error, NetworkScope, Result,
    encryption::{KeyEnvelope, Sealed},
    social::Distribution,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL: &str = "/red-leaves/delivery/2.0.0";
pub const CHUNK: usize = 256 * 1024;
pub const MAX_MESSAGE: usize = 384 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub scope: NetworkScope,
    pub id: ContentId,
    pub author: Address,
    pub source_peer: String,
    pub distribution: Distribution,
    pub created_at: u64,
    /// Only a public broadcast may advertise its title.
    pub title: String,
}
impl Notice {
    pub fn validate(&self, scope: &NetworkScope, now: u64) -> Result<()> {
        self.scope.ensure_matches(scope)?;
        if self.author.0 == [0; 20]
            || self.source_peer.len() > 128
            || self.source_peer.parse::<libp2p_identity::PeerId>().is_err()
            || self.created_at == 0
            || self.created_at > now.saturating_add(120)
            || self.title.len() > 256
            || (self.distribution != Distribution::Broadcast && !self.title.is_empty())
        {
            return Err(Error::InvalidMessage);
        }
        ContentId::from_slice(self.id.as_bytes())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Offer {
        notice: Notice,
        recipient: Address,
    },
    Fetch {
        id: ContentId,
        viewer: Address,
    },
    Follow {
        #[serde(with = "serde_bytes")]
        payload: Vec<u8>,
        #[serde(with = "serde_bytes")]
        signature: Vec<u8>,
    },
    MediaStart {
        id: ContentId,
        viewer: Address,
        media_id: String,
    },
    MediaChunk {
        token: String,
        offset: u64,
    },
    /// A bounded page of recent public notices for reconnect catch-up.
    Sync {
        offset: u32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Saved {
        body: bool,
    },
    Ignored,
    Content {
        sealed: Sealed,
        envelope: KeyEnvelope,
        signature: String,
    },
    Media {
        token: String,
        envelope: KeyEnvelope,
        #[serde(with = "serde_bytes")]
        nonce: Vec<u8>,
        size: u64,
    },
    Chunk {
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Notices {
        notices: Vec<Notice>,
    },
    Error {
        code: String,
    },
}
