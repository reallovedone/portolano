//! Errors of an RDP session and their mapping to the stable IPC error codes.
//!
//! Messages never contain credentials: connector errors are summarised, not echoed blindly
//! when they come from the authentication layer.

use ironrdp_connector::sspi::{Error as SspiError, ErrorKind as SspiErrorKind};
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
    /// Kerberos was used and the domain controller (KDC) could not be reached or did not answer
    /// in time. The connection may be retried with NTLM.
    #[error("Kerberos: the domain controller could not be reached")]
    KdcUnreachable,
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Io(String),
    /// The connection dropped while the session was active (timeout, reset, closed by the
    /// server). The raw OS text is only logged; the user sees a fixed message.
    #[error("The connection to the server was lost.")]
    ConnectionLost,
    /// The server ended the stream without a Terminate PDU (unexpected end of file).
    #[error("The server closed the connection.")]
    ServerClosed,
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
    /// Maps a network I/O failure of the active session. The raw OS text goes only to the log;
    /// the user sees a fixed message. A bare end of stream is reported as the server closing
    /// the connection, everything else (timeout, reset, broken pipe) as a lost connection.
    pub(crate) fn from_io(e: &std::io::Error) -> Self {
        tracing::warn!(kind = ?e.kind(), "RDP connection lost: {e}");
        match e.kind() {
            std::io::ErrorKind::UnexpectedEof => Self::ServerClosed,
            _ => Self::ConnectionLost,
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::HostUnreachable(_) | Self::KdcUnreachable => ErrorCode::HostUnreachable,
            Self::AuthFailed(_) => ErrorCode::AuthFailed,
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::Io(_)
            | Self::ConnectionLost
            | Self::ServerClosed
            | Self::Cancelled
            | Self::CertificateChanged => ErrorCode::Io,
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
        // The CredSSP error text comes from the authentication stack: do not echo it. Only the
        // fixed error kind decides the message.
        ConnectorErrorKind::Credssp(c) => credssp_error(c.error_type),
        ConnectorErrorKind::AccessDenied => generic_auth_failure(),
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
        // The network client reports an unreachable KDC (or a timeout) as a custom error that
        // carries sspi's own error kind.
        _ if sspi_kind_in_chain(e) == Some(SspiErrorKind::NoAuthenticatingAuthority) => {
            RdpError::KdcUnreachable
        }
        // The server picked a protocol that was not offered (TLS-only after a CredSSP-only offer).
        _ if nla.nla_only && e.to_string().contains("server selected") => {
            RdpError::Unsupported(NO_NLA_MESSAGE.to_owned())
        }
        _ => RdpError::Io(format!("RDP connection failed: {e}")),
    }
}

fn generic_auth_failure() -> RdpError {
    RdpError::AuthFailed("authentication failed (user name, password or domain)".into())
}

/// Maps the kind of a CredSSP (sspi) error. Messages are fixed texts: never the description of
/// the error, which may quote names or tokens.
fn credssp_error(kind: SspiErrorKind) -> RdpError {
    match kind {
        SspiErrorKind::NoAuthenticatingAuthority => RdpError::KdcUnreachable,
        SspiErrorKind::TimeSkew => {
            RdpError::Io("Kerberos: the clock differs too much from the domain controller".into())
        }
        SspiErrorKind::UnknownCredentials => RdpError::AuthFailed(
            "Kerberos: user or server not found in the domain (use the server's full name)".into(),
        ),
        // Failed pre-authentication (Kerberos) and refused logon (NTLM).
        SspiErrorKind::LogonDenied | SspiErrorKind::KdcInvalidRequest => {
            RdpError::AuthFailed("wrong user name or password".into())
        }
        _ => generic_auth_failure(),
    }
}

/// The first I/O error found in the cause chain of an error, if any.
pub(crate) fn io_in_chain<'a>(
    e: &'a (dyn std::error::Error + 'static),
) -> Option<&'a std::io::Error> {
    let mut current: Option<&'a (dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = current {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            return Some(io);
        }
        current = err.source();
    }
    None
}

