//! Errors of the share format. Messages never contain key material or file content.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    #[error("the share file is larger than the 5 MiB limit")]
    TooLarge,
    #[error("this is not a Portolano share file")]
    NotAShare,
    #[error("unsupported share file version {0}")]
    UnsupportedVersion(u64),
    #[error("the share file is damaged: {0}")]
    Malformed(String),
    #[error("the signature of the share file is not valid")]
    BadSignature,
    #[error("the shared folder cannot be used: {0}")]
    InvalidContent(String),
    #[error("the shared folder is too large or too deeply nested (at most {max_nodes} items and {max_depth} folder levels)")]
    TooComplex { max_nodes: usize, max_depth: usize },
    #[error("this is not a valid Portolano identity code")]
    InvalidPublicCode,
    #[error("the system could not provide random data")]
    NoRandomSource,
}
