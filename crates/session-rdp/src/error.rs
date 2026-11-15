//! Errors of an RDP session and their mapping to the stable IPC error codes.
//!
//! Messages never contain credentials: connector errors are summarised, not echoed blindly
//! when they come from the authentication layer.

use ironrdp_connector::{ConnectorError, ConnectorErrorKind};
use ironrdp_pdu::nego::FailureCode;
use portolano_core_model::{AppError, ErrorCode};

/// Shown when the server does not offer NLA and the connection did not allow going without it.
pub const NO_NLA_MESSAGE: &str = "the server does not offer Network Level Authentication (NLA), so your password was not sent. If you trust this server and network, enable \"Allow connections without NLA (less secure)\" in the connection's RDP options";

#[derive(Debug, thiserror::Error)]
pub enum RdpError {
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
    /// The session was closed from our side while a step was running. Not an error for the user.
    #[error("session closed")]
    Cancelled,
    /// The server certificate differs from the stored one. Reported to the user as a
    /// `CertPrompt` with `changed`, not as an error.
    #[error("the server certificate changed")]
    CertificateChanged,
    /// The user dismissed the credential prompt (reported as the `cancelled` code).
    #[error("No credential was provided.")]
    PromptCancelled,
}

impl RdpError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::HostUnreachable(_) => ErrorCode::HostUnreachable,
            Self::AuthFailed(_) => ErrorCode::AuthFailed,
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::Io(_) | Self::Cancelled | Self::CertificateChanged => ErrorCode::Io,
            Self::PromptCancelled => ErrorCode::Cancelled,
        }
    }
}

impl From<RdpError> for AppError {
    fn from(e: RdpError) -> Self {
        AppError::new(e.code(), e.to_string())
    }
}

/// What the connection offered during the security negotiation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NlaPolicy {
    /// A credential is sent, so NLA was attempted.
    pub has_credentials: bool,
    /// Only NLA was offered (a credential and no "allow without NLA").
    pub nla_only: bool,
}

impl NlaPolicy {
    pub fn of(has_credentials: bool, allow_without_nla: bool) -> Self {
        Self {
            has_credentials,
            nla_only: has_credentials && !allow_without_nla,
        }
    }
}

/// Maps a failure of the connection sequence.
pub(crate) fn from_connector(e: &ConnectorError, nla: NlaPolicy) -> RdpError {
    let has_credentials = nla.has_credentials;
    match e.kind() {
        // The CredSSP error text comes from the authentication stack: do not echo it.
        ConnectorErrorKind::Credssp(_) | ConnectorErrorKind::AccessDenied => {
            RdpError::AuthFailed("authentication failed (user name, password or domain)".into())
        }
        ConnectorErrorKind::EntraAuth(_) => {
            RdpError::Unsupported("Microsoft Entra ID authentication is not supported".into())
        }
        ConnectorErrorKind::Negotiation(failure) => {
            let code = failure.code();
            if nla.nla_only && code != FailureCode::HYBRID_REQUIRED_BY_SERVER {
                RdpError::Unsupported(format!("{NO_NLA_MESSAGE} ({failure})"))
            } else if code == FailureCode::HYBRID_REQUIRED_BY_SERVER && !has_credentials {
                RdpError::AuthFailed(
                    "the server requires Network Level Authentication: a user name and password are needed"
                        .into(),
                )
            } else if code == FailureCode::HYBRID_REQUIRED_BY_SERVER {
                RdpError::Unsupported(
                    "the server requires CredSSP, which could not be negotiated".into(),
                )
            } else {
                RdpError::Unsupported(format!("security negotiation failed: {failure}"))
            }
        }
        // The server picked a protocol that was not offered (TLS-only after a CredSSP-only offer).
        _ if nla.nla_only && e.to_string().contains("server selected") => {
            RdpError::Unsupported(NO_NLA_MESSAGE.to_owned())
        }
        _ => RdpError::Io(format!("RDP connection failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_stable_ones() {
        assert_eq!(
            RdpError::HostUnreachable("x".into()).code(),
            ErrorCode::HostUnreachable
        );
        assert_eq!(
            RdpError::AuthFailed("x".into()).code(),
            ErrorCode::AuthFailed
        );
        assert_eq!(
            RdpError::Unsupported("x".into()).code(),
            ErrorCode::Unsupported
        );
        assert_eq!(
            RdpError::InvalidInput("x".into()).code(),
            ErrorCode::InvalidInput
        );
        assert_eq!(RdpError::Io("x".into()).code(), ErrorCode::Io);
        let app: AppError = RdpError::AuthFailed("denied".into()).into();
        assert_eq!(app.message, "denied");
    }
}
