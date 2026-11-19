//! RDP framebuffer sessions (IronRDP).
//!
//! The event type is the shared framebuffer contract from `portolano-core-model`
//! (`FbEvent` and its binary wire format). See `session.rs` for the design.

pub mod cert;
pub mod clipboard;
pub mod dirty;
mod error;
mod keepalive;
pub mod kerberos;
pub mod keymap;
mod session;
pub mod tls;

pub use error::RdpError;
pub use portolano_core_model::fb::FbEvent;
pub use portolano_core_model::prompt;
pub use session::{
    EventSink, RdpAsk, RdpConfig, RdpCredentials, RdpSession, Snapshot, FLUSH_INTERVAL,
};
