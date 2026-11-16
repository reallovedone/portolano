//! `Draft`: the nodes and warnings collected by one parse, with the entry limit, folder reuse
//! and the jump-host checks shared by the parsers.

use std::collections::{HashMap, HashSet};

use portolano_core_model::{Connection, ConnectionOptions, CredentialRef, Folder, Node};
use uuid::Uuid;

use crate::{ImportError, ImportPreview, ImportWarning, MAX_ENTRIES};

/// Name given to an entry whose source has no usable name.
const UNNAMED: &str = "unnamed";

/// Fields of a connection as read from a source file. Ids are assigned by [`Draft`].
pub(crate) struct Entry {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub options: ConnectionOptions,
}

#[derive(Default)]
pub(crate) struct Draft {
    nodes: Vec<Node>,
    warnings: Vec<ImportWarning>,
    /// Folders created from path segments, keyed by the joined path.
    folders: HashMap<String, Uuid>,
    connections: usize,
}

impl Draft {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn warn(&mut self, line: Option<u32>, message: impl Into<String>) {
        self.warnings.push(ImportWarning {
            message: message.into(),
            line,
        });
    }

    pub(crate) fn has_connections(&self) -> bool {
        self.connections > 0
    }

    /// Adds a node, enforcing the entry limit and non-empty names. Returns its id.
    pub(crate) fn push(&mut self, mut node: Node, line: Option<u32>) -> Result<Uuid, ImportError> {
        if self.nodes.len() >= MAX_ENTRIES {
            return Err(ImportError::InvalidInput(format!(
                "the file has more than {MAX_ENTRIES} entries"
            )));
        }
        let trimmed = node.name().trim().to_owned();
        let name = if trimmed.is_empty() {
            self.warn(line, "an entry without a name was imported as 'unnamed'");
            UNNAMED.to_owned()
        } else {
            trimmed
        };
        set_name(&mut node, name);
        if matches!(node, Node::Connection(_)) {
            self.connections += 1;
        }
        let id = node.id();
        self.nodes.push(node);
        Ok(id)
    }

    pub(crate) fn add_folder(
        &mut self,
        parent: Option<Uuid>,
        name: &str,
        line: Option<u32>,
    ) -> Result<Uuid, ImportError> {
        self.push(
            Node::Folder(Folder {
                id: Uuid::new_v4(),
                parent_id: parent,
                name: name.to_owned(),
                // Imports never inherit a credential from the folder they land in.
                credential: CredentialRef::None,
            }),
            line,
        )
    }

    /// Adds a connection with no credential: the application asks for it when connecting.
    pub(crate) fn add_connection(
        &mut self,
        parent: Option<Uuid>,
        entry: Entry,
        line: Option<u32>,
    ) -> Result<Uuid, ImportError> {
        self.push(
            Node::Connection(Connection {
                id: Uuid::new_v4(),
                parent_id: parent,
                name: entry.name,
                host: entry.host,
                port: entry.port,
                credential: CredentialRef::None,
                username: entry.username,
                tags: entry.tags,
                notes: entry.notes,
                options: entry.options,
            }),
            line,
        )
    }

    /// Creates the folder chain `segments` (reusing folders already created for the same
    /// path) and returns the innermost one. An empty chain means the root.
    pub(crate) fn folder_path(
        &mut self,
        segments: &[String],
        line: Option<u32>,
    ) -> Result<Option<Uuid>, ImportError> {
        let mut parent = None;
        let mut key = String::new();
        for segment in segments {
            if !key.is_empty() {
                key.push('\u{0}');
            }
            key.push_str(segment);
            let id = match self.folders.get(&key) {
                Some(id) => *id,
                None => {
                    let id = self.add_folder(parent, segment, line)?;
                    self.folders.insert(key.clone(), id);
                    id
                }
            };
            parent = Some(id);
        }
        Ok(parent)
    }

    /// Points the SSH connection `id` at `jump`.
    pub(crate) fn set_jump_host(&mut self, id: Uuid, jump: Uuid) {
        for node in &mut self.nodes {
            if let Node::Connection(c) = node {
                if c.id == id {
                    if let ConnectionOptions::Ssh { jump_host, .. } = &mut c.options {
                        *jump_host = Some(jump);
                    }
                }
            }
        }
    }

