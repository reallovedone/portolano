//! Helpers shared by the tests of the shared folders (`cfg(test)` only). Everything lives under
//! `<repo>/.tmp/shares`; nothing here touches real user data or the network.

use std::path::Path;

use crate::shares::ShareSummary;
use portolano_core_model::ShareLocation;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use portolano_core_model::{Connection, ConnectionOptions, CredentialRef, Folder, Node};
use portolano_vault::{KdfParams, Vault};
use secrecy::SecretString;
use uuid::Uuid;

use crate::state::AppState;

pub fn scratch() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/shares");
    std::fs::create_dir_all(&base).expect("base");
    tempfile::tempdir_in(base).expect("scratch")
}

/// A state whose vault is created and unlocked with a cheap key derivation; with `identity`
/// the vault also holds a signing identity named "Ada".
pub fn state_with_vault(dir: &Path, identity: bool) -> AppState {
    let kdf = KdfParams {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };
    let mut vault = Vault::with_kdf_params(dir.join("vault.bin"), kdf).expect("vault params");
    let password = || SecretString::from("test password".to_owned());
    vault.create(password()).expect("create");
    vault.unlock(password()).expect("unlock");
    let state = AppState::from_parts(dir.to_path_buf(), vault);
    if identity {
        state
            .vault
            .lock()
            .expect("vault")
            .add_identity(Uuid::new_v4(), "Ada", &[7u8; 32])
            .expect("identity");
    }
    state
}

/// Counts the events the state emits.
pub fn count_events(state: &AppState) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    state.notifier.connect(move |_, _| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    count
}

pub fn events(count: &AtomicUsize) -> usize {
    count.load(Ordering::SeqCst)
}

pub fn folder(name: &str, parent: Option<Uuid>) -> Node {
    Node::Folder(Folder {
        sort_index: None,
        icon: None,
        color: None,
        id: Uuid::new_v4(),
        parent_id: parent,
        name: name.into(),
        credential: CredentialRef::Inherit,
    })
}

pub fn ssh(name: &str, host: &str, parent: Option<Uuid>) -> Node {
    connection(
        name,
        host,
        parent,
        ConnectionOptions::Ssh {
            jump_host: None,
            keepalive_secs: None,
        },
    )
}

pub fn local_shell(name: &str, parent: Option<Uuid>) -> Node {
    connection(
        name,
        "localhost",
        parent,
        ConnectionOptions::LocalShell {
            program: None,
            args: vec![],
            cwd: None,
        },
    )
}

fn connection(name: &str, host: &str, parent: Option<Uuid>, options: ConnectionOptions) -> Node {
    Node::Connection(Connection {
        sort_index: None,
        icon: None,
        color: None,
        id: Uuid::new_v4(),
        parent_id: parent,
        name: name.into(),
        host: host.into(),
        port: None,
        credential: CredentialRef::Inherit,
        username: Some("root".into()),
        tags: vec![],
        notes: String::new(),
        options,
    })
}

/// Stores a folder "Team" with an ssh connection "web" and a local shell; returns the folder
/// id and the ssh connection id.
pub fn team_folder(state: &AppState) -> (Uuid, Uuid) {
    let team = folder("Team", None);
    let web = ssh("web", "web.example.net", Some(team.id()));
    let shell = local_shell("shell", Some(team.id()));
    let (team_id, web_id) = (team.id(), web.id());
    crate::commands::model::with_store(state, |store| {
        store.upsert_node(team)?;
        store.upsert_node(web)?;
        store.upsert_node(shell)?;
        Ok(())
    })
    .expect("store");
    (team_id, web_id)
}

pub fn at(path: &Path) -> ShareLocation {
    ShareLocation::Path {
        value: path.to_string_lossy().into_owned(),
    }
}

/// An owner with a published "Team" folder at `<dir>/team.portolano-share`, and a subscriber
/// state that has not subscribed yet.
pub struct World {
    pub _owner_dir: tempfile::TempDir,
    pub _sub_dir: tempfile::TempDir,
    pub owner: AppState,
    pub sub: AppState,
    pub team: Uuid,
    pub web: Uuid,
    pub file: std::path::PathBuf,
    pub link: Uuid,
}

pub fn world() -> World {
    let owner_dir = scratch();
    let sub_dir = scratch();
    let owner = state_with_vault(owner_dir.path(), true);
    let sub = state_with_vault(sub_dir.path(), false);
    let (team, web) = team_folder(&owner);
    let file = owner_dir.path().join("team.portolano-share");
    let published = owner
        .shares
        .publish(team, &file.to_string_lossy())
        .expect("publish");
    World {
        _owner_dir: owner_dir,
        _sub_dir: sub_dir,
        owner,
        sub,
        team,
        web,
        file,
        link: published.id,
    }
}

pub async fn subscribe(w: &World) -> ShareSummary {
    let preview = w.sub.shares.preview(at(&w.file)).await.expect("preview");
    w.sub
        .shares
        .subscribe(preview.preview_id, None)
        .expect("subscribe")
}
