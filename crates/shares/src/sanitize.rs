//! Remote content is never trusted: whatever a share file says, what reaches the tree cannot
//! carry credentials, switch off certificate or NLA checks, or run a local program.

use std::collections::{BTreeMap, BTreeSet};

use portolano_core_model::appearance::{is_valid_color, is_valid_icon};
use portolano_core_model::{host_is_storable, ConnectionOptions, CredentialRef, Document, Node};
use uuid::Uuid;

use crate::error::ShareError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizeOutcome {
    pub nodes: Vec<Node>,
    /// The id of the single root folder.
    pub root: Uuid,
    /// Names of the connections that were left out (never their content).
    pub skipped: Vec<String>,
}

const MAX_NAME_CHARS: usize = 200;
const MAX_NOTES_CHARS: usize = 10_000;
/// Longest host name a shared connection may carry (the DNS limit).
const MAX_HOST_BYTES: usize = 253;
/// Most items a share may hold.
pub const MAX_NODES: usize = 10_000;
/// Most folder levels a share may have (the root folder is level 1).
pub const MAX_FOLDER_DEPTH: usize = 32;

/// Unicode bidirectional and format controls that can make text read differently from how it
/// is stored.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    )
}

/// `text` without control or bidi characters (and, when `keep_layout`, with `\n` and `\t`
/// kept), cut to `max` characters.
fn clean_text(text: &str, keep_layout: bool, max: usize) -> String {
    text.chars()
        .filter(|c| !is_bidi_control(*c))
        .filter(|c| !c.is_control() || (keep_layout && matches!(c, '\n' | '\t')))
        .take(max)
        .collect()
}

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_control() || is_bidi_control(c))
}

/// Whether a web address may be shared: http or https, with a host that can be stored.
fn web_url_is_acceptable(url: &str) -> bool {
    if has_control(url) {
        return false;
    }
    match url::Url::parse(url) {
        Ok(parsed) => {
            matches!(parsed.scheme(), "http" | "https")
                && parsed.host_str().is_some_and(host_is_storable)
        }
        Err(_) => false,
    }
}

fn invalid(reason: &str) -> ShareError {
    ShareError::InvalidContent(reason.to_owned())
}

fn too_complex() -> ShareError {
    ShareError::TooComplex {
        max_nodes: MAX_NODES,
        max_depth: MAX_FOLDER_DEPTH,
    }
}

/// The subtree of `folder_id` in `doc`, ready to be published: its root loses the parent and
/// everything goes through [`sanitize`].
pub fn prepare_for_publish(doc: &Document, folder_id: Uuid) -> Result<Vec<Node>, ShareError> {
    match doc.node(folder_id) {
        Some(node) if node.is_folder() => {}
        _ => return Err(invalid("the folder to publish does not exist")),
    }
    let mut included: BTreeSet<Uuid> = BTreeSet::from([folder_id]);
    let mut frontier = vec![folder_id];
    while let Some(parent) = frontier.pop() {
        for node in doc.nodes.iter().filter(|n| n.parent_id() == Some(parent)) {
            if included.insert(node.id()) {
                frontier.push(node.id());
            }
        }
    }
    let nodes = doc
        .nodes
        .iter()
        .filter(|n| included.contains(&n.id()))
        .cloned()
        .map(|mut n| {
            if n.id() == folder_id {
                n.set_parent_id(None);
            }
            n
        })
        .collect();
    Ok(sanitize(nodes)?.nodes)
}