    /// Removes the jump host of every SSH connection whose jump chain leads back to itself,
    /// so that no import can create a loop of jump hosts.
    pub(crate) fn break_jump_cycles(&mut self) {
        let mut jumps: HashMap<Uuid, Uuid> = HashMap::new();
        for node in &self.nodes {
            if let Node::Connection(c) = node {
                if let ConnectionOptions::Ssh {
                    jump_host: Some(jump),
                    ..
                } = c.options
                {
                    jumps.insert(c.id, jump);
                }
            }
        }
        let mut cut = Vec::new();
        for node in &self.nodes {
            let id = node.id();
            if !jumps.contains_key(&id) {
                continue;
            }
            let mut seen = HashSet::new();
            let mut cursor = jumps.get(&id).copied();
            let mut loops_back = false;
            while let Some(current) = cursor {
                if current == id {
                    loops_back = true;
                    break;
                }
                if !seen.insert(current) {
                    break;
                }
                cursor = jumps.get(&current).copied();
            }
            if loops_back {
                jumps.remove(&id);
                cut.push(id);
            }
        }
        for id in cut {
            let name = self.clear_jump_host(id);
            self.warn(
                None,
                format!("the jump host of '{name}' was dropped because it would loop back"),
            );
        }
    }

    /// Clears the jump host of connection `id` and returns its name.
    fn clear_jump_host(&mut self, id: Uuid) -> String {
        let mut name = String::new();
        for node in &mut self.nodes {
            if let Node::Connection(c) = node {
                if c.id == id {
                    name = c.name.clone();
                    if let ConnectionOptions::Ssh { jump_host, .. } = &mut c.options {
                        *jump_host = None;
                    }
                }
            }
        }
        name
    }

