//! Apply (preview into a `DocumentStore`) and export (JSON round trip, `.rdp`, atomic write).

mod common;

use std::collections::BTreeMap;

use common::*;
use portolano_core_model::{
    Connection, ConnectionOptions, CredentialRef, Document, DocumentStore, Folder, Node,
};
use portolano_importers::{
    apply, export, preview, select, ExportFormat, ImportError, ImportFormat, ImportPreview,
};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn store_in(dir: &TempDir) -> DocumentStore {
    DocumentStore::new(dir.path("connections.json"))
}

fn folder(id: u128, parent: Option<u128>, name: &str) -> Node {
    Node::Folder(Folder {
        id: u(id),
        parent_id: parent.map(u),
        name: name.into(),
        credential: CredentialRef::Inherit,
    })
}

fn connection_node(id: u128, parent: Option<u128>, name: &str, options: ConnectionOptions) -> Node {
    Node::Connection(Connection {
        id: u(id),
        parent_id: parent.map(u),
        name: name.into(),
        host: format!("{name}.example.org"),
        port: None,
        credential: CredentialRef::None,
        username: None,
        tags: vec![],
        notes: String::new(),
        options,
    })
}

fn ssh() -> ConnectionOptions {
    ConnectionOptions::Ssh {
        jump_host: None,
        keepalive_secs: None,
    }
}

/// Document as the test sees it: a loaded store, cloned.
fn snapshot(store: &mut DocumentStore) -> Document {
    store.ensure_loaded().expect("load");
    store.document().clone()
}

// ------------------------------------------------------------------------- apply

#[test]
fn apply_puts_nodes_under_the_target_and_persists_them() {
    let dir = TempDir::new("apply-target");
    let mut store = store_in(&dir);
    store
        .upsert_node(folder(100, None, "Target"))
        .expect("target");

    let p = preview(ImportFormat::SshConfig, &fixture("ssh/config.conf")).expect("preview");
    let count = apply(&mut store, Some(u(100)), &p).expect("apply");
    assert_eq!(count as usize, p.nodes.len());

    let doc = snapshot(&mut store);
    let roots: Vec<&str> = doc
        .nodes
        .iter()
        .filter(|n| n.parent_id() == Some(u(100)))
        .map(Node::name)
        .collect();
    assert_eq!(roots.len(), 8, "every imported root is under the target");

    // The jump host of db01 points at the bastion node of the same import.
    let bastion = id_of(&p, "bastion");
    let db01 = doc
        .nodes
        .iter()
        .find(|n| n.name() == "db01")
        .expect("db01 stored");
    match db01 {
        Node::Connection(c) => assert!(matches!(
            c.options,
            ConnectionOptions::Ssh { jump_host: Some(j), .. } if j == bastion
        )),
        Node::Folder(_) => panic!("expected a connection"),
    }

    // Persisted to disk: a fresh store reads the same document.
    let reloaded = DocumentStore::load(dir.path("connections.json")).expect("reload");
    assert_eq!(reloaded.document(), &doc);
}

#[test]
fn apply_at_root_and_second_import_gets_new_ids() {
    let dir = TempDir::new("apply-root");
    let mut store = store_in(&dir);
    let first = preview(ImportFormat::SshConfig, &fixture("ssh/config.conf")).expect("first");
    let second = preview(ImportFormat::SshConfig, &fixture("ssh/config.conf")).expect("second");
    apply(&mut store, None, &first).expect("first apply");
    apply(&mut store, None, &second).expect("second apply, no id clash");
    let doc = snapshot(&mut store);
    assert_eq!(doc.nodes.len(), first.nodes.len() * 2);
    assert!(doc.nodes.iter().all(|n| n.parent_id().is_none()));
}

#[test]
fn apply_refuses_a_missing_target_and_leaves_the_document_unchanged() {
    let dir = TempDir::new("apply-missing");
    let mut store = store_in(&dir);
    store
        .upsert_node(folder(1, None, "Existing"))
        .expect("existing");
    let before = snapshot(&mut store);

    let p = preview(ImportFormat::Csv, &fixture("csv/connections.csv")).expect("preview");
    let err = apply(&mut store, Some(u(77)), &p).expect_err("missing target");
    assert!(matches!(err, ImportError::NotFound(_)));
    assert_eq!(snapshot(&mut store), before);
}