/// Validates the shape of the tree (one root folder, every node below it, no duplicate ids, no
/// cycles) and strips everything unsafe. Order is preserved.
pub fn sanitize(nodes: Vec<Node>) -> Result<SanitizeOutcome, ShareError> {
    if nodes.len() > MAX_NODES {
        return Err(too_complex());
    }
    let mut by_id: BTreeMap<Uuid, &Node> = BTreeMap::new();
    for node in &nodes {
        if by_id.insert(node.id(), node).is_some() {
            return Err(invalid("two items have the same id"));
        }
    }
    let mut roots = nodes.iter().filter(|n| n.parent_id().is_none());
    let root = match (roots.next(), roots.next()) {
        (Some(root), None) if root.is_folder() => root.id(),
        (Some(_), None) => return Err(invalid("the shared item is not a folder")),
        (None, _) => return Err(invalid("there is no root folder")),
        (Some(_), Some(_)) => return Err(invalid("there is more than one root")),
    };
    // Every node must reach the root through folders: this also rules out cycles and orphans.
    // Depths are memoised, so the pass is linear however deep the chain is.
    let mut depth: BTreeMap<Uuid, usize> = BTreeMap::from([(root, 0)]);
    for node in &nodes {
        let mut path: Vec<Uuid> = Vec::new();
        let mut current = node;
        let base = loop {
            if let Some(d) = depth.get(&current.id()) {
                break *d;
            }
            path.push(current.id());
            if path.len() > nodes.len() {
                return Err(invalid("the folders contain a cycle"));
            }
            let Some(parent) = current.parent_id() else {
                // A parentless node other than the root was ruled out above.
                return Err(invalid("an item is not inside the shared folder"));
            };
            match by_id.get(&parent) {
                Some(p) if p.is_folder() => current = p,
                _ => return Err(invalid("an item is not inside the shared folder")),
            }
        };
        for (offset, id) in path.iter().rev().enumerate() {
            depth.insert(*id, base + offset + 1);
        }
    }
    let too_deep = nodes
        .iter()
        .filter(|n| n.is_folder())
        .any(|n| depth.get(&n.id()).is_some_and(|d| d + 1 > MAX_FOLDER_DEPTH));
    if too_deep {
        return Err(too_complex());
    }

    let mut skipped = Vec::new();
    let mut kept: Vec<Node> = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::Folder(mut f) => {
                f.name = clean_text(&f.name, false, MAX_NAME_CHARS);
                f.credential = CredentialRef::Inherit;
                f.icon = f.icon.filter(|i| is_valid_icon(i));
                f.color = f.color.filter(|c| is_valid_color(c));
                kept.push(Node::Folder(f));
            }
            Node::Connection(mut c) => {
                c.name = clean_text(&c.name, false, MAX_NAME_CHARS);
                c.notes = clean_text(&c.notes, true, MAX_NOTES_CHARS);
                c.tags = c
                    .tags
                    .iter()
                    .map(|t| clean_text(t, false, MAX_NAME_CHARS))
                    .collect();
                let unusable = match &c.options {
                    ConnectionOptions::LocalShell { .. } => true,
                    ConnectionOptions::Web { url, .. } => !web_url_is_acceptable(url),
                    ConnectionOptions::Rdp { domain, .. } => {
                        domain.as_deref().is_some_and(has_control)
                    }
                    _ => false,
                };
                if unusable
                    || !host_is_storable(&c.host)
                    || c.host.len() > MAX_HOST_BYTES
                    || c.username.as_deref().is_some_and(has_control)
                {
                    skipped.push(c.name.clone());
                    continue;
                }
                c.credential = CredentialRef::Inherit;
                c.icon = c.icon.filter(|i| is_valid_icon(i));
                c.color = c.color.filter(|col| is_valid_color(col));
                if let ConnectionOptions::Rdp {
                    ignore_cert,
                    allow_without_nla,
                    kerberos_kdc,
                    share_clipboard,
                    ..
                } = &mut c.options
                {
                    *ignore_cert = false;
                    *allow_without_nla = false;
                    *share_clipboard = false;
                    *kerberos_kdc = None;
                }
                kept.push(Node::Connection(c));
            }
        }
    }
    let remaining: BTreeSet<Uuid> = kept.iter().map(Node::id).collect();
    for node in &mut kept {
        if let Node::Connection(c) = node {
            if let ConnectionOptions::Ssh { jump_host, .. } = &mut c.options {
                if jump_host.is_some_and(|j| !remaining.contains(&j)) {
                    *jump_host = None;
                }
            }
        }
    }
    Ok(SanitizeOutcome {
        nodes: kept,
        root,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use portolano_core_model::{Connection, Folder};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(n: u128, parent: Option<u128>) -> Node {
        Node::Folder(Folder {
            id: id(n),
            parent_id: parent.map(id),
            name: format!("folder {n}"),
            credential: CredentialRef::Id { id: id(900) },
            sort_index: None,
            icon: Some("not-an-icon".into()),
            color: Some("not-a-colour".into()),
        })
    }

    fn conn(n: u128, parent: Option<u128>, options: ConnectionOptions) -> Node {
        Node::Connection(Connection {
            id: id(n),
            parent_id: parent.map(id),
            name: format!("conn {n}"),
            host: "h.example.net".into(),
            port: None,
            credential: CredentialRef::Id { id: id(901) },
            username: None,
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

    fn rdp() -> ConnectionOptions {
        ConnectionOptions::Rdp {
            width: None,
            height: None,
            domain: None,
            ignore_cert: true,
            allow_without_nla: true,
            kerberos_kdc: Some("dc.example.com".into()),
            share_clipboard: true,
            mac_shortcuts: false,
        }
    }

    fn shell() -> ConnectionOptions {
        ConnectionOptions::LocalShell {
            program: Some("/bin/sh".into()),
            args: vec![],
            cwd: None,
        }
    }

    #[test]
    fn sanitize_sets_credentials_to_inherit() {
        let out = sanitize(vec![folder(1, None), conn(2, Some(1), ssh(None))]).expect("ok");
        for node in &out.nodes {
            let credential = match node {
                Node::Folder(f) => f.credential,
                Node::Connection(c) => c.credential,
            };
            assert_eq!(credential, CredentialRef::Inherit);
        }
        assert_eq!(out.root, id(1));
        let Node::Folder(f) = &out.nodes[0] else {
            panic!("folder")
        };
        assert_eq!((f.icon.clone(), f.color.clone()), (None, None));
    }

    #[test]
    fn sanitize_resets_ignore_cert_and_nla() {
        let out = sanitize(vec![folder(1, None), conn(2, Some(1), rdp())]).expect("ok");
        let Node::Connection(c) = &out.nodes[1] else {
            panic!("connection")
        };
        let ConnectionOptions::Rdp {
            ignore_cert,
            allow_without_nla,
            kerberos_kdc,
            share_clipboard,
            ..
        } = &c.options
        else {
            panic!("rdp")
        };
        assert!(!ignore_cert && !allow_without_nla && !share_clipboard);
        assert_eq!(*kerberos_kdc, None);
    }

    #[test]
    fn sanitize_drops_local_shells_and_reports_them() {
        let out = sanitize(vec![
            folder(1, None),
            conn(2, Some(1), shell()),
            conn(3, Some(1), ssh(None)),
        ])
        .expect("ok");
        assert_eq!(out.nodes.len(), 2);
        assert_eq!(out.skipped, vec!["conn 2".to_owned()]);
    }

    #[test]
    fn sanitize_drops_outside_jump_host() {
        let out = sanitize(vec![
            folder(1, None),
            conn(2, Some(1), ssh(Some(3))),
            conn(3, Some(1), ssh(None)),
            conn(4, Some(1), ssh(Some(99))),
            conn(5, Some(1), ssh(Some(6))),
            conn(6, Some(1), shell()),
        ])
        .expect("ok");
        let jump = |n: usize| match &out.nodes[n] {
            Node::Connection(c) => match &c.options {
                ConnectionOptions::Ssh { jump_host, .. } => *jump_host,
                _ => panic!("ssh"),
            },
            Node::Folder(_) => panic!("connection"),
        };
        assert_eq!(jump(1), Some(id(3)));
        assert_eq!(jump(3), None);
        assert_eq!(jump(4), None); // pointed at a dropped local shell
    }

    #[test]
    fn sanitize_drops_hosts_that_cannot_be_stored() {
        let mut bad = conn(2, Some(1), ssh(None));
        if let Node::Connection(c) = &mut bad {
            c.host = "evil host\nname".into();
        }
        let out = sanitize(vec![folder(1, None), bad]).expect("ok");
        assert_eq!(out.nodes.len(), 1);
        assert_eq!(out.skipped, vec!["conn 2".to_owned()]);
    }

    #[test]
    fn sanitize_limits_the_host_length() {
        let with_host = |len: usize| {
            let mut c = conn(2, Some(1), ssh(None));
            if let Node::Connection(c) = &mut c {
                c.host = "a".repeat(len);
            }
            sanitize(vec![folder(1, None), c]).expect("ok")
        };
        let ok = with_host(MAX_HOST_BYTES);
        assert_eq!(ok.nodes.len(), 2);
        let long = with_host(MAX_HOST_BYTES + 1);
        assert_eq!(long.nodes.len(), 1);
        assert_eq!(long.skipped, vec!["conn 2".to_owned()]);
    }

    fn web(n: u128, url: &str) -> Node {
        conn(
            n,
            Some(1),
            ConnectionOptions::Web {
                url: url.into(),
                isolated_profile: false,
            },
        )
    }

    #[test]
    fn sanitize_drops_web_with_bad_url_or_scheme() {
        let mut nodes = vec![folder(1, None), web(2, "https://ok.example.net/path")];
        for (n, url) in [
            (3, "file:///etc/passwd"),
            (4, "javascript:alert(1)"),
            (5, "not a url"),
            (6, "https://exa mple.net"),
            (7, "https://ok.example.net/\u{7}"),
            (8, "http://"),
        ] {
            nodes.push(web(n, url));
        }
        nodes.push(web(9, "http://other.example.net"));
        let mut bad_user = conn(10, Some(1), ssh(None));
        if let Node::Connection(c) = &mut bad_user {
            c.username = Some("a\u{1b}b".into());
        }
        let mut bad_domain = conn(11, Some(1), rdp());
        if let Node::Connection(c) = &mut bad_domain {
            if let ConnectionOptions::Rdp { domain, .. } = &mut c.options {
                *domain = Some("CORP\n".into());
            }
        }
        nodes.extend([bad_user, bad_domain]);
        let out = sanitize(nodes).expect("ok");
        let kept: Vec<Uuid> = out.nodes.iter().map(Node::id).collect();
        assert_eq!(kept, vec![id(1), id(2), id(9)]);
        assert_eq!(out.skipped.len(), 8);
    }

    #[test]
    fn sanitize_strips_control_chars_and_caps_lengths() {
        let mut c = conn(2, Some(1), ssh(None));
        if let Node::Connection(c) = &mut c {
            c.name = format!("a\u{7}b\n{}", "x".repeat(300));
            c.notes = format!("l1\nl2\tz\u{1b}[31m{}", "n".repeat(20_000));
            c.tags = vec!["t\u{0}ag".into()];
        }
        let mut f = folder(1, None);
        if let Node::Folder(f) = &mut f {
            f.name = "fo\rld".into();
        }
        let out = sanitize(vec![f, c]).expect("ok");
        assert_eq!(out.nodes[0].name(), "fold");
        let Node::Connection(c) = &out.nodes[1] else {
            panic!("connection")
        };
        assert_eq!(c.name.chars().count(), 200);
        assert!(c.name.starts_with("abxxx"));
        assert!(c.notes.starts_with("l1\nl2\tz[31m"));
        assert_eq!(c.notes.chars().count(), 10_000);
        assert_eq!(c.tags, vec!["tag".to_owned()]);
    }

    #[test]
    fn sanitize_rejects_two_roots_cycles_and_duplicate_ids() {
        let invalid = |nodes| matches!(sanitize(nodes), Err(ShareError::InvalidContent(_)));
        assert!(invalid(vec![folder(1, None), folder(2, None)]));
        assert!(invalid(vec![folder(1, None), folder(1, None)]));
        // 2 and 3 point at each other, detached from the root.
        assert!(invalid(vec![
            folder(1, None),
            folder(2, Some(3)),
            folder(3, Some(2))
        ]));
        assert!(invalid(vec![folder(1, None), folder(2, Some(42))]));
        assert!(invalid(vec![]));
        assert!(invalid(vec![conn(1, None, ssh(None))]));
        // A connection cannot be a parent.
        assert!(invalid(vec![
            folder(1, None),
            conn(2, Some(1), ssh(None)),
            folder(3, Some(2))
        ]));
    }

    #[test]
    fn sanitize_strips_bidi_controls() {
        let mut c = conn(2, Some(1), ssh(None));
        if let Node::Connection(c) = &mut c {
            c.name = "a\u{202E}b\u{2066}c\u{2069}d\u{200E}e\u{200F}f\u{061C}g".into();
            c.notes = "n\u{202A}o\u{202D}p".into();
        }
        let out = sanitize(vec![folder(1, None), c]).expect("ok");
        let Node::Connection(c) = &out.nodes[1] else {
            panic!("connection")
        };
        assert_eq!(c.name, "abcdefg");
        assert_eq!(c.notes, "nop");
        // A host or username carrying one is not storable at all.
        let mut bad = conn(3, Some(1), ssh(None));
        if let Node::Connection(c) = &mut bad {
            c.username = Some("ad\u{202E}min".into());
        }
        let out = sanitize(vec![folder(1, None), bad]).expect("ok");
        assert_eq!(out.nodes.len(), 1);
    }

    /// A chain of `levels` folders (the root is level 1).
    fn chain(levels: u128) -> Vec<Node> {
        (1..=levels)
            .map(|n| folder(n, (n > 1).then_some(n - 1)))
            .collect()
    }

    #[test]
    fn sanitize_limits_folder_depth() {
        let max = MAX_FOLDER_DEPTH as u128;
        assert!(sanitize(chain(max)).is_ok());
        assert!(matches!(
            sanitize(chain(max + 1)),
            Err(ShareError::TooComplex { .. })
        ));
        // Connections below the deepest allowed folder do not count as a folder level.
        let mut nodes = chain(max);
        nodes.push(conn(1000, Some(max), ssh(None)));
        assert!(sanitize(nodes).is_ok());
        // A very long chain is rejected quickly (linear pass, no per-node walk).
        assert!(matches!(
            sanitize(chain(9_000)),
            Err(ShareError::TooComplex { .. })
        ));
    }

    #[test]
    fn sanitize_limits_node_count() {
        let flat = |count: usize| {
            let mut nodes = vec![folder(1, None)];
            nodes.extend((0..count - 1).map(|i| conn(10 + i as u128, Some(1), ssh(None))));
            nodes
        };
        assert!(sanitize(flat(MAX_NODES)).is_ok());
        assert!(matches!(
            sanitize(flat(MAX_NODES + 1)),
            Err(ShareError::TooComplex { .. })
        ));
    }

    #[test]
    fn prepare_for_publish_takes_only_the_subtree() {
        let doc = Document {
            nodes: vec![
                folder(1, None),
                folder(2, Some(1)),
                conn(3, Some(2), ssh(Some(4))),
                conn(4, Some(1), ssh(None)),
                folder(5, Some(2)),
                conn(6, Some(5), rdp()),
            ],
            ..Document::default()
        };
        let nodes = prepare_for_publish(&doc, id(2)).expect("ok");
        let ids: Vec<Uuid> = nodes.iter().map(Node::id).collect();
        assert_eq!(ids, vec![id(2), id(3), id(5), id(6)]);
        assert_eq!(nodes[0].parent_id(), None);
        let Node::Connection(c) = &nodes[1] else {
            panic!("connection")
        };
        // The jump host lives outside the subtree.
        assert!(matches!(
            c.options,
            ConnectionOptions::Ssh {
                jump_host: None,
                ..
            }
        ));
        assert!(prepare_for_publish(&doc, id(3)).is_err());
        assert!(prepare_for_publish(&doc, id(77)).is_err());
    }
}
