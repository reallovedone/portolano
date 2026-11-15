//! Shared contracts of Portolano: the connection document, the application error and the
//! small types that cross the IPC boundary and are used by more than one crate.
//!
//! # JSON convention (applies to every type that crosses a boundary)
//!
//! - Struct fields are `camelCase`.
//! - Enums that carry data are internally tagged with a `"type"` property whose value is the
//!   `camelCase` variant name, and their fields sit next to it
//!   (`{"type":"ssh","jumpHost":null,"keepaliveSecs":30}`). Newtype variants wrap a struct so
//!   that this works (`Node::Folder(Folder)` is `{"type":"folder","id":...}`).
//! - Enums without data are plain `camelCase` strings (`"notCreated"`).
//! - `Option` fields are always present and `null` when empty; on input a missing optional
//!   field is accepted as `None`.
//! - UUIDs are lowercase hyphenated strings. Error codes are `snake_case` strings.
//! - Bytes and pixels never travel as JSON: they use raw binary bodies (see `fb` and the
//!   session crates for the frame layouts).

pub mod credential;
pub mod error;
pub mod fb;
pub mod model;
pub mod paths;
pub mod private_fs;
pub mod prompt;
pub mod store;
pub mod web;

pub use credential::resolve_credential;
pub use error::{AppError, ErrorCode, ModelError};
pub use model::{
    host_is_storable, Connection, ConnectionOptions, CredentialRef, Document, Folder, Node,
    SCHEMA_VERSION,
};
pub use paths::{
    connections_path, data_dir, known_hosts_path, resolve_data_dir, vault_path, DATA_DIR_ENV,
};
pub use store::DocumentStore;
