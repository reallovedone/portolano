//! Credential inheritance (§4.1).

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::error::ModelError;
use crate::model::{CredentialRef, Document, Node};

/// Walks from `node_id` up through its parents until a `CredentialRef::Id` or
/// `CredentialRef::None` is found. `Ok(None)` means "no credential" (explicit `None`, or
/// `Inherit` all the way up to a root node).
///
/// Signature deviation from the plan (`-> Option<Uuid>`): it returns a `Result` because it
/// reports unknown ids (`NotFound`) and parent cycles (`InvalidInput`).
pub fn resolve_credential(doc: &Document, connection_id: Uuid) -> Result<Option<Uuid>, ModelError> {
    let index: HashMap<Uuid, &Node> = doc.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut visited: HashSet<Uuid> = HashSet::new();
    let mut current = connection_id;
    loop {
        let node = index
            .get(&current)
            .copied()
            .ok_or_else(|| ModelError::NotFound(current.to_string()))?;
        if !visited.insert(current) {
            return Err(ModelError::InvalidInput(format!(
                "cycle in the parents of node {connection_id}"
            )));
        }
        let reference = match node {
            Node::Folder(f) => f.credential,
            Node::Connection(c) => c.credential,
        };
        match reference {
            CredentialRef::Id { id } => return Ok(Some(id)),
            CredentialRef::None => return Ok(None),
            CredentialRef::Inherit => match node.parent_id() {
                Some(parent) => current = parent,
                None => return Ok(None),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Connection, ConnectionOptions, Folder};

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(id: u128, parent: Option<u128>, credential: CredentialRef) -> Node {
        Node::Folder(Folder {
            sort_index: None,
            icon: None,
            color: None,
            id: u(id),
            parent_id: parent.map(u),
            name: format!("f{id}"),
            credential,
        })
    }

    fn conn(id: u128, parent: Option<u128>, credential: CredentialRef) -> Node {
        Node::Connection(Connection {
            sort_index: None,
            icon: None,
            color: None,
            id: u(id),
            parent_id: parent.map(u),
            name: format!("c{id}"),
            host: "h".into(),
            port: None,
            credential,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Vnc { view_only: false },
        })
    }

    fn doc(nodes: Vec<Node>) -> Document {
        Document {
            nodes,
            ..Document::default()
        }
    }

    #[test]
    fn own_id_wins() {
        let d = doc(vec![
            folder(1, None, CredentialRef::Id { id: u(100) }),
            conn(2, Some(1), CredentialRef::Id { id: u(200) }),
        ]);
        assert_eq!(resolve_credential(&d, u(2)).expect("ok"), Some(u(200)));
    }

    #[test]
    fn inherits_over_several_levels() {
        let d = doc(vec![
            folder(1, None, CredentialRef::Id { id: u(100) }),
            folder(2, Some(1), CredentialRef::Inherit),
            folder(3, Some(2), CredentialRef::Inherit),
            conn(4, Some(3), CredentialRef::Inherit),
        ]);
        assert_eq!(resolve_credential(&d, u(4)).expect("ok"), Some(u(100)));
    }

    #[test]
    fn nearest_ancestor_with_value_wins() {
        let d = doc(vec![
            folder(1, None, CredentialRef::Id { id: u(100) }),
            folder(2, Some(1), CredentialRef::Id { id: u(200) }),
            folder(3, Some(2), CredentialRef::Inherit),
            conn(4, Some(3), CredentialRef::Inherit),
        ]);
        assert_eq!(resolve_credential(&d, u(4)).expect("ok"), Some(u(200)));
    }

    #[test]
    fn explicit_none_stops_the_walk() {
        let d = doc(vec![
            folder(1, None, CredentialRef::Id { id: u(100) }),
            folder(2, Some(1), CredentialRef::None),
            conn(3, Some(2), CredentialRef::Inherit),
        ]);
        assert_eq!(resolve_credential(&d, u(3)).expect("ok"), None);
    }

    #[test]
    fn root_inherit_resolves_to_none() {
        let d = doc(vec![conn(1, None, CredentialRef::Inherit)]);
        assert_eq!(resolve_credential(&d, u(1)).expect("ok"), None);
    }

    #[test]
    fn unknown_id_is_not_found() {
        let err = resolve_credential(&Document::default(), u(9)).expect_err("unknown");
        assert!(matches!(err, ModelError::NotFound(_)));
    }

    #[test]
    fn dangling_parent_is_not_found() {
        let d = doc(vec![conn(1, Some(77), CredentialRef::Inherit)]);
        let err = resolve_credential(&d, u(1)).expect_err("dangling");
        assert!(matches!(err, ModelError::NotFound(_)));
    }

    #[test]
    fn parent_cycle_is_an_error() {
        let d = doc(vec![
            folder(1, Some(2), CredentialRef::Inherit),
            folder(2, Some(1), CredentialRef::Inherit),
            conn(3, Some(1), CredentialRef::Inherit),
        ]);
        let err = resolve_credential(&d, u(3)).expect_err("cycle");
        assert!(matches!(err, ModelError::InvalidInput(_)));
    }
}
