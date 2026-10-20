use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Result<T> = std::result::Result<T, Error>;

/// Errors deliberately contain no untrusted payloads, tokens or key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("Unsupported protocol version.")]
    UnsupportedVersion,
    #[error("Unsupported protocol capability.")]
    UnsupportedCapability,
    #[error("Network scope does not match.")]
    NetworkMismatch,
    #[error("Invalid protocol message.")]
    InvalidMessage,
    #[error("Invalid signature.")]
    InvalidSignature,
    #[error("Protocol resource limit exceeded.")]
    ResourceExhausted,
    #[error("Incomplete protocol frame.")]
    Incomplete,
    #[error("Non-canonical encoding.")]
    NonCanonical,
    #[error("Expired protocol record.")]
    Expired,
    #[error("Replay or record revision rollback detected.")]
    Replay,
    #[error("Conflicting record revision.")]
    Conflict,
    #[error("Content integrity check failed.")]
    IntegrityFailed,
    #[error("User authorization is required.")]
    AuthRequired,
}

macro_rules! fixed_bytes {
    ($name:ident, $size:expr) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(#[serde(with = "serde_bytes")] pub [u8; $size]);

        impl $name {
            pub fn from_slice(bytes: &[u8]) -> Result<Self> {
                Ok(Self(bytes.try_into().map_err(|_| Error::InvalidMessage)?))
            }
            pub fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }
        }
    };
}

fixed_bytes!(Address, 20);
fixed_bytes!(Nonce, 32);

/// A SHA-256 multihash: 0x12 (sha2-256), 0x20 (32 bytes), digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentId(#[serde(with = "serde_bytes")] [u8; 34]);

impl ContentId {
    pub fn of(bytes: &[u8]) -> Self {
        let mut id = [0; 34];
        id[..2].copy_from_slice(&[0x12, 0x20]);
        id[2..].copy_from_slice(&Sha256::digest(bytes));
        Self(id)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 34 || bytes[..2] != [0x12, 0x20] {
            return Err(Error::InvalidMessage);
        }
        Ok(Self(bytes.try_into().map_err(|_| Error::InvalidMessage)?))
    }

    pub fn as_bytes(&self) -> &[u8; 34] {
        &self.0
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        if *self == Self::of(bytes) {
            Ok(())
        } else {
            Err(Error::IntegrityFailed)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkScope {
    pub protocol_family: String,
    pub network_version: u32,
    pub chain_id: u64,
    pub registry_contract: Address,
}

impl NetworkScope {
    pub fn new(chain_id: u64, registry_contract: Address) -> Result<Self> {
        let scope = Self {
            protocol_family: "red-leaves".into(),
            network_version: 1,
            chain_id,
            registry_contract,
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn validate(&self) -> Result<()> {
        if self.protocol_family != "red-leaves" || self.network_version != 1 {
            return Err(Error::UnsupportedVersion);
        }
        if self.chain_id == 0 || self.registry_contract.0 == [0; 20] {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }

    pub fn ensure_matches(&self, expected: &Self) -> Result<()> {
        self.validate()?;
        expected.validate()?;
        if self != expected {
            return Err(Error::NetworkMismatch);
        }
        Ok(())
    }
}
