//! Shared folders (phase A): who signs a share, the signed file format, the rules for accepting
//! a new version, sanitising of remote content and the merged tree a subscriber sees.
//!
//! Everything here is pure (no I/O, no clock): the service that reads and writes files lives in
//! the application crate. Private keys never appear in errors, logs or `Debug` output.

pub mod accept;
pub mod error;
pub mod format;
pub mod identity;
pub mod sanitize;
pub mod view;

pub use error::ShareError;
