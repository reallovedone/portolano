//! Import preview for every format: happy paths, skipped protocols, dropped passwords,
//! malformed and limit cases. All inputs are synthetic fixtures.

mod common;

use common::*;
use portolano_core_model::{ConnectionOptions, CredentialRef, Node};
use portolano_importers::{preview, ImportError, ImportFormat, ImportPreview};

fn parse(format: ImportFormat, file: &str) -> ImportPreview {
    preview(format, &fixture(file)).expect("preview")
}

fn write_bytes(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn ssh_jump(preview: &ImportPreview, name: &str) -> Option<uuid::Uuid> {
    match &connection(preview, name).options {
        ConnectionOptions::Ssh { jump_host, .. } => *jump_host,
        _ => None,
    }
}

// ---------------------------------------------------------------- OpenSSH client config

#[test]
fn ssh_config_happy_path() {
    let p = parse(ImportFormat::SshConfig, "ssh/config.conf");
    assert_eq!(
        connection_names(&p),
        vec!["bastion", "db01", "web01", "app01", "app02", "edge", "gw-one", "gw-two"]
    );
    let bastion = connection(&p, "bastion");
    assert_eq!(bastion.host, "bastion.example.org");
    assert_eq!(bastion.port, None, "port 22 is the default and is omitted");
    assert_eq!(bastion.username.as_deref(), Some("alice"));
    assert_eq!(bastion.credential, CredentialRef::None);
    assert!(matches!(
        bastion.options,
        ConnectionOptions::Ssh {
            jump_host: None,
            ..
        }
    ));

    assert_eq!(ssh_jump(&p, "db01"), Some(bastion.id));
    assert_eq!(connection(&p, "db01").host, "db01.example.org");

    let web = connection(&p, "web01");
    assert_eq!(web.host, "10.0.0.5", "HostName is used as the host");
    assert_eq!(web.port, Some(2222));
    assert_eq!(web.username.as_deref(), Some("web admin"));

    // Two aliases on one Host line are two connections; the jump host is resolved inside the file
    // even though its user and port are not applied (warned below).
    assert_eq!(connection(&p, "app01").host, "app.example.org");
    assert_eq!(ssh_jump(&p, "app01"), Some(bastion.id));
    assert_eq!(ssh_jump(&p, "app02"), Some(bastion.id));
    assert_eq!(connection(&p, "app01").username, None);

    // ProxyJump with several hops is not supported.
    assert_eq!(ssh_jump(&p, "edge"), None);

    // gw-one and gw-two point at each other: one of the two jumps is removed.
    let surviving = [ssh_jump(&p, "gw-one"), ssh_jump(&p, "gw-two")]
        .iter()
        .flatten()
        .count();
    assert_eq!(surviving, 1);

    // Wildcard blocks, Match and Include apply nothing.
    assert_eq!(warnings_containing(&p, "wildcard Host patterns"), 1);
    assert_eq!(warnings_containing(&p, "Match blocks are ignored"), 1);
    assert_eq!(warnings_containing(&p, "Include ignored"), 1);
    assert_eq!(warnings_containing(&p, "IdentityFile ignored"), 1);
    assert_eq!(warnings_containing(&p, "user and port of ProxyJump"), 2);
    assert_eq!(warnings_containing(&p, "multi-hop ProxyJump"), 1);
    assert_eq!(warnings_containing(&p, "loop back"), 1);
    let all = json(&p);
    assert!(!all.contains("ignored.example.org"));
    assert!(!all.contains("ignored-match-user"));
    assert!(!all.contains("ops-default"));
    assert!(!all.contains("conf.d"));
    assert!(p.nodes.iter().all(|n| n.parent_id().is_none()));
}

// ------------------------------------------------------------------------------- .rdp

#[test]
fn rdp_utf8_file_with_port_and_dropped_password() {
    let p = parse(ImportFormat::Rdp, "rdp/desk01.rdp");
    let c = connection(&p, "desk01");
    assert_eq!(c.host, "10.0.0.5");
    assert_eq!(c.port, Some(3390));
    assert_eq!(c.username.as_deref(), Some("ops"));
    assert_eq!(c.credential, CredentialRef::None);
    match &c.options {
        ConnectionOptions::Rdp {
            width,
            height,
            domain,
            ignore_cert,
            allow_without_nla,
            share_clipboard,
            mac_shortcuts,
        } => {
            assert_eq!(*width, Some(1920));
            assert!(!allow_without_nla);
            assert!(!share_clipboard);
            assert!(!mac_shortcuts);
            assert_eq!(*height, Some(1080));
            assert_eq!(domain.as_deref(), Some("EXAMPLE"));
            assert!(!ignore_cert);
        }
        other => panic!("expected rdp options, got {other:?}"),
    }
    assert_eq!(warnings_containing(&p, "password ignored"), 1);
    fake_password_absent(&p);
}

#[test]
fn rdp_utf16le_with_bom_default_port_is_omitted() {
    let p = parse(ImportFormat::Rdp, "rdp/win-host.rdp");
    let c = connection(&p, "win-host");
    assert_eq!(c.host, "win01.example.org");
    assert_eq!(c.port, None, "server port 3389 is the default");
    assert_eq!(c.username.as_deref(), Some("alice"));
    assert_eq!(warnings_containing(&p, "password ignored"), 1);
    fake_password_absent(&p);
}

#[test]
fn rdp_without_full_address_is_invalid() {
    let err = preview(ImportFormat::Rdp, &fixture("rdp/no-address.rdp")).expect_err("no address");
    assert!(matches!(err, ImportError::InvalidInput(_)));
}

// ------------------------------------------------------------------------ MobaXterm

#[test]
fn mobaxterm_folders_protocols_and_skips() {
    let p = parse(ImportFormat::Mobaxterm, "mobaxterm/sessions.mxtsessions");
    assert_eq!(
        connection_names(&p),
        vec![
            "gateway",
            "web-ssh",
            "web-rdp",
            "web-vnc",
            "edge01.example.org (.\\alice)",
            "desk01.example.org",
            "password-vault"
        ]
    );
    let gateway = connection(&p, "gateway");
    assert_eq!(gateway.host, "gw01.example.org");
    assert_eq!(gateway.port, None);
    assert_eq!(gateway.username.as_deref(), Some("ops"));
    assert_eq!(parent_name(&p, "gateway"), None);

    assert_eq!(parent_name(&p, "web-ssh").as_deref(), Some("Web"));
    assert_eq!(parent_name(&p, "Web").as_deref(), Some("Prod"));
    assert_eq!(parent_name(&p, "Prod"), None);

    let web_ssh = connection(&p, "web-ssh");
    assert_eq!(web_ssh.port, None, "empty port means the default");
    assert_eq!(web_ssh.username.as_deref(), Some("deploy"));

    let web_rdp = connection(&p, "web-rdp");
    assert_eq!(web_rdp.port, Some(3390));
    assert!(matches!(web_rdp.options, ConnectionOptions::Rdp { .. }));
    let web_vnc = connection(&p, "web-vnc");
    assert_eq!(web_vnc.port, Some(5901));
    assert!(matches!(web_vnc.options, ConnectionOptions::Vnc { .. }));

    assert_eq!(warnings_containing(&p, "Telnet is not supported"), 1);
    assert_eq!(
        warnings_containing(&p, "session type 7 is not supported"),
        1
    );
    assert_eq!(warnings_containing(&p, "no host"), 1);
    assert_eq!(warnings_containing(&p, "password ignored"), 1);
    assert_eq!(warnings_containing(&p, "password section ignored"), 1);
    assert!(
        !json(&p).contains("misc.example.org"),
        "other sections are ignored"
    );
    assert!(!json(&p).contains("switch01"));
    fake_password_absent(&p);
}

/// Behaviours that only show with the layout of a real export (CRLF, 7 `#` segments, long
/// RDP bodies, nested folders, domain users, credential-manager references).
#[test]
fn mobaxterm_real_layout_users_domains_names_and_nesting() {
    let p = parse(ImportFormat::Mobaxterm, "mobaxterm/sessions.mxtsessions");

    // `DOMAIN\user` on RDP: user and domain are separate; the explicit default port is dropped.
    let web_rdp = connection(&p, "web-rdp");
    assert_eq!(web_rdp.username.as_deref(), Some("alice"));
    match &web_rdp.options {
        ConnectionOptions::Rdp { domain, .. } => assert_eq!(domain.as_deref(), Some("EXAMPLE")),
        other => panic!("not RDP: {other:?}"),
    }

    // `.\user` is a local account; names with parentheses, brackets and a backslash survive.
    let local = connection(&p, "edge01.example.org (.\\alice)");
    assert_eq!(local.host, "edge01.example.org");
    assert_eq!(local.port, None, "3389 is the RDP default");
    assert_eq!(local.username.as_deref(), Some("alice"));
    assert!(matches!(
        &local.options,
        ConnectionOptions::Rdp { domain: Some(d), .. } if d == "."
    ));

    // `[label]` is a reference into MobaXterm's credential manager, not a user name.
    let referenced = connection(&p, "desk01.example.org");
    assert_eq!(referenced.username, None);
    assert_eq!(referenced.notes, "MobaXterm credential: Team admins");
    assert!(matches!(
        &referenced.options,
        ConnectionOptions::Rdp { domain: None, .. }
    ));
    assert_eq!(warnings_containing(&p, "stored credential ignored"), 1);

    // A session whose name merely contains "password" is still a session.
    let vault = connection(&p, "password-vault");
    assert_eq!(vault.host, "vault01.example.org");
    assert_eq!(vault.username.as_deref(), Some("ops"));

    // The trailing font/layout segments never leak into the fields; the explicit SSH port is
    // the default; user names are plain.
    let gateway = connection(&p, "gateway");
    assert_eq!(gateway.port, None);
    assert_eq!(gateway.username.as_deref(), Some("ops"));

    // `SubRep` with four levels becomes four nested folders, reused by the sessions of the
    // same section.
    assert_eq!(
        parent_name(&p, "edge01.example.org (.\\alice)").as_deref(),
        Some("Edge")
    );
    assert_eq!(parent_name(&p, "Edge").as_deref(), Some("Frontend"));
    assert_eq!(parent_name(&p, "Frontend").as_deref(), Some("Web"));
    assert_eq!(parent_name(&p, "Web").as_deref(), Some("Prod"));
    assert_eq!(
        p.nodes.iter().filter(|n| n.name() == "Web").count(),
        1,
        "Prod\\Web is shared by two sections"
    );
    fake_password_absent(&p);
}

// ------------------------------------------------------------------------ mRemoteNG

#[test]
fn mremoteng_containers_protocols_and_passwords() {
    let p = parse(ImportFormat::Mremoteng, "mremoteng/confCons.xml");
    assert_eq!(
        connection_names(&p),
        vec!["lab-ssh", "lab-rdp", "lab-vnc", "wiki", "intranet", "bad-port"]
    );
    assert_eq!(parent_name(&p, "lab-ssh").as_deref(), Some("Lab"));
    assert_eq!(parent_name(&p, "lab-vnc").as_deref(), Some("Nested"));
    assert_eq!(parent_name(&p, "Nested").as_deref(), Some("Lab"));

    let ssh = connection(&p, "lab-ssh");
    assert_eq!(ssh.host, "lab01.example.org");
    assert_eq!(ssh.port, None);
    assert_eq!(ssh.username.as_deref(), Some("root"));
    assert_eq!(ssh.notes, "Build host");

    let rdp = connection(&p, "lab-rdp");
    match &rdp.options {
        ConnectionOptions::Rdp { domain, .. } => assert_eq!(domain.as_deref(), Some("LAB")),
        other => panic!("expected rdp, got {other:?}"),
    }
    assert_eq!(rdp.port, None);

    assert_eq!(
        connection(&p, "wiki").options,
        ConnectionOptions::Web {
            url: "https://wiki.example.org".into(),
            isolated_profile: false
        },
        "default https port is omitted from the url"
    );
    assert_eq!(
        connection(&p, "intranet").options,
        ConnectionOptions::Web {
            url: "http://intranet.example.org:8080".into(),
            isolated_profile: false
        }
    );

    assert_eq!(
        warnings_containing(&p, "protocol 'telnet' is not supported"),
        1
    );
    assert_eq!(warnings_containing(&p, "without a hostname"), 1);
    assert_eq!(warnings_containing(&p, "invalid port"), 1);
    assert_eq!(warnings_containing(&p, "password ignored"), 2);
    fake_password_absent(&p);
}

#[test]
fn mremoteng_encrypted_file_is_unsupported() {
    let err = preview(ImportFormat::Mremoteng, &fixture("mremoteng/encrypted.xml"))
        .expect_err("encrypted");
    assert!(matches!(err, ImportError::Unsupported(_)));
}

// ------------------------------------------------------------------------------ CSV

#[test]
fn csv_folders_tags_notes_and_protocols() {
    let p = parse(ImportFormat::Csv, "csv/connections.csv");
    assert_eq!(
        connection_names(&p),
        vec![
            "db01",
            "desk01",
            "web, portal",
            "intranet",
            "vnc01",
            "nameless.example.org"
        ]
    );
    let db = connection(&p, "db01");
    assert_eq!(db.port, None);
    assert_eq!(db.username.as_deref(), Some("alice"));
    assert_eq!(db.tags, vec!["prod", "db"]);
    assert_eq!(db.notes, "primary, read-write");
    assert_eq!(parent_name(&p, "db01").as_deref(), Some("DB"));
    assert_eq!(parent_name(&p, "DB").as_deref(), Some("Prod"));

    let desk = connection(&p, "desk01");
    assert_eq!(desk.port, Some(3390));
    assert_eq!(desk.notes, "says \"hi\"");
    assert!(matches!(desk.options, ConnectionOptions::Rdp { .. }));

    let portal = connection(&p, "web, portal");
    assert_eq!(
        portal.options,
        ConnectionOptions::Web {
            url: "https://portal.example.org/app".into(),
            isolated_profile: false
        }
    );
    assert_eq!(portal.host, "portal.example.org");
    assert_eq!(portal.tags, vec!["public"]);
    assert_eq!(parent_name(&p, "web, portal").as_deref(), Some("Web"));

    assert_eq!(
        connection(&p, "intranet").options,
        ConnectionOptions::Web {
            url: "https://intranet.example.org".into(),
            isolated_profile: false
        },
        "a web row without a url is built from its host"
    );
    assert_eq!(parent_name(&p, "vnc01").as_deref(), Some("Lab"));
    assert_eq!(
        warnings_containing(&p, "protocol 'telnet' is not supported"),
        1
    );
    assert_eq!(warnings_containing(&p, "no host"), 1);
}

#[test]
fn csv_passwords_and_url_credentials_are_dropped() {
    let p = parse(ImportFormat::Csv, "csv/with-password.csv");
    let ssh = connection(&p, "ssh-pw");
    assert_eq!(ssh.username.as_deref(), Some("ops"));
    let web = connection(&p, "web-pw");
    assert_eq!(
        web.options,
        ConnectionOptions::Web {
            url: "https://admin.example.org/".into(),
            isolated_profile: false
        }
    );
    assert_eq!(
        web.username.as_deref(),
        Some("ops"),
        "the user name is kept"
    );
    assert_eq!(warnings_containing(&p, "password ignored"), 2);
    fake_password_absent(&p);
}

#[test]
fn csv_malformed_record_keeps_the_valid_rows() {
    let p = parse(ImportFormat::Csv, "csv/malformed.csv");
    assert!(connection_names(&p).contains(&"good".to_owned()));
}

#[test]
fn csv_without_required_columns_is_invalid() {
    let dir = TempDir::new("csv-header");
    let path = write_bytes(&dir, "no-protocol.csv", b"name,host\nx,h.example.org\n");
    let err = preview(ImportFormat::Csv, &path).expect_err("header");
    assert!(matches!(err, ImportError::InvalidInput(_)));
}

// ----------------------------------------------------------------- Portolano JSON

#[test]
fn portolano_json_gets_new_ids_and_remapped_jump_hosts() {
    let p = parse(ImportFormat::Json, "json/export.json");
    let prod = node(&p, "Prod");
    assert!(prod.is_folder());
    assert_ne!(
        prod.id(),
        uuid::Uuid::from_u128(1),
        "source ids are not kept"
    );
    match prod {
        // A credential id from another vault is dropped, not kept.
        Node::Folder(f) => assert_eq!(f.credential, CredentialRef::None),
        Node::Connection(_) => panic!("expected a folder"),
    }
    // One grouped warning (changed: it used to be one "was dropped" warning per reference).
    assert_eq!(warnings_containing(&p, "reset to none"), 1);

    let bastion = connection(&p, "bastion");
    assert_eq!(parent_name(&p, "bastion").as_deref(), Some("Prod"));
    assert_eq!(bastion.username.as_deref(), Some("alice"));
    assert_eq!(
        ssh_jump(&p, "db01"),
        Some(bastion.id),
        "jump host remapped inside the file"
    );
    assert_eq!(connection(&p, "db01").port, Some(2222));
    assert_eq!(connection(&p, "db01").credential, CredentialRef::None);

    assert_eq!(ssh_jump(&p, "edge"), None);
    assert_eq!(warnings_containing(&p, "jump host outside the file"), 1);
    assert!(matches!(
        connection(&p, "web").options,
        ConnectionOptions::Web { .. }
    ));
}

#[test]
fn portolano_json_password_keys_are_dropped() {
    let p = parse(ImportFormat::Json, "json/stray-password.json");
    assert_eq!(connection(&p, "box").username.as_deref(), Some("ops"));
    assert_eq!(warnings_containing(&p, "password ignored"), 1);
    fake_password_absent(&p);
}

#[test]
fn portolano_json_errors_are_clear() {
    let truncated = preview(ImportFormat::Json, &fixture("json/truncated.json")).expect_err("cut");
    assert!(matches!(truncated, ImportError::InvalidInput(_)));
    let other =
        preview(ImportFormat::Json, &fixture("json/not-portolano.json")).expect_err("other");
    assert!(matches!(other, ImportError::InvalidInput(_)));
    let newer =
        preview(ImportFormat::Json, &fixture("json/newer-version.json")).expect_err("newer");
    assert!(matches!(newer, ImportError::Unsupported(_)));
}

// ------------------------------------------------------- garbage, empty, limits

#[test]
fn non_text_bytes_are_refused_by_every_format() {
    let dir = TempDir::new("garbage");
    let bytes = [0x80u8, 0x81, 0xFF, 0x00, 0x12, 0xC3, 0x28];
    let path = write_bytes(&dir, "garbage.bin", &bytes);
    for format in [
        ImportFormat::SshConfig,
        ImportFormat::Rdp,
        ImportFormat::Mobaxterm,
        ImportFormat::Mremoteng,
        ImportFormat::Csv,
        ImportFormat::Json,
    ] {
        let err = preview(format, &path).expect_err("binary input");
        assert!(
            matches!(err, ImportError::InvalidInput(_)),
            "{format:?}: {err}"
        );
    }
}

#[test]
fn text_garbage_gives_warnings_or_clear_errors_never_a_panic() {
    let dir = TempDir::new("text-garbage");
    let path = write_bytes(
        &dir,
        "garbage.txt",
        b"this is not a config\n%%%% ====\n[[[\n",
    );
    let ssh = preview(ImportFormat::SshConfig, &path).expect("ssh is tolerant");
    assert!(ssh.nodes.is_empty());
    assert_eq!(warnings_containing(&ssh, "no connections found"), 1);
    let mxt = preview(ImportFormat::Mobaxterm, &path).expect("mobaxterm is tolerant");
    assert!(mxt.nodes.is_empty());
    assert!(preview(ImportFormat::Csv, &path).is_err());
    assert!(preview(ImportFormat::Rdp, &path).is_err());
    assert!(preview(ImportFormat::Mremoteng, &path).is_err());
    assert!(preview(ImportFormat::Json, &path).is_err());
}

#[test]
fn empty_files() {
    let dir = TempDir::new("empty");
    let path = write_bytes(&dir, "empty.txt", b"");
    for format in [ImportFormat::SshConfig, ImportFormat::Mobaxterm] {
        let p = preview(format, &path).expect("empty is not an error");
        assert!(p.nodes.is_empty());
        assert_eq!(warnings_containing(&p, "no connections found"), 1);
    }
    for format in [
        ImportFormat::Rdp,
        ImportFormat::Mremoteng,
        ImportFormat::Csv,
        ImportFormat::Json,
    ] {
        assert!(
            matches!(preview(format, &path), Err(ImportError::InvalidInput(_))),
            "{format:?}"
        );
    }
}

#[test]
fn utf16_big_endian_is_unsupported() {
    let dir = TempDir::new("utf16be");
    let path = write_bytes(&dir, "be.rdp", &[0xFE, 0xFF, 0x00, 0x61]);
    let err = preview(ImportFormat::Rdp, &path).expect_err("big endian");
    assert!(matches!(err, ImportError::Unsupported(_)));
}

#[test]
fn files_over_ten_mib_are_refused_and_exactly_ten_mib_is_read() {
    let dir = TempDir::new("size");
    let limit = portolano_importers::MAX_FILE_BYTES as usize;

    let too_big = write_bytes(&dir, "too-big.conf", &vec![b'#'; limit + 1]);
    let err = preview(ImportFormat::SshConfig, &too_big).expect_err("too big");
    assert!(matches!(err, ImportError::InvalidInput(_)));

    let exact = write_bytes(&dir, "exact.conf", &vec![b'#'; limit]);
    let p = preview(ImportFormat::SshConfig, &exact).expect("exactly at the limit");
    assert!(p.nodes.is_empty());
}

#[test]
fn more_than_ten_thousand_entries_are_refused() {
    let dir = TempDir::new("entries");
    let config = |count: usize| {
        let mut text = String::new();
        for n in 0..count {
            text.push_str(&format!("Host h{n}\n  HostName h{n}.example.org\n"));
        }
        text
    };
    let at_limit = write_bytes(&dir, "at-limit.conf", config(10_000).as_bytes());
    let p = preview(ImportFormat::SshConfig, &at_limit).expect("10000 entries");
    assert_eq!(p.nodes.len(), 10_000);

    let over = write_bytes(&dir, "over.conf", config(10_001).as_bytes());
    let err = preview(ImportFormat::SshConfig, &over).expect_err("10001 entries");
    assert!(matches!(err, ImportError::InvalidInput(_)));
}

#[test]
fn missing_file_is_an_io_error() {
    let err = preview(ImportFormat::Csv, &fixture("does-not-exist.csv")).expect_err("missing");
    assert!(matches!(err, ImportError::Io(_)));
}

#[test]
fn preview_ids_are_new_on_every_import() {
    let a = parse(ImportFormat::SshConfig, "ssh/config.conf");
    let b = parse(ImportFormat::SshConfig, "ssh/config.conf");
    assert_ne!(a.preview_id, b.preview_id);
    assert_ne!(id_of(&a, "bastion"), id_of(&b, "bastion"));
    assert!(a.nodes.iter().all(|n| !matches!(n, Node::Folder(_))));
}
