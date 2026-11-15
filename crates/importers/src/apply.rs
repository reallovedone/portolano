//! Applies a preview to a `DocumentStore`, all nodes or none.
//!
//! Everything that can be refused is checked first (target folder, id clashes, parent links,
//! cycles), then all nodes are added in one `DocumentStore::apply_batch`: a single clone, a
//! single validation and a single save. If anything fails the document, in memory and on disk,
//! is left untouched.

use std::collections::{HashMap, HashSet, VecDeque};

use portolano_core_model::{DocumentStore, Node};
use uuid::Uuid;

use crate::{ImportError, ImportPreview};

/// Inserts `preview` under `target` (`None` = root) and returns the number of nodes imported.
pub fn apply(
    store: &mut DocumentStore,
    target: Option<Uuid>,
    preview: &ImportPreview,
) -> Result<u32, ImportError> {
    store.ensure_loaded()?;
    if let Some(folder) = target {
        match store.document().node(folder) {
            None => {
                return Err(ImportError::NotFound(format!(
                    "target folder {folder} not found"
                )))
            }
            Some(node) if !node.is_folder() => {
                return Err(ImportError::InvalidInput(
                    "the target is not a folder".into(),
                ))
            }
            Some(_) => {}
        }
    }
    let existing: HashSet<Uuid> = store.document().nodes.iter().map(Node::id).collect();
    if preview.nodes.iter().any(|n| existing.contains(&n.id())) {
        return Err(ImportError::InvalidInput(
            "the preview conflicts with existing nodes: run the import again".into(),
        ));
    }
    let order = insertion_order(&preview.nodes)?;

    let count = u32::try_from(order.len())
        .map_err(|_| ImportError::InvalidInput("too many nodes to import".into()))?;
    store.apply_batch(|doc| {
        doc.nodes.reserve(order.len());
        for node in order {
            let mut node = node.clone();
            if node.parent_id().is_none() {
                node.set_parent_id(target);
            }
            doc.nodes.push(node);
        }
        Ok(())
    })?;
    Ok(count)
}

/// The nodes ordered so that every parent comes before its children. Refuses duplicate ids,
/// parents outside the preview and parent cycles.
fn insertion_order(nodes: &[Node]) -> Result<Vec<&Node>, ImportError> {
    let mut by_id: HashMap<Uuid, &Node> = HashMap::with_capacity(nodes.len());
    for node in nodes {
        if by_id.insert(node.id(), node).is_some() {
            return Err(ImportError::InvalidInput(
                "the preview contains a duplicate node id".into(),
            ));
        }
    }
    let mut children: HashMap<Uuid, Vec<&Node>> = HashMap::new();
    let mut queue: VecDeque<&Node> = VecDeque::new();
    for node in nodes {
        match node.parent_id() {
            None => queue.push_back(node),
            Some(parent) => {
                if !by_id.contains_key(&parent) {
                    return Err(ImportError::InvalidInput(
                        "the preview refers to a parent outside itself".into(),
                    ));
                }
                children.entry(parent).or_default().push(node);
            }
        }
    }
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for child in children.get(&node.id()).into_iter().flatten() {
            queue.push_back(child);
        }
    }
    if order.len() != nodes.len() {
        return Err(ImportError::InvalidInput(
            "the preview contains a parent cycle".into(),
        ));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents_come_first() {
        use portolano_core_model::{Connection, ConnectionOptions, CredentialRef, Folder};
        let folder = Uuid::from_u128(1);
        let child = Uuid::from_u128(2);
        let nodes = vec![
            Node::Connection(Connection {
                id: child,
                parent_id: Some(folder),
                name: "c".into(),
                host: "h".into(),
                port: None,
                credential: CredentialRef::None,
                username: None,
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::Vnc { view_only: false },
            }),
            Node::Folder(Folder {
                id: folder,
                parent_id: None,
                name: "f".into(),
                credential: CredentialRef::Inherit,
            }),
        ];
        let order = insertion_order(&nodes).expect("order");
        assert_eq!(order[0].id(), folder);
        assert_eq!(order[1].id(), child);
    }
}
