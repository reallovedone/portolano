//! The error type shared across the IPC boundary.
//!
//! `AppError` lives here (not in `src-tauri`) because session crates embed it in their
//! channel events (`TermEvent::Error`, `FbEvent::Error`). `src-tauri/src/error.rs` re-exports it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable error codes. The set is part of the IPC contract: do not rename or remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    VaultLocked,
    AuthFailed,
    HostUnreachable,
    HostKeyMismatch,
    NotFound,
    InvalidInput,
    Io,
    Unsupported,
    Internal,
    /// The user cancelled a credential prompt (WP11). Additive: not an error to report.
    Cancelled,
    /// A server stopped offering the protection it offered before (a stored TLS certificate
    /// exists, the server now offers none). Additive: the connection is refused.
    SecurityDowngrade,
    /// The user dismissed the Touch ID / Windows Hello request of quick unlock. Additive.
    QuickUnlockCancelled,
    /// The biometric check failed or quick unlock could not open the vault. Additive: the UI
    /// falls back to the master password.
    QuickUnlockFailed,
    /// Quick unlock has no key held (never unlocked with the password in this run, turned off,
    /// or the password changed). Additive.
    QuickUnlockNotReady,
}

impl ErrorCode {
    /// The wire representation of the code.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VaultLocked => "vault_locked",
            Self::AuthFailed => "auth_failed",
            Self::HostUnreachable => "host_unreachable",
            Self::HostKeyMismatch => "host_key_mismatch",
            Self::NotFound => "not_found",
            Self::InvalidInput => "invalid_input",
            Self::Io => "io",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
            Self::Cancelled => "cancelled",
            Self::SecurityDowngrade => "security_downgrade",
            Self::QuickUnlockCancelled => "quick_unlock_cancelled",
            Self::QuickUnlockFailed => "quick_unlock_failed",
            Self::QuickUnlockNotReady => "quick_unlock_not_ready",
        }
    }
}

/// `{ code, message }` as seen by the UI. Messages must never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Error returned by every stubbed entry point of the scaffold.
    pub fn unsupported(what: &str) -> Self {
        Self::new(
            ErrorCode::Unsupported,
            format!("{what} is not implemented yet"),
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AppError {}

/// Errors of the document model and store.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("{0} is not implemented yet")]
    Unsupported(&'static str),
    #[error("node not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot determine the data directory")]
    NoDataDir,
    /// The file is not valid JSON, is truncated, or fails the document invariants and no valid
    /// backup could replace it. The file is left untouched.
    #[error("the connections file is corrupt: {0}")]
    Corrupt(String),
    /// The file was written by a newer version of the application. It is left untouched.
    #[error(
        "the connections file uses schema version {found}, this version supports up to {supported}"
    )]
    NewerSchema { found: u32, supported: u32 },
}

impl ModelError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::Io(_) => ErrorCode::Io,
            Self::NoDataDir => ErrorCode::Internal,
            Self::Corrupt(_) => ErrorCode::Io,
            Self::NewerSchema { .. } => ErrorCode::Unsupported,
        }
    }
}

impl From<ModelError> for AppError {
    fn from(e: ModelError) -> Self {
        AppError::new(e.code(), e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_error_json_shape() {
        let json = serde_json::to_value(AppError::new(ErrorCode::VaultLocked, "locked"))
            .expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({"code": "vault_locked", "message": "locked"})
        );
    }

    #[test]
    fn every_code_serializes_to_as_str() {
        for code in [
            ErrorCode::VaultLocked,
            ErrorCode::AuthFailed,
            ErrorCode::HostUnreachable,
            ErrorCode::HostKeyMismatch,
            ErrorCode::NotFound,
            ErrorCode::InvalidInput,
            ErrorCode::Io,
            ErrorCode::Unsupported,
            ErrorCode::Internal,
            ErrorCode::Cancelled,
            ErrorCode::SecurityDowngrade,
            ErrorCode::QuickUnlockCancelled,
            ErrorCode::QuickUnlockFailed,
            ErrorCode::QuickUnlockNotReady,
        ] {
            let json = serde_json::to_value(code).expect("serialize");
            assert_eq!(json, serde_json::Value::String(code.as_str().to_owned()));
        }
    }

    #[test]
    fn model_error_maps_to_code() {
        let err: AppError = ModelError::NotFound("x".into()).into();
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}