#[test]
fn apply_refuses_a_connection_as_target() {
    let dir = TempDir::new("apply-not-folder");
    let mut store = store_in(&dir);
    store
        .upsert_node(connection_node(5, None, "plain", ssh()))
        .expect("connection");
    let p = preview(ImportFormat::Csv, &fixture("csv/connections.csv")).expect("preview");
    let err = apply(&mut store, Some(u(5)), &p).expect_err("not a folder");
    assert!(matches!(err, ImportError::InvalidInput(_)));
}

#[test]
fn a_failing_insert_rolls_the_whole_import_back() {
    let dir = TempDir::new("apply-rollback");
    let mut store = store_in(&dir);
    store
        .upsert_node(folder(1, None, "Existing"))
        .expect("existing");
    let before = snapshot(&mut store);

    // The second connection is a child of the first one, which is not a folder: the store
    // refuses that insert after the first node was already written.
    let preview = ImportPreview {
        preview_id: u(900),
        nodes: vec![
            connection_node(10, None, "first", ssh()),
            connection_node(11, Some(10), "child-of-connection", ssh()),
        ],
        warnings: vec![],
    };
    let err = apply(&mut store, None, &preview).expect_err("refused insert");
    assert!(matches!(err, ImportError::InvalidInput(_)), "{err}");
    assert_eq!(snapshot(&mut store), before, "the document is unchanged");
    let reloaded = DocumentStore::load(dir.path("connections.json")).expect("reload");
    assert_eq!(reloaded.document(), &before, "and so is the file");
}

#[test]
fn a_failing_import_leaves_the_files_byte_identical() {
    let dir = TempDir::new("apply-bytes");
    let mut store = store_in(&dir);
    store
        .upsert_node(folder(1, None, "Existing"))
        .expect("existing");
    store
        .upsert_node(connection_node(2, Some(1), "kept", ssh()))
        .expect("second save creates the backup");
    let main = dir.path("connections.json");
    let backup = dir.path("connections.json.bak");
    let main_before = std::fs::read(&main).expect("read main");
    let backup_before = std::fs::read(&backup).expect("read backup");

    // Fails at the whole-document validation (empty name), after thousands of valid nodes.
    let mut nodes: Vec<Node> = (0..500u128)
        .map(|i| connection_node(1000 + i, None, &format!("n{i}"), ssh()))
        .collect();
    nodes.push(connection_node(5000, None, "  ", ssh()));
    let preview = ImportPreview {
        preview_id: u(901),
        nodes,
        warnings: vec![],
    };
    let err = apply(&mut store, None, &preview).expect_err("invalid name");
    assert!(matches!(err, ImportError::InvalidInput(_)), "{err}");
    assert_eq!(std::fs::read(&main).expect("read main"), main_before);
    assert_eq!(std::fs::read(&backup).expect("read backup"), backup_before);
}

#[test]
fn two_thousand_nodes_import_quickly_and_persist() {
    let dir = TempDir::new("apply-speed");
    let mut store = store_in(&dir);
    let mut nodes = vec![folder(1, None, "Bulk")];
    for i in 0..2000u128 {
        nodes.push(connection_node(
            10 + i,
            Some(1),
            &format!("host-{i}"),
            ssh(),
        ));
    }
    let preview = ImportPreview {
        preview_id: u(902),
        nodes,
        warnings: vec![],
    };
    let started = std::time::Instant::now();
    let count = apply(&mut store, None, &preview).expect("import");
    let elapsed = started.elapsed();
    assert_eq!(count, 2001);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "importing 2001 nodes took {elapsed:?}"
    );
    let reloaded = DocumentStore::load(dir.path("connections.json")).expect("reload");
    assert_eq!(reloaded.document().nodes.len(), 2001);
}

