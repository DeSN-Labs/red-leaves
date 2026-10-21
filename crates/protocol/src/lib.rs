//! NAS communication protocol primitives. No sockets, database or private-key storage.
//!
//! Decoding establishes structural validity, never user authorization. Callers must
//! check current registry bindings, grants, revocations, policy and durable storage
//! before accepting a user context, serving content or acknowledging delivery.

pub mod canonical;
pub mod codec;
pub mod delivery;
pub mod encryption;
pub mod event;
pub mod file;
pub mod handshake;
pub mod receipt;
pub mod route;
pub mod social;
pub mod types;
pub mod wire;

pub use types::{Address, ContentId, Error, NetworkScope, Nonce, Result};

pub const CONTROL_PROTOCOL: &str = "/red-leaves/control/1.0.0";
pub const CONTENT_PROTOCOL: &str = "/red-leaves/content/1.0.0";
pub const WIRE_VERSION: u32 = 1;
pub const MAX_CONTROL_FRAME: usize = 64 * 1024;
pub const MAX_OBJECT: usize = 256 * 1024;
pub const MAX_OBJECT_FRAME: usize = 288 * 1024;
pub const MAX_CHUNK: usize = 256 * 1024;
pub const MAX_CHUNK_FRAME: usize = 272 * 1024;
pub const MAX_ROUTE: usize = 8 * 1024;
pub const MAX_SYNC_PAGE: usize = 100;
