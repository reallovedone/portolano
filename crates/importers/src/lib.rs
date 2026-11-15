//! Import and export of external connection formats (two steps: preview, then apply).
//!
//! [`preview`] parses a file without touching the document and returns the nodes that would be
//! created, with warnings. [`apply`] inserts them through a `DocumentStore`, all or nothing.
//! [`export`] writes Portolano JSON or one `.rdp` file.
//!
//! Passwords found in source files are never imported: they are dropped with a warning, and no
//! warning or error message contains their value. Usernames are kept.

mod apply;
mod csv_import;
mod draft;
mod export;
mod json;
mod mobaxterm;
mod mremoteng;
mod rdp;
mod ssh_config;
mod text;

use std::collections::VecDeque;
use std::path::Path;

use portolano_core_model::{AppError, ErrorCode, ModelError, Node};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use apply::apply;
pub use export::{export, select};

/// Source files larger than this are refused.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Imports with more entries than this are refused.
pub const MAX_ENTRIES: usize = 10_000;
/// Parsed previews kept in memory for `import_apply`; the oldest is evicted first.
pub const PREVIEW_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportFormat {
    SshConfig,
    Rdp,
    Mobaxterm,
    Mremoteng,
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Json,
    Rdp,
}

/// Something the user must know before applying an import (discarded password, unknown field...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportWarning {
    pub message: String,
    /// 1-based line in the source file, when known.
    pub line: Option<u32>,
}

/// Result of `import_preview`: the nodes that would be created (not yet in the document).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub preview_id: Uuid,
    pub nodes: Vec<Node>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl From<ImportError> for AppError {
    fn from(e: ImportError) -> Self {
        let code = match e {
            ImportError::Unsupported(_) => ErrorCode::Unsupported,
            ImportError::NotFound(_) => ErrorCode::NotFound,
            ImportError::Io(_) => ErrorCode::Io,
            ImportError::InvalidInput(_) => ErrorCode::InvalidInput,
        };
        AppError::new(code, e.to_string())
    }
}

impl From<ModelError> for ImportError {
    fn from(e: ModelError) -> Self {
        match e {
            ModelError::NotFound(s) => Self::NotFound(s),
            ModelError::InvalidInput(s) => Self::InvalidInput(s),
            ModelError::Unsupported(s) => Self::Unsupported(s.to_owned()),
            ModelError::Io(io) => Self::Io(io),
            other => Self::Io(std::io::Error::other(other.to_string())),
        }
    }
}

/// Parses `path` as `format` without touching the document. Refuses files over
/// [`MAX_FILE_BYTES`] and imports with more than [`MAX_ENTRIES`] entries.
pub fn preview(format: ImportFormat, path: &Path) -> Result<ImportPreview, ImportError> {
    let bytes = text::read_limited(path)?;
    let source = text::decode(&bytes)?;
    let name_hint = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_owned();
    let mut draft = match format {
        ImportFormat::SshConfig => ssh_config::parse(&source)?,
        ImportFormat::Rdp => rdp::parse(&source, &name_hint)?,
        ImportFormat::Mobaxterm => mobaxterm::parse(&source)?,
        ImportFormat::Mremoteng => mremoteng::parse(&source)?,
        ImportFormat::Csv => csv_import::parse(&source)?,
        ImportFormat::Json => json::parse(&source)?,
    };
    // Whatever the format, an imported file never brings credentials, relaxed certificate
    // checks or programs to run.
    draft.sanitize();
    if !draft.has_connections() {
        draft.warn(None, "no connections found in the file");
    }
    Ok(draft.into_preview())
}

/// Previews kept for `import_apply`: at most [`PREVIEW_CAPACITY`], the oldest evicted first.
#[derive(Debug, Default)]
pub struct PreviewCache {
    entries: VecDeque<ImportPreview>,
}

impl PreviewCache {
    pub fn insert(&mut self, preview: ImportPreview) {
        if self.entries.len() >= PREVIEW_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(preview);
    }

    /// A copy of the preview with `id`, if it is still cached.
    pub fn get(&self, id: Uuid) -> Option<ImportPreview> {
        self.entries.iter().find(|p| p.preview_id == id).cloned()
    }

    pub fn remove(&mut self, id: Uuid) {
        self.entries.retain(|p| p.preview_id != id);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn format_json_shapes() {
        assert_eq!(
            serde_json::to_value(ImportFormat::SshConfig).expect("serialize"),
            json!("sshConfig")
        );
        assert_eq!(
            serde_json::to_value(ImportFormat::Mremoteng).expect("serialize"),
            json!("mremoteng")
        );
        assert_eq!(
            serde_json::to_value(ExportFormat::Rdp).expect("serialize"),
            json!("rdp")
        );
    }

    #[test]
    fn preview_json_shape() {
        let p = ImportPreview {
            preview_id: Uuid::from_u128(5),
            nodes: vec![],
            warnings: vec![ImportWarning {
                message: "password discarded".into(),
                line: Some(3),
            }],
        };
        assert_eq!(
            serde_json::to_value(p).expect("serialize"),
            json!({
                "previewId": "00000000-0000-0000-0000-000000000005",
                "nodes": [],
                "warnings": [{"message": "password discarded", "line": 3}]
            })
        );
    }

    #[test]
    fn import_errors_map_to_stable_codes() {
        let code = |e: ImportError| AppError::from(e).code;
        assert_eq!(code(ImportError::NotFound("x".into())), ErrorCode::NotFound);
        assert_eq!(
            code(ImportError::InvalidInput("x".into())),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(ImportError::Unsupported("x".into())),
            ErrorCode::Unsupported
        );
    }

    fn preview_with(id: u128) -> ImportPreview {
        ImportPreview {
            preview_id: Uuid::from_u128(id),
            nodes: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn cache_evicts_the_oldest_preview() {
        let mut cache = PreviewCache::default();
        for id in 1..=(PREVIEW_CAPACITY as u128 + 1) {
            cache.insert(preview_with(id));
        }
        assert_eq!(cache.len(), PREVIEW_CAPACITY);
        assert!(cache.get(Uuid::from_u128(1)).is_none());
        assert!(cache.get(Uuid::from_u128(2)).is_some());
        cache.remove(Uuid::from_u128(2));
        assert!(cache.get(Uuid::from_u128(2)).is_none());
        assert!(!cache.is_empty());
    }
}
