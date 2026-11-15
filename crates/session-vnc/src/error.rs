//! Errors of a VNC session and their mapping to the stable IPC error codes.
//!
//! Messages never contain the password: nothing in this crate formats a secret.

use portolano_core_model::{AppError, ErrorCode};

#[derive(Debug, thiserror::Error)]
pub enum VncError {
    #[error("{0}")]
    HostUnreachable(String),
    #[error("{0}")]
    AuthFailed(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Io(String),
    /// The server ended the connection cleanly at a message boundary (reported as `Closed`).
    #[error("{0}")]
    ServerClosed(String),
    /// The server wants VNC authentication and no password is known yet. Internal: the
    /// session turns it into a credential prompt (or `auth_failed` when it cannot ask).
    #[error("a password is required")]
    PasswordRequired,
    /// VeNCrypt Plain needs a user name and a password and the connection has not both.
    /// Internal, like `PasswordRequired`.
    #[error("a user name and a password are required")]
    PlainCredentialsRequired,
    /// The server was asked for VeNCrypt and offered no subtype this client supports. Internal:
    /// the session reconnects and uses another security type (or fails with `unsupported`).
    #[error("the server offers no supported VeNCrypt subtype")]
    VeNCryptUnusable { anonymous: bool },
    /// The user did not accept the server certificate (reported as a clean close).
    #[error("the server certificate was not accepted")]
    CertDeclined,
    /// The server certificate differs from the stored one. Reported to the user as a
    /// `CertPrompt` with `changed`, not as an error.
    #[error("the server certificate changed")]
    CertificateChanged,
    /// The host used TLS before (a certificate is pinned) and now offers none: refused.
    #[error("{0}")]
    SecurityDowngrade(String),
    /// The user did not agree to send the password without encryption (reported as a close).
    #[error("the connection is not encrypted and was not allowed")]
    UnencryptedDeclined,
    /// The session was closed from our side while a step was running. Not an error for the user.
    #[error("session closed")]
    Cancelled,
    /// The user dismissed the credential prompt (reported as the `cancelled` code).
    #[error("No credential was provided.")]
    PromptCancelled,
}

impl VncError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::HostUnreachable(_) => ErrorCode::HostUnreachable,
            Self::AuthFailed(_) | Self::PasswordRequired | Self::PlainCredentialsRequired => {
                ErrorCode::AuthFailed
            }
            Self::Unsupported(_) | Self::VeNCryptUnusable { .. } => ErrorCode::Unsupported,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::Io(_)
            | Self::ServerClosed(_)
            | Self::Cancelled
            | Self::CertDeclined
            | Self::CertificateChanged
            | Self::UnencryptedDeclined => ErrorCode::Io,
            Self::SecurityDowngrade(_) => ErrorCode::SecurityDowngrade,
            Self::PromptCancelled => ErrorCode::Cancelled,
        }
    }
}

impl From<VncError> for AppError {
    fn from(e: VncError) -> Self {
        AppError::new(e.code(), e.to_string())
    }
}

/// Maps an I/O error of the stream. End of stream is "the server closed the connection".
pub(crate) fn io_error(e: &std::io::Error) -> VncError {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset => {
            VncError::Io("the server closed the connection".into())
        }
        _ => VncError::Io(format!("network error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_stable_ones() {
        let cases = [
            (
                VncError::HostUnreachable("x".into()),
                ErrorCode::HostUnreachable,
            ),
            (VncError::AuthFailed("x".into()), ErrorCode::AuthFailed),
            (VncError::PasswordRequired, ErrorCode::AuthFailed),
            (VncError::Unsupported("x".into()), ErrorCode::Unsupported),
            (VncError::InvalidInput("x".into()), ErrorCode::InvalidInput),
            (VncError::Io("x".into()), ErrorCode::Io),
            (VncError::ServerClosed("x".into()), ErrorCode::Io),
            (VncError::PromptCancelled, ErrorCode::Cancelled),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
        let app: AppError = VncError::AuthFailed("denied".into()).into();
        assert_eq!(app.message, "denied");
    }
}