#[test]
fn apply_refuses_cycles_orphans_and_id_clashes() {
    let dir = TempDir::new("apply-invalid");
    let mut store = store_in(&dir);
    store
        .upsert_node(folder(1, None, "Existing"))
        .expect("existing");
    let before = snapshot(&mut store);

    let cycle = ImportPreview {
        preview_id: u(901),
        nodes: vec![folder(20, Some(21), "a"), folder(21, Some(20), "b")],
        warnings: vec![],
    };
    assert!(matches!(
        apply(&mut store, None, &cycle),
        Err(ImportError::InvalidInput(_))
    ));

    let orphan = ImportPreview {
        preview_id: u(902),
        nodes: vec![folder(30, Some(99), "orphan")],
        warnings: vec![],
    };
    assert!(matches!(
        apply(&mut store, None, &orphan),
        Err(ImportError::InvalidInput(_))
    ));

    let clash = ImportPreview {
        preview_id: u(903),
        nodes: vec![folder(1, None, "same id as existing")],
        warnings: vec![],
    };
    assert!(matches!(
        apply(&mut store, None, &clash),
        Err(ImportError::InvalidInput(_))
    ));
    assert_eq!(snapshot(&mut store), before);
}

// ------------------------------------------------------------------------ export

/// Tree shape without ids and credential references: names, parents by name, options with
/// jump hosts by name. Credentials are compared as "reference kept" or "none".
fn shape(doc: &Document) -> Vec<serde_json::Value> {
    let names: BTreeMap<Uuid, String> = doc
        .nodes
        .iter()
        .map(|n| (n.id(), n.name().to_owned()))
        .collect();
    let mut out: Vec<serde_json::Value> = doc
        .nodes
        .iter()
        .map(|node| {
            let credential = |c: CredentialRef| match c {
                CredentialRef::Id { .. } => "id",
                CredentialRef::Inherit => "inherit",
                CredentialRef::None => "none",
            };
            match node {
                Node::Folder(f) => serde_json::json!({
                    "kind": "folder",
                    "name": f.name,
                    "parent": f.parent_id.and_then(|p| names.get(&p)),
                    "credential": credential(f.credential),
                }),
                Node::Connection(c) => {
                    let jump = match &c.options {
                        ConnectionOptions::Ssh { jump_host, .. } => {
                            jump_host.and_then(|j| names.get(&j).cloned())
                        }
                        _ => None,
                    };
                    serde_json::json!({
                        "kind": "connection",
                        "name": c.name,
                        "parent": c.parent_id.and_then(|p| names.get(&p)),
                        "host": c.host,
                        "port": c.port,
                        "username": c.username,
                        "tags": c.tags,
                        "notes": c.notes,
                        "options": options_without_jump_id(&c.options),
                        "jump": jump,
                        "credential": credential(c.credential),
                    })
                }
            }
        })
        .collect();
    out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_owned());
    out
}

/// The options as JSON, with the jump host id removed (the jump is compared by name).
fn options_without_jump_id(options: &ConnectionOptions) -> serde_json::Value {
    let mut value = serde_json::to_value(options).expect("options");
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("jumpHost") {
            object.insert("jumpHost".into(), serde_json::Value::Null);
        }
    }
    value
}

fn rich_document() -> Document {
    let mut doc = Document::default();
    let nodes = vec![
        folder(1, None, "Prod"),
        Node::Folder(Folder {
            id: u(2),
            parent_id: Some(u(1)),
            name: "Secret vault folder".into(),
            credential: CredentialRef::Id { id: u(500) },
        }),
        Node::Connection(Connection {
            id: u(3),
            parent_id: Some(u(1)),
            name: "bastion".into(),
            host: "bastion.example.org".into(),
            port: Some(2200),
            credential: CredentialRef::Id { id: u(501) },
            username: Some("alice".into()),
            tags: vec!["prod".into(), "edge".into()],
            notes: "jump box".into(),
            options: ConnectionOptions::Ssh {
                jump_host: None,
                keepalive_secs: Some(30),
            },
        }),
        Node::Connection(Connection {
            id: u(4),
            parent_id: Some(u(2)),
            name: "db01".into(),
            host: "db01.example.org".into(),
            port: None,
            credential: CredentialRef::Inherit,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Ssh {
                jump_host: Some(u(3)),
                keepalive_secs: None,
            },
        }),
        connection_node(
            5,
            None,
            "desk01",
            ConnectionOptions::Rdp {
                width: Some(1280),
                height: Some(720),
                domain: Some("EXAMPLE".into()),
                ignore_cert: false,
                allow_without_nla: false,
                share_clipboard: false,
                mac_shortcuts: false,
            },
        ),
    ];
    doc.nodes = nodes;
    doc
}