/// The kind of the first sspi error found in the cause chain of a connector error.
pub(crate) fn sspi_kind_in_chain(e: &ConnectorError) -> Option<SspiErrorKind> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = current {
        if let Some(sspi) = err.downcast_ref::<SspiError>() {
            return Some(sspi.error_type);
        }
        current = err.source();
    }
    None
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

    #[test]
    fn network_failures_map_to_fixed_messages() {
        use std::io::{Error, ErrorKind};
        // 60 is ETIMEDOUT on macOS; the OS text must not reach the user.
        for e in [
            Error::from_raw_os_error(60),
            Error::new(ErrorKind::TimedOut, "operation timed out"),
            Error::new(ErrorKind::ConnectionReset, "reset"),
            Error::new(ErrorKind::BrokenPipe, "pipe"),
        ] {
            let err = RdpError::from_io(&e);
            assert!(matches!(err, RdpError::ConnectionLost));
            assert_eq!(err.code(), ErrorCode::Io);
            let app: AppError = err.into();
            assert_eq!(app.message, "The connection to the server was lost.");
        }
        let eof = RdpError::from_io(&Error::new(ErrorKind::UnexpectedEof, "eof"));
        assert_eq!(eof.to_string(), "The server closed the connection.");
        assert_eq!(eof.code(), ErrorCode::Io);
    }

    #[test]
    fn io_errors_are_found_in_a_cause_chain() {
        use std::io::{Error, ErrorKind};
        let inner = Error::new(ErrorKind::TimedOut, "t");
        assert_eq!(
            io_in_chain(&inner).map(Error::kind),
            Some(ErrorKind::TimedOut)
        );
        let other = ConnectorError::new("x", ConnectorErrorKind::General);
        assert!(io_in_chain(&other).is_none());
    }

    fn credssp(kind: SspiErrorKind) -> ConnectorError {
        // The description is what must never reach the user.
        ConnectorError::new(
            "CredSSP",
            ConnectorErrorKind::Credssp(SspiError::new(kind, "ann@corp.example.com tgt=SECRET")),
        )
    }

    fn message(kind: SspiErrorKind) -> String {
        from_connector(&credssp(kind), NlaPolicy::of(true, false)).to_string()
    }

    #[test]
    fn credssp_errors_map_by_kind_to_distinct_messages() {
        assert_eq!(
            message(SspiErrorKind::NoAuthenticatingAuthority),
            "Kerberos: the domain controller could not be reached"
        );
        assert_eq!(
            message(SspiErrorKind::TimeSkew),
            "Kerberos: the clock differs too much from the domain controller"
        );
        assert_eq!(
            message(SspiErrorKind::UnknownCredentials),
            "Kerberos: user or server not found in the domain (use the server's full name)"
        );
        assert_eq!(
            message(SspiErrorKind::LogonDenied),
            "wrong user name or password"
        );
        assert_eq!(
            message(SspiErrorKind::KdcInvalidRequest),
            "wrong user name or password"
        );
        assert_eq!(
            message(SspiErrorKind::InternalError),
            "authentication failed (user name, password or domain)"
        );
    }

    #[test]
    fn credssp_error_codes_let_the_session_decide_what_to_do() {
        let nla = NlaPolicy::of(true, false);
        let unreachable = from_connector(&credssp(SspiErrorKind::NoAuthenticatingAuthority), nla);
        assert!(matches!(unreachable, RdpError::KdcUnreachable));
        assert_eq!(unreachable.code(), ErrorCode::HostUnreachable);
        // A wrong password asks the user again; a clock problem does not.
        assert_eq!(
            from_connector(&credssp(SspiErrorKind::LogonDenied), nla).code(),
            ErrorCode::AuthFailed
        );
        assert_eq!(
            from_connector(&credssp(SspiErrorKind::TimeSkew), nla).code(),
            ErrorCode::Io
        );
    }

    #[test]
    fn no_credssp_message_echoes_the_error_text() {
        for kind in [
            SspiErrorKind::NoAuthenticatingAuthority,
            SspiErrorKind::TimeSkew,
            SspiErrorKind::UnknownCredentials,
            SspiErrorKind::LogonDenied,
            SspiErrorKind::KdcInvalidRequest,
            SspiErrorKind::InternalError,
            SspiErrorKind::Unknown,
        ] {
            let text = message(kind);
            assert!(!text.contains("SECRET") && !text.contains("ann@"), "{text}");
        }
    }

    #[test]
    fn a_network_client_failure_is_an_unreachable_kdc() {
        let e = crate::kerberos::unreachable_error("connection refused");
        assert!(matches!(
            from_connector(&e, NlaPolicy::of(true, false)),
            RdpError::KdcUnreachable
        ));
    }

    #[test]
    fn an_unrelated_custom_error_keeps_the_generic_text() {
        let e = ConnectorError::new("x", ConnectorErrorKind::General);
        assert!(matches!(
            from_connector(&e, NlaPolicy::of(true, false)),
            RdpError::Io(_)
        ));
    }
}
