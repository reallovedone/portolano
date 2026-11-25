//! The tree a subscriber sees: local nodes plus every mounted share, with remote ids derived
//! so they never collide with local ones and stay stable between syncs.

use std::collections::{BTreeMap, BTreeSet};

use portolano_core_model::{
    Connection, ConnectionOptions, CredentialRef, Document, Node, ShareLink, ShareTarget,
};
use uuid::Uuid;

use crate::sanitize::SanitizedNodes;

/// A share's sanitised nodes (source ids) together with the link that mounts them.
#[derive(Debug, Clone)]
pub struct Mount {
    pub link: ShareLink,
    pub nodes: SanitizedNodes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteNode {
    pub link_id: Uuid,
    pub source_id: Uuid,
    pub is_root: bool,
}

/// A connection of a share whose target is new or changed and not confirmed yet: what the user
/// is asked to approve.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingHost {
    /// The connection's id in the merged tree.
    pub node_id: Uuid,
    pub name: String,
    /// `ssh`, `rdp`, `vnc`, `web`, or `local` for a connection without a host.
    pub protocol: String,
    /// The host (for a web connection, its address).
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MergedView {
    /// Local nodes followed by the remote ones (derived ids).
    pub document: Document,
    /// Remote nodes by their derived id.
    pub remote: BTreeMap<Uuid, RemoteNode>,
    /// Per link id, the connections whose target is not confirmed (only links with at least one).
    pub pending: BTreeMap<Uuid, u32>,
    /// The same connections as `pending` counts, one entry each, in tree order.
    pub pending_hosts: BTreeMap<Uuid, Vec<PendingHost>>,
}

/// The local id of the remote node `source_id` of link `link_id`.
pub fn derive_id(link_id: Uuid, source_id: Uuid) -> Uuid {
    Uuid::new_v5(&link_id, source_id.as_bytes())
}

/// How a connection is listed for the user to approve: `node_id` is the id it is shown under.
pub fn pending_host(node_id: Uuid, connection: &Connection) -> PendingHost {
    let target = target_of(connection);
    PendingHost {
        node_id,
        name: connection.name.clone(),
        protocol: target
            .as_ref()
            .map_or_else(|| "local".to_owned(), |t| t.protocol.clone()),
        host: target.map(|t| t.host).unwrap_or_default(),
        port: connection.port,
        username: connection.username.clone(),
    }
}

/// Where a connection sends its credential. `None` for kinds that have no host (local shell).
/// For a web connection the URL stands in for the host, so a changed address counts as a
/// changed target.
pub fn target_of(connection: &Connection) -> Option<ShareTarget> {
    let (protocol, host, domain, jump) = match &connection.options {
        ConnectionOptions::LocalShell { .. } => return None,
        ConnectionOptions::Ssh { jump_host, .. } => {
            ("ssh", connection.host.clone(), None, *jump_host)
        }
        ConnectionOptions::Rdp { domain, .. } => {
            ("rdp", connection.host.clone(), domain.clone(), None)
        }
        ConnectionOptions::Vnc { .. } => ("vnc", connection.host.clone(), None, None),
        ConnectionOptions::Web { url, .. } => ("web", url.clone(), None, None),
    };
    Some(ShareTarget {
        protocol: protocol.to_owned(),
        host,
        port: connection.port,
        username: connection.username.clone(),
        domain,
        jump_host: jump,
    })
}

pub fn merge(local: &Document, mounts: &[Mount]) -> MergedView {
    let mut document = local.clone();
    let mut remote = BTreeMap::new();
    let mut pending = BTreeMap::new();
    let mut pending_hosts = BTreeMap::new();
    let local_folders: BTreeSet<Uuid> = local
        .nodes
        .iter()
        .filter(|n| n.is_folder())
        .map(Node::id)
        .collect();

    for mount in mounts {
        let link = &mount.link;
        let mount_parent = link.parent_id.filter(|p| local_folders.contains(p));
        let source_ids: BTreeSet<Uuid> = mount.nodes.iter().map(Node::id).collect();
        let mut unconfirmed = 0u32;
        let mut hosts = Vec::new();
        for node in &mount.nodes {
            let source_id = node.id();
            let derived = derive_id(link.id, source_id);
            let is_root = node.parent_id().is_none();
            let parent_id = match node.parent_id() {
                None => mount_parent,
                Some(parent) => Some(derive_id(link.id, parent)),
            };
            let credential = |confirmed: bool| {
                if confirmed {
                    link.overrides
                        .get(&source_id)
                        .copied()
                        .unwrap_or(CredentialRef::Inherit)
                } else {
                    CredentialRef::None
                }
            };
            let merged = match node {
                Node::Folder(f) => {
                    let mut f = f.clone();
                    f.id = derived;
                    f.parent_id = parent_id;
                    f.credential = credential(true);
                    Node::Folder(f)
                }
                Node::Connection(c) => {
                    let mut c = c.clone();
                    let confirmed = match (target_of(&c), link.confirmed_targets.get(&source_id)) {
                        (Some(actual), Some(known)) => actual == *known,
                        _ => false,
                    };
                    if !confirmed {
                        unconfirmed += 1;
                        hosts.push(pending_host(derived, &c));
                    }
                    c.id = derived;
                    c.parent_id = parent_id;
                    c.credential = credential(confirmed);
                    if let ConnectionOptions::Ssh { jump_host, .. } = &mut c.options {
                        *jump_host = jump_host
                            .filter(|j| source_ids.contains(j))
                            .map(|j| derive_id(link.id, j));
                    }
                    Node::Connection(c)
                }
            };
            document.nodes.push(merged);
            remote.insert(
                derived,
                RemoteNode {
                    link_id: link.id,
                    source_id,
                    is_root,
                },
            );
        }
        if unconfirmed > 0 {
            pending.insert(link.id, unconfirmed);
            pending_hosts.insert(link.id, hosts);
        }
    }
    MergedView {
        document,
        remote,
        pending,
        pending_hosts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(link: ShareLink, nodes: Vec<Node>) -> Mount {
        Mount {
            link,
            nodes: SanitizedNodes::unchecked(nodes),
        }
    }
    use portolano_core_model::{Folder, ShareRole};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(n: u128, parent: Option<u128>) -> Node {
        Node::Folder(Folder {
            id: id(n),
            parent_id: parent.map(id),
            name: format!("folder {n}"),
            credential: CredentialRef::Inherit,
            sort_index: None,
            icon: None,
            color: None,
        })
    }

    fn conn(n: u128, parent: Option<u128>, host: &str, options: ConnectionOptions) -> Node {
        Node::Connection(Connection {
            id: id(n),
            parent_id: parent.map(id),
            name: format!("conn {n}"),
            host: host.into(),
            port: Some(22),
            credential: CredentialRef::Inherit,
            username: Some("deploy".into()),
            tags: vec![],
            notes: String::new(),
            options,
            sort_index: None,
            icon: None,
            color: None,
        })
    }

    fn ssh(jump: Option<u128>) -> ConnectionOptions {
        ConnectionOptions::Ssh {
            jump_host: jump.map(id),
            keepalive_secs: None,
        }
    }

    fn link(parent: Option<u128>) -> ShareLink {
        ShareLink {
            id: id(500),
            share_id: id(501),
            name: "Prod".into(),
            role: ShareRole::Subscriber,
            location: None,
            publish_path: None,
            parent_id: parent.map(id),
            owner_name: "Ann".into(),
            owner_key: String::new(),
            last_revision: 1,
            editors_version: 1,
            last_sync: None,
            last_error: None,
            overrides: BTreeMap::new(),
            confirmed_targets: BTreeMap::new(),
            owned: None,
        }
    }

    fn remote_nodes() -> Vec<Node> {
        vec![
            folder(1, None),
            conn(2, Some(1), "web.example.net", ssh(Some(3))),
            conn(3, Some(1), "jump.example.net", ssh(None)),
        ]
    }

    fn credential_of(view: &MergedView, source: u128) -> CredentialRef {
        match view
            .document
            .node(derive_id(id(500), id(source)))
            .expect("node")
        {
            Node::Folder(f) => f.credential,
            Node::Connection(c) => c.credential,
        }
    }

    #[test]
    fn derive_id_is_stable() {
        assert_eq!(derive_id(id(1), id(2)), derive_id(id(1), id(2)));
        assert_ne!(derive_id(id(1), id(2)), derive_id(id(1), id(3)));
        assert_ne!(derive_id(id(1), id(2)), derive_id(id(9), id(2)));
        assert_ne!(derive_id(id(1), id(2)), id(2));
    }

    #[test]
    fn merge_forces_none_on_changed_or_new_target() {
        let nodes = remote_nodes();
        let Node::Connection(web) = &nodes[1] else {
            panic!("connection")
        };
        let mut l = link(None);
        // 2: confirmed for another host; 3: never confirmed.
        let mut stale = target_of(web).expect("target");
        stale.host = "old.example.net".into();
        l.confirmed_targets.insert(id(2), stale);
        l.overrides.insert(id(2), CredentialRef::Id { id: id(77) });
        l.overrides.insert(id(3), CredentialRef::Id { id: id(78) });
        let view = merge(&Document::default(), &[mount(l, nodes)]);
        assert_eq!(credential_of(&view, 2), CredentialRef::None);
        assert_eq!(credential_of(&view, 3), CredentialRef::None);
        assert_eq!(view.pending.get(&id(500)), Some(&2));
        assert_eq!(credential_of(&view, 1), CredentialRef::Inherit);
    }

    #[test]
    fn merge_applies_override_when_target_confirmed() {
        let nodes = remote_nodes();
        let mut l = link(None);
        for node in &nodes {
            if let Node::Connection(c) = node {
                l.confirmed_targets
                    .insert(c.id, target_of(c).expect("target"));
            }
        }
        l.overrides.insert(id(2), CredentialRef::Id { id: id(77) });
        l.overrides.insert(id(1), CredentialRef::Id { id: id(78) });
        let view = merge(&Document::default(), &[mount(l, nodes)]);
        assert_eq!(credential_of(&view, 2), CredentialRef::Id { id: id(77) });
        assert_eq!(credential_of(&view, 3), CredentialRef::Inherit);
        assert_eq!(credential_of(&view, 1), CredentialRef::Id { id: id(78) });
        assert!(view.pending.is_empty());
        // The jump host follows the derived id.
        let Some(Node::Connection(c)) = view.document.node(derive_id(id(500), id(2))) else {
            panic!("connection")
        };
        assert!(matches!(
            c.options,
            ConnectionOptions::Ssh { jump_host: Some(j), .. } if j == derive_id(id(500), id(3))
        ));
    }

    #[test]
    fn merge_mounts_under_parent_or_root_when_parent_missing() {
        let local = Document {
            nodes: vec![folder(10, None)],
            ..Document::default()
        };
        let root = derive_id(id(500), id(1));
        let under = merge(&local, &[mount(link(Some(10)), remote_nodes())]);
        assert_eq!(
            under.document.node(root).expect("root").parent_id(),
            Some(id(10))
        );
        assert!(under.remote[&root].is_root);
        assert!(!under.remote[&derive_id(id(500), id(2))].is_root);
        assert_eq!(under.document.nodes.len(), 4);

        let missing = merge(&local, &[mount(link(Some(404)), remote_nodes())]);
        assert_eq!(missing.document.node(root).expect("root").parent_id(), None);
        // A connection is not a mount point either.
        let local = Document {
            nodes: vec![conn(10, None, "h", ssh(None))],
            ..Document::default()
        };
        let on_conn = merge(&local, &[mount(link(Some(10)), remote_nodes())]);
        assert_eq!(on_conn.document.node(root).expect("root").parent_id(), None);
    }

    #[test]
    fn merge_marks_pending_when_jump_host_or_domain_changes() {
        let rdp_with = |domain: &str| ConnectionOptions::Rdp {
            width: None,
            height: None,
            domain: Some(domain.into()),
            ignore_cert: false,
            allow_without_nla: false,
            kerberos_kdc: None,
            share_clipboard: false,
            mac_shortcuts: false,
        };
        let confirmed_for = |nodes: &[Node]| {
            let mut l = link(None);
            for node in nodes {
                if let Node::Connection(c) = node {
                    l.confirmed_targets
                        .insert(c.id, target_of(c).expect("target"));
                }
            }
            l
        };
        let before = vec![
            folder(1, None),
            conn(2, Some(1), "a", ssh(Some(3))),
            conn(3, Some(1), "j", ssh(None)),
            conn(4, Some(1), "w", ssh(Some(3))),
            conn(5, Some(1), "r", rdp_with("CORP")),
        ];
        let l = confirmed_for(&before);
        // Unchanged: nothing pending.
        let same = merge(&Document::default(), &[mount(l.clone(), before.clone())]);
        assert!(same.pending.is_empty());
        // The owner reroutes connection 2 through another jump host (4) and moves 5 to a
        // different domain.
        let mut after = before;
        after[1] = conn(2, Some(1), "a", ssh(Some(4)));
        after[4] = conn(5, Some(1), "r", rdp_with("OTHER"));
        let view = merge(&Document::default(), &[mount(l, after)]);
        assert_eq!(view.pending.get(&id(500)), Some(&2));
        let hosts = &view.pending_hosts[&id(500)];
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].name, "conn 2");
        assert_eq!(hosts[0].host, "a");
        assert_eq!(hosts[0].protocol, "ssh");
        assert_eq!(hosts[1].name, "conn 5");
        assert_eq!(hosts[1].host, "r");
        assert_eq!(hosts[1].protocol, "rdp");
        assert_eq!(hosts[0].node_id, derive_id(id(500), id(2)));
        assert_eq!(credential_of(&view, 2), CredentialRef::None);
        assert_eq!(credential_of(&view, 5), CredentialRef::None);
        assert_eq!(credential_of(&view, 3), CredentialRef::Inherit);
    }

    #[test]
    fn target_of_has_no_target_for_local_shells() {
        let shell = conn(
            1,
            None,
            "",
            ConnectionOptions::LocalShell {
                program: None,
                args: vec![],
                cwd: None,
            },
        );
        let Node::Connection(c) = shell else {
            panic!("connection")
        };
        assert_eq!(target_of(&c), None);
    }
}
