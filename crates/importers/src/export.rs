//! Export: selection of nodes with their descendants, and the atomic write of the file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::Path;

use portolano_core_model::{Document, Node};
use uuid::Uuid;

use crate::{json, rdp, ExportFormat, ImportError};

/// The nodes to export: the whole document for `None`, otherwise the given nodes and all their
/// descendants, in document order. A selected node whose parent is not selected becomes a root
/// of the exported fragment.
pub fn select(doc: &Document, node_ids: Option<&[Uuid]>) -> Result<Vec<Node>, ImportError> {
    let Some(ids) = node_ids else {
        return Ok(doc.nodes.clone());
    };
    let mut keep: HashSet<Uuid> = HashSet::new();
    let mut queue: VecDeque<Uuid> = VecDeque::new();
    for id in ids {
        if doc.node(*id).is_none() {
            return Err(ImportError::NotFound(format!("node {id} not found")));
        }
        if keep.insert(*id) {
            queue.push_back(*id);
        }
    }
    let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for node in &doc.nodes {
        if let Some(parent) = node.parent_id() {
            children.entry(parent).or_default().push(node.id());
        }
    }
    while let Some(id) = queue.pop_front() {
        for child in children.get(&id).into_iter().flatten() {
            if keep.insert(*child) {
                queue.push_back(*child);
            }
        }
    }
    Ok(doc
        .nodes
        .iter()
        .filter(|node| keep.contains(&node.id()))
        .map(|node| {
            let mut node = node.clone();
            if node
                .parent_id()
                .is_some_and(|parent| !keep.contains(&parent))
            {
                node.set_parent_id(None);
            }
            node
        })
        .collect())
}

/// Writes `nodes` to `path` as `format`. The file is replaced atomically (temporary file in the
/// same folder, then rename), so a failure leaves the previous file intact.
pub fn export(format: ExportFormat, path: &Path, nodes: &[Node]) -> Result<(), ImportError> {
    let bytes = match format {
        ExportFormat::Json => json::export_bytes(nodes)?,
        ExportFormat::Rdp => rdp::export_bytes(nodes)?,
    };
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| ImportError::Io(e.error))?;
    Ok(())
}
