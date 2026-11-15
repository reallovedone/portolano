//! Connection document (§4.1). See the crate docs for the JSON convention.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current on-disk schema version.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    pub nodes: Vec<Node>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            nodes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Node {
    Folder(Folder),
    Connection(Connection),
}

impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Folder(f) => f.id,
            Self::Connection(c) => c.id,
        }
    }

    pub fn parent_id(&self) -> Option<Uuid> {
        match self {
            Self::Folder(f) => f.parent_id,
            Self::Connection(c) => c.parent_id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Folder(f) => &f.name,
            Self::Connection(c) => &c.name,
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, Self::Folder(_))
    }

    pub fn set_parent_id(&mut self, parent_id: Option<Uuid>) {
        match self {
            Self::Folder(f) => f.parent_id = parent_id,
            Self::Connection(c) => c.parent_id = parent_id,
        }
    }
}

impl Document {
    /// The node with `id`, if any (linear scan).
    pub fn node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id() == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub name: String,
    /// Inherited by the children when they use `CredentialRef::Inherit`.
    pub credential: CredentialRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: Uuid,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub credential: CredentialRef,
    /// Optional, non-secret user name used to pre-fill the credential prompt shown when the
    /// connection has no credential (WP11). A missing field in an old document means `None`.
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub options: ConnectionOptions,
}