#[test]
fn json_round_trip_keeps_the_tree_but_not_ids_or_credentials() {
    let dir = TempDir::new("roundtrip");
    let mut source = store_in(&dir);
    for node in rich_document().nodes {
        source.upsert_node(node).expect("source node");
    }
    let source_doc = snapshot(&mut source);

    let file = dir.path("export.json");
    export(
        ExportFormat::Json,
        &file,
        &select(&source_doc, None).expect("select"),
    )
    .expect("export");

    let parsed = preview(ImportFormat::Json, &file).expect("import the export");
    let target_dir = TempDir::new("roundtrip-target");
    let mut target = store_in(&target_dir);
    apply(&mut target, None, &parsed).expect("apply");
    let target_doc = snapshot(&mut target);

    // Same shape, except that every credential reference becomes "none": an import never
    // brings an id and never inherits (changed: the test used to expect "inherit" to survive).
    let mut expected = shape(&source_doc);
    for entry in &mut expected {
        if entry["credential"] == "id" || entry["credential"] == "inherit" {
            entry["credential"] = serde_json::json!("none");
        }
    }
    assert_eq!(shape(&target_doc), expected);
    // Ids are new.
    let source_ids: Vec<Uuid> = source_doc.nodes.iter().map(Node::id).collect();
    assert!(target_doc
        .nodes
        .iter()
        .all(|n| !source_ids.contains(&n.id())));
}

#[test]
fn json_export_of_a_subtree_is_self_contained() {
    let doc = rich_document();
    // db01 has bastion as jump host; bastion is outside the selection.
    let selected = select(&doc, Some(&[u(4)])).expect("select db01");
    assert_eq!(selected.len(), 1);
    let dir = TempDir::new("subtree");
    let file = dir.path("db01.json");
    export(ExportFormat::Json, &file, &selected).expect("export");
    let text = std::fs::read_to_string(&file).expect("read");
    assert!(text.contains("\"jumpHost\": null"));
    assert!(!text.contains(&u(3).to_string()));
    // The folder of db01 is not selected, so the connection becomes a root.
    assert!(text.contains("\"parentId\": null"));
}

#[test]
fn selecting_a_folder_includes_its_descendants() {
    let doc = rich_document();
    let selected = select(&doc, Some(&[u(1)])).expect("select Prod");
    let names: Vec<&str> = selected.iter().map(Node::name).collect();
    assert_eq!(
        names,
        vec!["Prod", "Secret vault folder", "bastion", "db01"]
    );
    assert!(matches!(
        select(&doc, Some(&[u(999)])),
        Err(ImportError::NotFound(_))
    ));
    assert_eq!(select(&doc, None).expect("all").len(), doc.nodes.len());
}

#[test]
fn json_export_never_contains_a_password_key_or_credential_ids() {
    let dir = TempDir::new("no-secrets");
    let file = dir.path("all.json");
    export(ExportFormat::Json, &file, &rich_document().nodes).expect("export");
    let text = std::fs::read_to_string(&file).expect("read");
    assert!(!text.contains("\"password\""));
    assert!(!text.contains(&u(500).to_string()));
    assert!(!text.contains(&u(501).to_string()));
    assert!(text.contains("\"format\": \"portolano-export\""));
}

fn decode_utf16le_with_bom(bytes: &[u8]) -> String {
    assert_eq!(&bytes[..2], &[0xFF, 0xFE], "UTF-16LE byte order mark");
    let units: Vec<u16> = bytes[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes(*p))
        .collect();
    String::from_utf16(&units).expect("valid UTF-16")
}

