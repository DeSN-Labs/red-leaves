use crate::network::transport::Handle;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::{Notify, broadcast};

pub struct Runtime {
    pub network: Mutex<Option<Handle>>,
    pub changes: broadcast::Sender<String>,
    pub wake: Notify,
    pub watchers: Mutex<HashMap<String, usize>>,
    pub stopping: std::sync::atomic::AtomicBool,
    pub shutdown: Notify,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            network: Mutex::new(None),
            changes: broadcast::channel(256).0,
            wake: Notify::new(),
            watchers: Mutex::new(HashMap::new()),
            stopping: std::sync::atomic::AtomicBool::new(false),
            shutdown: Notify::new(),
        }
    }
}
impl Runtime {
    pub fn changed(&self, owner: &str) {
        let _ = self.changes.send(owner.into());
    }
    pub fn network(&self) -> Option<Handle> {
        self.network.lock().ok()?.clone()
    }
}