/// Whether `host` can be written to a line-based store (known hosts, pinned certificates)
/// without breaking its format: no whitespace and no control characters. A real host name or
/// address never has any.
pub fn host_is_storable(host: &str) -> bool {
    !host.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `Id` is a struct variant (`{"type":"id","id":"..."}`) because internally tagged enums
/// cannot hold a bare string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CredentialRef {
    Inherit,
    None,
    Id { id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConnectionOptions {
    LocalShell {
        #[serde(default)]
        program: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    Ssh {
        /// Another connection used as jump host.
        #[serde(default)]
        jump_host: Option<Uuid>,
        #[serde(default)]
        keepalive_secs: Option<u32>,
    },
    Rdp {
        #[serde(default)]
        width: Option<u16>,
        #[serde(default)]
        height: Option<u16>,
        #[serde(default)]
        domain: Option<String>,
        #[serde(default)]
        ignore_cert: bool,
        /// Accept a server that only offers TLS-level security (no NLA) although a credential
        /// is sent. Off unless the user chose it for this connection.
        #[serde(default)]
        allow_without_nla: bool,
    },
    Vnc {
        #[serde(default)]
        view_only: bool,
    },
    Web {
        url: String,
        #[serde(default)]
        isolated_profile: bool,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn document_json_shape() {
        let doc = Document::default();
        assert_eq!(
            serde_json::to_value(&doc).expect("serialize"),
            json!({"schemaVersion": 1, "nodes": []})
        );
    }

    #[test]
    fn folder_json_shape() {
        let node = Node::Folder(Folder {
            id: id(1),
            parent_id: None,
            name: "Prod".into(),
            credential: CredentialRef::Id { id: id(9) },
        });
        let expected = json!({
            "type": "folder",
            "id": "00000000-0000-0000-0000-000000000001",
            "parentId": null,
            "name": "Prod",
            "credential": {"type": "id", "id": "00000000-0000-0000-0000-000000000009"}
        });
        assert_eq!(serde_json::to_value(&node).expect("serialize"), expected);
        assert_eq!(
            serde_json::from_value::<Node>(expected).expect("parse"),
            node
        );
    }

    #[test]
    fn connection_json_shape() {
        let node = Node::Connection(Connection {
            id: id(2),
            parent_id: Some(id(1)),
            name: "web01".into(),
            host: "web01.example.net".into(),
            port: Some(22),
            credential: CredentialRef::Inherit,
            username: Some("deploy".into()),
            tags: vec!["prod".into()],
            notes: String::new(),
            options: ConnectionOptions::Ssh {
                jump_host: None,
                keepalive_secs: Some(30),
            },
        });
        let expected = json!({
            "type": "connection",
            "id": "00000000-0000-0000-0000-000000000002",
            "parentId": "00000000-0000-0000-0000-000000000001",
            "name": "web01",
            "host": "web01.example.net",
            "port": 22,
            "credential": {"type": "inherit"},
            "username": "deploy",
            "tags": ["prod"],
            "notes": "",
            "options": {"type": "ssh", "jumpHost": null, "keepaliveSecs": 30}
        });
        assert_eq!(serde_json::to_value(&node).expect("serialize"), expected);
        assert_eq!(
            serde_json::from_value::<Node>(expected).expect("parse"),
            node
        );
    }

    #[test]
    fn username_is_optional_in_old_documents_and_round_trips_when_set() {
        // A document written before WP11 has no `username`: it loads as `None`.
        let old = json!({
            "type": "connection",
            "id": "00000000-0000-0000-0000-000000000002",
            "parentId": null,
            "name": "old",
            "host": "h",
            "port": null,
            "credential": {"type": "none"},
            "tags": [],
            "notes": "",
            "options": {"type": "ssh", "jumpHost": null, "keepaliveSecs": null}
        });
        let Node::Connection(parsed) = serde_json::from_value::<Node>(old).expect("parse old")
        else {
            panic!("expected a connection");
        };
        assert_eq!(parsed.username, None);
        // Serialized again it carries an explicit `null` (the crate's JSON convention).
        let again = serde_json::to_value(Node::Connection(parsed)).expect("serialize");
        assert_eq!(again["username"], serde_json::Value::Null);

        // With the field set it round-trips through a whole document.
        let doc = Document {
            nodes: vec![Node::Connection(Connection {
                id: id(5),
                parent_id: None,
                name: "new".into(),
                host: "h".into(),
                port: None,
                credential: CredentialRef::None,
                username: Some("alice".into()),
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::Vnc { view_only: false },
            })],
            ..Document::default()
        };
        let text = serde_json::to_string(&doc).expect("serialize");
        assert!(text.contains(r#""username":"alice""#));
        assert_eq!(serde_json::from_str::<Document>(&text).expect("parse"), doc);
    }

    #[test]
    fn credential_ref_json_shape() {
        assert_eq!(
            serde_json::to_value(CredentialRef::Inherit).expect("serialize"),
            json!({"type": "inherit"})
        );
        assert_eq!(
            serde_json::to_value(CredentialRef::None).expect("serialize"),
            json!({"type": "none"})
        );
    }

    #[test]
    fn options_json_shapes() {
        let cases = [
            (
                ConnectionOptions::LocalShell {
                    program: Some("zsh".into()),
                    args: vec!["-l".into()],
                    cwd: None,
                },
                json!({"type": "localShell", "program": "zsh", "args": ["-l"], "cwd": null}),
            ),
            (
                ConnectionOptions::Rdp {
                    width: Some(1920),
                    height: Some(1080),
                    domain: Some("CORP".into()),
                    ignore_cert: false,
                    allow_without_nla: false,
                },
                json!({"type": "rdp", "width": 1920, "height": 1080, "domain": "CORP", "ignoreCert": false, "allowWithoutNla": false}),
            ),
            (
                ConnectionOptions::Vnc { view_only: true },
                json!({"type": "vnc", "viewOnly": true}),
            ),
            (
                ConnectionOptions::Web {
                    url: "https://example.net".into(),
                    isolated_profile: true,
                },
                json!({"type": "web", "url": "https://example.net", "isolatedProfile": true}),
            ),
        ];
        for (options, expected) in cases {
            assert_eq!(serde_json::to_value(&options).expect("serialize"), expected);
            assert_eq!(
                serde_json::from_value::<ConnectionOptions>(expected).expect("parse"),
                options
            );
        }
    }

    #[test]
    fn rdp_document_without_the_nla_option_still_loads() {
        // A connection saved before the option existed.
        let parsed: ConnectionOptions = serde_json::from_value(
            json!({"type": "rdp", "width": null, "height": null, "domain": null, "ignoreCert": true}),
        )
        .expect("parse");
        assert_eq!(
            parsed,
            ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: true,
                allow_without_nla: false,
            }
        );
    }

    #[test]
    fn missing_optional_fields_are_accepted() {
        let parsed: ConnectionOptions =
            serde_json::from_value(json!({"type": "ssh"})).expect("parse");
        assert_eq!(
            parsed,
            ConnectionOptions::Ssh {
                jump_host: None,
                keepalive_secs: None
            }
        );
    }
}
