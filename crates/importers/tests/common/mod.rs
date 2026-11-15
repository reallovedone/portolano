//! Helpers shared by the integration tests. Every test file uses a subset, hence `dead_code`.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use portolano_core_model::{Connection, Node};
use portolano_importers::ImportPreview;
use uuid::Uuid;

/// The fake password used in fixtures. It must never appear in any preview or document.
pub const FAKE_PASSWORD: &str = "not-a-real-password";

/// Temporary folder under `<repo>/.tmp/wp10a/` (git-ignored), removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp/wp10a")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.0) {
            eprintln!("cleanup failed: {e}");
        }
    }
}

/// A synthetic fixture under `tests/fixtures/`.
pub fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

pub fn json(preview: &ImportPreview) -> String {
    serde_json::to_string(preview).expect("serialize preview")
}

pub fn node_names(preview: &ImportPreview) -> Vec<String> {
    preview.nodes.iter().map(|n| n.name().to_owned()).collect()
}

pub fn connection_names(preview: &ImportPreview) -> Vec<String> {
    preview
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Connection(c) => Some(c.name.clone()),
            Node::Folder(_) => None,
        })
        .collect()
}

pub fn warning_messages(preview: &ImportPreview) -> Vec<String> {
    preview.warnings.iter().map(|w| w.message.clone()).collect()
}

pub fn warnings_containing(preview: &ImportPreview, needle: &str) -> usize {
    preview
        .warnings
        .iter()
        .filter(|w| w.message.contains(needle))
        .count()
}

/// The first node with `name`. Panics when absent (test helper).
pub fn node<'a>(preview: &'a ImportPreview, name: &str) -> &'a Node {
    preview
        .nodes
        .iter()
        .find(|n| n.name() == name)
        .unwrap_or_else(|| panic!("no node named {name}"))
}

pub fn connection<'a>(preview: &'a ImportPreview, name: &str) -> &'a Connection {
    match node(preview, name) {
        Node::Connection(c) => c,
        Node::Folder(_) => panic!("{name} is a folder"),
    }
}

/// Name of the parent of the node called `name` (`None` at the root).
pub fn parent_name(preview: &ImportPreview, name: &str) -> Option<String> {
    let parent = node(preview, name).parent_id()?;
    preview
        .nodes
        .iter()
        .find(|n| n.id() == parent)
        .map(|n| n.name().to_owned())
}

pub fn id_of(preview: &ImportPreview, name: &str) -> Uuid {
    node(preview, name).id()
}

pub fn fake_password_absent(preview: &ImportPreview) {
    assert!(
        !json(preview).contains(FAKE_PASSWORD),
        "the fake password leaked into the preview"
    );
}