#[test]
fn rdp_export_has_the_exact_content_and_no_password() {
    let dir = TempDir::new("rdp-export");
    let node = connection_node(
        7,
        None,
        "desk01",
        ConnectionOptions::Rdp {
            width: Some(1920),
            height: Some(1080),
            domain: Some("EXAMPLE".into()),
            ignore_cert: true,
            allow_without_nla: false,
            share_clipboard: false,
            mac_shortcuts: false,
        },
    );
    let Node::Connection(mut conn) = node else {
        panic!("connection expected")
    };
    conn.host = "desk01.example.org".into();
    conn.port = Some(3390);
    conn.username = Some("ops".into());
    let nodes = vec![Node::Connection(conn)];

    let file = dir.path("desk01.rdp");
    export(ExportFormat::Rdp, &file, &nodes).expect("export");
    let text = decode_utf16le_with_bom(&std::fs::read(&file).expect("read"));
    assert_eq!(
        text,
        "full address:s:desk01.example.org:3390\r\n\
         username:s:ops\r\n\
         domain:s:EXAMPLE\r\n\
         desktopwidth:i:1920\r\n\
         desktopheight:i:1080\r\n"
    );

    let back = preview(ImportFormat::Rdp, &file).expect("re-import");
    let c = connection(&back, "desk01");
    assert_eq!(c.host, "desk01.example.org");
    assert_eq!(c.port, Some(3390));
    assert_eq!(c.username.as_deref(), Some("ops"));
}

#[test]
fn rdp_export_omits_unset_fields_and_defaults() {
    let dir = TempDir::new("rdp-minimal");
    let nodes = vec![connection_node(
        8,
        None,
        "plain",
        ConnectionOptions::Rdp {
            width: None,
            height: None,
            domain: None,
            ignore_cert: false,
            allow_without_nla: false,
            share_clipboard: false,
            mac_shortcuts: false,
        },
    )];
    let file = dir.path("plain.rdp");
    export(ExportFormat::Rdp, &file, &nodes).expect("export");
    let text = decode_utf16le_with_bom(&std::fs::read(&file).expect("read"));
    assert_eq!(text, "full address:s:plain.example.org\r\n");
}

#[test]
fn rdp_export_refuses_what_it_cannot_write() {
    let dir = TempDir::new("rdp-refuse");
    let file = dir.path("refused.rdp");

    let ssh_node = vec![connection_node(9, None, "ssh01", ssh())];
    assert!(matches!(
        export(ExportFormat::Rdp, &file, &ssh_node),
        Err(ImportError::InvalidInput(_))
    ));

    let two = vec![
        connection_node(
            10,
            None,
            "a",
            ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: false,
                allow_without_nla: false,
                share_clipboard: false,
                mac_shortcuts: false,
            },
        ),
        connection_node(
            11,
            None,
            "b",
            ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: false,
                allow_without_nla: false,
                share_clipboard: false,
                mac_shortcuts: false,
            },
        ),
    ];
    assert!(matches!(
        export(ExportFormat::Rdp, &file, &two),
        Err(ImportError::InvalidInput(_))
    ));

    let injected = vec![Node::Connection(Connection {
        id: u(12),
        parent_id: None,
        name: "x".into(),
        host: "good.example.org\r\npassword 51:b:not-a-real-password".into(),
        port: None,
        credential: CredentialRef::None,
        username: None,
        tags: vec![],
        notes: String::new(),
        options: ConnectionOptions::Rdp {
            width: None,
            height: None,
            domain: None,
            ignore_cert: false,
            allow_without_nla: false,
            share_clipboard: false,
            mac_shortcuts: false,
        },
    })];
    assert!(matches!(
        export(ExportFormat::Rdp, &file, &injected),
        Err(ImportError::InvalidInput(_))
    ));
    assert!(
        !file.exists(),
        "nothing is written when the export is refused"
    );
}

#[test]
fn export_overwrites_atomically_and_leaves_no_temporary_file() {
    let dir = TempDir::new("atomic");
    let file = dir.path("out.json");
    std::fs::write(&file, b"old content").expect("old file");
    export(ExportFormat::Json, &file, &rich_document().nodes).expect("export over old file");
    let text = std::fs::read_to_string(&file).expect("read");
    assert!(text.starts_with('{') && !text.contains("old content"));
    let entries: Vec<String> = std::fs::read_dir(dir.path(""))
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["out.json".to_owned()],
        "no temporary file is left"
    );
}

#[test]
fn export_into_a_missing_folder_is_an_io_error() {
    let dir = TempDir::new("missing-folder");
    let file = dir.path("no-such-folder/out.json");
    let err = export(ExportFormat::Json, &file, &[]).expect_err("no folder");
    assert!(matches!(err, ImportError::Io(_)));
}