    /// Neutralises everything an imported file could use to act with the user's authority.
    /// A file may come from anyone, so whatever its format says:
    ///
    /// - every credential reference becomes `None` (never `Inherit` from the folder the import
    ///   lands in, never an id of a vault the file knows nothing about);
    /// - "do not verify the server certificate", "allow without NLA" and "share the clipboard"
    ///   are switched off;
    /// - a local shell with a custom program, arguments or working directory is not imported
    ///   (it would run that program when opened). A plain local shell is fine.
    ///
    /// Every change is reported as a warning, grouped when it repeats.
    pub(crate) fn sanitize(&mut self) {
        let mut credentials: Vec<String> = Vec::new();
        let mut ignore_cert: Vec<String> = Vec::new();
        let mut without_nla: Vec<String> = Vec::new();
        let mut clipboard: Vec<String> = Vec::new();
        let mut programs: Vec<String> = Vec::new();

        for node in &mut self.nodes {
            match node {
                Node::Folder(f) => {
                    if f.credential != CredentialRef::None {
                        f.credential = CredentialRef::None;
                        credentials.push(f.name.clone());
                    }
                }
                Node::Connection(c) => {
                    if c.credential != CredentialRef::None {
                        c.credential = CredentialRef::None;
                        credentials.push(c.name.clone());
                    }
                    match &mut c.options {
                        ConnectionOptions::Rdp {
                            ignore_cert: ignore,
                            allow_without_nla: without,
                            share_clipboard: share,
                            ..
                        } => {
                            if std::mem::take(share) {
                                clipboard.push(c.name.clone());
                            }
                            if std::mem::take(ignore) {
                                ignore_cert.push(c.name.clone());
                            }
                            if std::mem::take(without) {
                                without_nla.push(c.name.clone());
                            }
                        }
                        ConnectionOptions::LocalShell { program, args, cwd } => {
                            let custom = program.as_deref().is_some_and(|p| !p.trim().is_empty())
                                || !args.is_empty()
                                || cwd.as_deref().is_some_and(|p| !p.trim().is_empty());
                            if custom {
                                programs.push(c.name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if !programs.is_empty() {
            let skipped: HashSet<&str> = programs.iter().map(String::as_str).collect();
            let mut removed = 0usize;
            self.nodes.retain(|node| match node {
                Node::Connection(c)
                    if matches!(
                        &c.options,
                        ConnectionOptions::LocalShell { program, args, cwd }
                            if program.as_deref().is_some_and(|p| !p.trim().is_empty())
                                || !args.is_empty()
                                || cwd.as_deref().is_some_and(|p| !p.trim().is_empty())
                    ) && skipped.contains(c.name.as_str()) =>
                {
                    removed += 1;
                    false
                }
                _ => true,
            });
            self.connections = self.connections.saturating_sub(removed);
            for name in &programs {
                self.warn(
                    None,
                    format!(
                        "'{name}' was not imported: local shell entries with a custom program are not imported"
                    ),
                );
            }
        }
        if !credentials.is_empty() {
            let message = grouped(
                &credentials,
                "credential reference",
                "was reset to none: credentials do not travel between vaults and an import never inherits one",
                "were reset to none: credentials do not travel between vaults and an import never inherits one",
            );
            self.warn(None, message);
        }
        if !ignore_cert.is_empty() {
            let message = grouped(
                &ignore_cert,
                "RDP connection",
                "had \"do not verify the server certificate\": the option was turned off",
                "had \"do not verify the server certificate\": the option was turned off",
            );
            self.warn(None, message);
        }
        if !without_nla.is_empty() {
            let message = grouped(
                &without_nla,
                "RDP connection",
                "allowed connections without NLA: the option was turned off",
                "allowed connections without NLA: the option was turned off",
            );
            self.warn(None, message);
        }
        if !clipboard.is_empty() {
            let message = grouped(
                &clipboard,
                "RDP connection",
                "shared the clipboard with the remote desktop: the option was turned off",
                "shared the clipboard with the remote desktop: the option was turned off",
            );
            self.warn(None, message);
        }
    }

    pub(crate) fn into_preview(self) -> ImportPreview {
        ImportPreview {
            preview_id: Uuid::new_v4(),
            nodes: self.nodes,
            warnings: self.warnings,
        }
    }
}

/// "1 credential reference ('a') was ..." / "4 credential references ('a', 'b', 'c' and 1
/// more) were ...": one warning for a repetitive finding, naming a few entries.
fn grouped(names: &[String], what: &str, one: &str, many: &str) -> String {
    const SHOWN: usize = 3;
    let listed: Vec<String> = names.iter().take(SHOWN).map(|n| format!("'{n}'")).collect();
    let rest = names.len().saturating_sub(SHOWN);
    let list = match rest {
        0 => listed.join(", "),
        n => format!("{} and {n} more", listed.join(", ")),
    };
    if names.len() == 1 {
        format!("1 {what} ({list}) {one}")
    } else {
        format!("{} {what}s ({list}) {many}", names.len())
    }
}

fn set_name(node: &mut Node, name: String) {
    match node {
        Node::Folder(f) => f.name = name,
        Node::Connection(c) => c.name = name,
    }
}

/// `value` trimmed, or `fallback` trimmed when `value` is blank.
pub(crate) fn name_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.trim().to_owned()
    } else {
        value.to_owned()
    }
}

/// Parses a port: `Ok(None)` when blank, an error text when it is not a valid port.
pub(crate) fn parse_port(value: &str) -> Result<Option<u16>, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(0) | Err(_) => Err("not a port number"),
        Ok(port) => Ok(Some(port)),
    }
}

/// `None` when the port is the protocol default: the application then uses that default.
pub(crate) fn port_or_default(port: Option<u16>, default: u16) -> Option<u16> {
    port.filter(|p| *p != default)
}

/// The message of every password found in a source file. It never includes the value.
pub(crate) const PASSWORD_IGNORED: &str = "password ignored: passwords are never imported";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_parsing() {
        assert_eq!(parse_port(""), Ok(None));
        assert_eq!(parse_port(" 2222 "), Ok(Some(2222)));
        assert!(parse_port("0").is_err());
        assert!(parse_port("70000").is_err());
        assert!(parse_port("ssh").is_err());
        assert_eq!(port_or_default(Some(22), 22), None);
        assert_eq!(port_or_default(Some(2222), 22), Some(2222));
    }

    #[test]
    fn blank_names_fall_back() {
        assert_eq!(name_or("  ", " host "), "host");
        assert_eq!(name_or(" db ", "host"), "db");
    }

    #[test]
    fn folder_paths_are_reused() {
        let mut draft = Draft::new();
        let a = draft
            .folder_path(&["Prod".into(), "Web".into()], None)
            .expect("path")
            .expect("folder");
        let b = draft
            .folder_path(&["Prod".into(), "Web".into()], None)
            .expect("path again")
            .expect("folder");
        assert_eq!(a, b);
        assert_eq!(draft.nodes.len(), 2);
        assert_eq!(draft.folder_path(&[], None).expect("root"), None);
    }

    #[test]
    fn jump_cycles_are_cut() {
        let mut draft = Draft::new();
        let ssh = || ConnectionOptions::Ssh {
            jump_host: None,
            keepalive_secs: None,
        };
        let entry = |name: &str| Entry {
            name: name.into(),
            host: "h".into(),
            port: None,
            username: None,
            notes: String::new(),
            tags: vec![],
            options: ssh(),
        };
        let a = draft.add_connection(None, entry("a"), None).expect("a");
        let b = draft.add_connection(None, entry("b"), None).expect("b");
        draft.set_jump_host(a, b);
        draft.set_jump_host(b, a);
        draft.break_jump_cycles();
        let preview = draft.into_preview();
        let jump_of = |id: Uuid| {
            preview.nodes.iter().find_map(|n| match n {
                Node::Connection(c) if c.id == id => match c.options {
                    ConnectionOptions::Ssh { jump_host, .. } => Some(jump_host),
                    _ => None,
                },
                _ => None,
            })
        };
        // Exactly one edge of the loop is removed; the other one stays.
        let remaining = [jump_of(a), jump_of(b)]
            .iter()
            .filter(|j| matches!(j, Some(Some(_))))
            .count();
        assert_eq!(remaining, 1);
        assert_eq!(preview.warnings.len(), 1);
    }
}
