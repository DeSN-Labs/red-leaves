pub mod keys;
pub mod registry;
pub mod runtime;
use crate::identity::Error;
use red_leaves_protocol::{Address, NetworkScope};
use std::{path::PathBuf, sync::Arc};

pub struct Service {
    pub runtime: runtime::Runtime,
    pub keys: keys::KeyStore,
    pub chain: Option<registry::Chain>,
    pub objects: PathBuf,
}
impl Service {
    pub fn from_env() -> Result<Arc<Self>, Error> {
        let root = PathBuf::from(
            std::env::var("RED_LEAVES_CONTENT_PATH")
                .unwrap_or_else(|_| "runtime/nas/content".into()),
        );
        let keys = keys::KeyStore::new(
            PathBuf::from(
                std::env::var("RED_LEAVES_USER_KEY_PATH")
                    .unwrap_or_else(|_| "runtime/nas/user-keys".into()),
            ),
            PathBuf::from(
                std::env::var("RED_LEAVES_MASTER_KEY_PATH")
                    .unwrap_or_else(|_| "runtime/nas/master.key".into()),
            ),
        );
        let address = std::env::var("RED_LEAVES_USER_REGISTRY_ADDRESS").unwrap_or_default();
        let chain = if address.is_empty() {
            None
        } else {
            Some(registry::Chain::new(
                std::env::var("RED_LEAVES_RPC_URL")
                    .unwrap_or_else(|_| "https://sepolia.base.org".into()),
                Address(crate::identity::address(&address)?),
            )?)
        };
        Ok(Arc::new(Self {
            runtime: runtime::Runtime::default(),
            keys,
            chain,
            objects: root,
        }))
    }
    pub fn scope(&self) -> Result<NetworkScope, Error> {
        Ok(NetworkScope::new(
            crate::identity::CHAIN_ID,
            self.chain
                .as_ref()
                .ok_or("Registry is not configured")?
                .registry,
        )?)
    }
}
