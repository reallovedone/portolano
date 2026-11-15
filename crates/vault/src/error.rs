use portolano_core_model::{AppError, ErrorCode};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("the vault is locked")]
    Locked,
    #[error("the vault does not exist yet")]
    NotCreated,
    #[error("the vault already exists")]
    AlreadyExists,
    /// Wrong master password or tampered file (they are indistinguishable by design).
    #[error("authentication failed")]
    AuthFailed,
    /// The file is structurally invalid (bad magic, truncated, out-of-range parameters).
    /// Reported with the `auth_failed` code: a damaged vault cannot be authenticated.
    #[error("vault file is damaged or is not a vault: {0}")]
    Corrupt(&'static str),
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u16),
    #[error("credential not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("vault internal error: {0}")]
    Internal(String),
}

impl VaultError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Locked | Self::NotCreated => ErrorCode::VaultLocked,
            Self::AlreadyExists | Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::AuthFailed | Self::Corrupt(_) => ErrorCode::AuthFailed,
            Self::UnsupportedVersion(_) => ErrorCode::Unsupported,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::Io(_) => ErrorCode::Io,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }
}

impl From<VaultError> for AppError {
    fn from(e: VaultError) -> Self {
        AppError::new(e.code(), e.to_string())
    }
}
