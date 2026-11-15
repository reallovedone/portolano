//! VNC framebuffer sessions (RFB 3.3/3.7/3.8), implemented in this crate.
//!
//! The event type is the shared framebuffer contract from `portolano-core-model`
//! (`FbEvent` and its binary wire format). See `session.rs` for the design and `rfb.rs` for
//! the protocol subset. Plain VNC is not encrypted: the session carries everything except the
//! password exchange in the clear. VeNCrypt with an X509 certificate (TLS) is encrypted.

mod auth;
pub mod cert;
pub mod dirty;
mod error;
mod input;
mod keymap;
pub mod rfb;
mod session;
mod tls;

pub use error::VncError;
pub use portolano_core_model::fb::FbEvent;
pub use portolano_core_model::prompt;
pub use session::{
    EventSink, Snapshot, VncAsk, VncConfig, VncSession, FLUSH_INTERVAL, SAVED_USERNAME,
};
