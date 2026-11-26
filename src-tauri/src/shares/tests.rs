use std::fs;

use portolano_core_model::prompt::{SavedCredential, SavedSecret};
use portolano_core_model::{CredentialRef, ErrorCode, Node};
use secrecy::SecretString;
use uuid::Uuid;

use super::testkit::{
    at, count_events, events, scratch, state_with_vault, subscribe, team_folder, world,
};
use super::*;
use crate::commands::model::with_document;
use crate::state::AppState;

fn summaries(state: &AppState) -> Vec<ShareSummary> {
    state
        .shares
        .with_store(|store| state.shares.summaries(store.document()))
        .expect("summaries")
}

fn find(state: &AppState, name: &str) -> Option<Node> {
    with_document(state, |doc| {
        Ok(doc.nodes.iter().find(|n| n.name() == name).cloned())
    })
    .expect("document")
}

fn raw_document(state: &AppState) -> String {
    state
        .shares
        .with_store(|store| Ok(serde_json::to_string(store.document()).expect("json")))
        .expect("document")
}

#[tokio::test]
async fn publish_then_subscribe_round_trip() {
    let w = world();
    let owner_side = &summaries(&w.owner)[0];
    assert_eq!(owner_side.role, ShareRole::Owner);
    assert_eq!(owner_side.revision, 1);
    assert_eq!(owner_side.root_node_id, Some(w.team));

    let preview = w.sub.shares.preview(at(&w.file)).await.expect("preview");
    assert_eq!(preview.name, "Team");
    assert_eq!(preview.owner_name, "Ada");
    assert_eq!((preview.folders, preview.connections), (1, 1));
    assert!(
        preview.skipped.is_empty(),
        "local shells never reach the file"
    );
    assert_eq!(preview.fingerprint, owner_side.fingerprint);
    // Every target is listed, because subscribing confirms all of them.
    assert_eq!(preview.hosts.len(), 1);
    let host = &preview.hosts[0];
    assert_eq!(
        (
            host.name.as_str(),
            host.protocol.as_str(),
            host.host.as_str()
        ),
        ("web", "ssh", "web.example.net")
    );
    assert_eq!(host.username.as_deref(), Some("root"));

    let summary = w
        .sub
        .shares
        .subscribe(preview.preview_id, None)
        .expect("subscribe");
    assert_eq!(summary.role, ShareRole::Subscriber);
    assert_eq!(
        summary.pending_targets, 0,
        "the user saw the targets in the preview"
    );
    let root = find(&w.sub, "Team").expect("mounted root");
    assert_eq!(Some(root.id()), summary.root_node_id);
    let web = find(&w.sub, "web").expect("mounted connection");
    assert_ne!(web.id(), w.web, "mounted ids are derived, not the owner's");
    assert!(find(&w.sub, "shell").is_none());
    let Node::Connection(web) = web else {
        panic!("connection");
    };
    assert_eq!(web.credential, CredentialRef::Inherit);
    assert!(w.sub.shares.cache_path(summary.id).exists());
    // The preview is used up.
    assert_eq!(
        w.sub
            .shares
            .subscribe(preview.preview_id, None)
            .expect_err("used")
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn subscribe_same_share_twice_is_conflict() {
    let w = world();
    subscribe(&w).await;
    let again = w.sub.shares.preview(at(&w.file)).await.expect("preview");
    assert_eq!(
        w.sub
            .shares
            .subscribe(again.preview_id, None)
            .expect_err("duplicate")
            .code,
        ErrorCode::Conflict
    );
    // The owner cannot subscribe to its own share either.
    let own = w.owner.shares.preview(at(&w.file)).await.expect("preview");
    assert_eq!(
        w.owner
            .shares
            .subscribe(own.preview_id, None)
            .expect_err("own share")
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn republish_increments_revision() {
    let w = world();
    let summary = w.owner.shares.republish(w.link).expect("republish");
    assert_eq!(summary.revision, 2);
    let share = decode(&fs::read(&w.file).expect("file")).expect("decode");
    assert_eq!(share.payload.revision, 2);

    subscribe(&w).await;
    w.owner.shares.republish(w.link).expect("again");
    let synced = w.sub.shares.sync(None).await.expect("sync");
    assert_eq!(synced[0].revision, 3);
    assert_eq!(synced[0].last_error, None);
    assert_eq!(synced[0].status, ShareStatus::Ok);
}

#[tokio::test]
async fn publish_with_locked_vault_writes_nothing() {
    let dir = scratch();
    let state = state_with_vault(dir.path(), true);
    let (team, _) = team_folder(&state);
    state.vault.lock().expect("vault").lock().expect("lock");
    let file = dir.path().join("x.portolano-share");
    let err = state
        .shares
        .publish(team, &file.to_string_lossy())
        .expect_err("locked");
    assert_eq!(err.code, ErrorCode::VaultLocked);
    assert!(!file.exists());
    assert!(summaries(&state).is_empty());
}

#[test]
fn publish_refuses_a_folder_shared_already_and_needs_an_identity() {
    let dir = scratch();
    let state = state_with_vault(dir.path(), false);
    let (team, _) = team_folder(&state);
    let file = dir.path().join("x.portolano-share");
    assert_eq!(
        state
            .shares
            .publish(team, &file.to_string_lossy())
            .expect_err("no identity")
            .code,
        ErrorCode::NotFound
    );
    assert!(!file.exists());
    state
        .vault
        .lock()
        .expect("vault")
        .add_identity(Uuid::new_v4(), "Ada", &[1u8; 32])
        .expect("identity");
    state
        .shares
        .publish(team, &file.to_string_lossy())
        .expect("publish");
    assert_eq!(
        state
            .shares
            .publish(team, &dir.path().join("y").to_string_lossy())
            .expect_err("twice")
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn sync_keeps_cache_on_bad_signature_and_sets_last_error() {
    let w = world();
    let summary = subscribe(&w).await;
    let cache = w.sub.shares.cache_path(summary.id);
    let cached = fs::read(&cache).expect("cache");

    let mut tampered = fs::read(&w.file).expect("file");
    let at_byte = tampered.len() / 2;
    tampered[at_byte] = if tampered[at_byte] == b'A' {
        b'B'
    } else {
        b'A'
    };
    fs::write(&w.file, &tampered).expect("tamper");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert!(synced[0].last_error.is_some());
    assert_eq!(synced[0].status, ShareStatus::Rejected);
    assert_eq!(
        fs::read(&cache).expect("cache"),
        cached,
        "the cache is untouched"
    );
    assert!(
        find(&w.sub, "web").is_some(),
        "the mounted tree is untouched"
    );

    fs::write(&w.file, b"not a share").expect("garbage");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert!(synced[0].last_error.is_some());
    assert_eq!(fs::read(&cache).expect("cache"), cached);

    // A good file again clears the error.
    w.owner.shares.republish(w.link).expect("republish");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert_eq!(synced[0].last_error, None);
    assert_eq!(synced[0].status, ShareStatus::Ok);
    assert_eq!(synced[0].revision, 2);
}

#[tokio::test]
async fn sync_refuses_same_revision_with_other_content() {
    let w = world();
    let summary = subscribe(&w).await;
    let cache = w.sub.shares.cache_path(summary.id);
    let cached = fs::read(&cache).expect("cache");

    // The owner key signs another payload under the same revision.
    let mut share = decode(&fs::read(&w.file).expect("file")).expect("decode");
    share.payload.published_at = "2030-01-01T00:00:00Z".into();
    let owner = Identity::from_secret(Uuid::new_v4(), "Ada", &[7u8; 32]);
    fs::write(&w.file, encode(&share.payload, &owner).expect("encode")).expect("rewrite");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert!(synced[0]
        .last_error
        .as_deref()
        .is_some_and(|e| e.contains("without a new revision")));
    assert_eq!(synced[0].revision, 1);
    assert_eq!(
        fs::read(&cache).expect("cache"),
        cached,
        "the cache is kept"
    );

    // A proper republish (a new revision) recovers.
    w.owner.shares.republish(w.link).expect("republish");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert_eq!(synced[0].last_error, None);
    assert_eq!(synced[0].revision, 2);
}

#[tokio::test]
async fn sync_refuses_an_older_revision_and_a_foreign_owner() {
    let w = world();
    let old = fs::read(&w.file).expect("revision 1");
    let summary = subscribe(&w).await;
    w.owner.shares.republish(w.link).expect("revision 2");
    w.sub.shares.sync(None).await.expect("sync");
    fs::write(&w.file, &old).expect("roll back");
    let synced = w.sub.shares.sync(None).await.expect("sync");
    assert_eq!(synced[0].revision, 2);
    assert!(synced[0].last_error.is_some(), "a rollback is refused");

    // Someone else publishes a file for the same share id: refused.
    let other_dir = scratch();
    let other = state_with_vault(other_dir.path(), true);
    let (team, _) = team_folder(&other);
    let forged = other_dir.path().join("forged.portolano-share");
    other
        .shares
        .publish(team, &forged.to_string_lossy())
        .expect("publish");
    fs::copy(&forged, &w.file).expect("replace");
    let synced = w.sub.shares.sync(Some(summary.id)).await.expect("sync");
    assert!(synced[0].last_error.is_some());
    assert_eq!(synced[0].revision, 2);
}

#[tokio::test]
async fn sync_keeps_cache_when_path_missing() {
    let w = world();
    let summary = subscribe(&w).await;
    fs::remove_file(&w.file).expect("remove");
    let synced = w.sub.shares.sync(None).await.expect("sync");
    let error = synced[0].last_error.clone().expect("error");
    assert!(error.contains("could not be found"), "{error}");
    assert_eq!(synced[0].status, ShareStatus::Offline);
    assert!(
        !error.contains(&*w.file.to_string_lossy()),
        "no paths in errors"
    );
    assert!(w.sub.shares.cache_path(summary.id).exists());
    assert!(find(&w.sub, "web").is_some());
    assert!(
        synced[0].root_node_id.is_some(),
        "still mounted from the copy"
    );
}

#[tokio::test]
async fn sync_not_modified_does_nothing() {
    let w = world();
    subscribe(&w).await;
    let events_seen = count_events(&w.sub);
    let before = raw_document(&w.sub);
    for _ in 0..2 {
        let synced = w.sub.shares.sync(None).await.expect("sync");
        assert_eq!(synced[0].last_error, None);
    }
    assert_eq!(
        raw_document(&w.sub),
        before,
        "the document is not rewritten"
    );
    assert_eq!(events(&events_seen), 0, "no event without a change");

    w.owner.shares.republish(w.link).expect("republish");
    w.sub.shares.sync(None).await.expect("sync");
    assert_eq!(events(&events_seen), 1);
}

#[tokio::test]
async fn sync_of_an_unknown_link_is_not_found() {
    let w = world();
    assert_eq!(
        w.sub
            .shares
            .sync(Some(Uuid::new_v4()))
            .await
            .expect_err("unknown")
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn changed_host_becomes_pending_until_confirmed() {
    let w = world();
    let summary = subscribe(&w).await;
    let web = find(&w.sub, "web").expect("web").id();
    let wanted = CredentialRef::Id { id: Uuid::new_v4() };
    w.sub.shares.set_credential(web, wanted).expect("choose");
    let credential = |state: &AppState| match find(state, "web") {
        Some(Node::Connection(c)) => c.credential,
        other => panic!("{other:?}"),
    };
    // Subscribing confirmed what the preview showed: the choice applies at once.
    assert_eq!(credential(&w.sub), wanted);
    assert_eq!(summaries(&w.sub)[0].pending_targets, 0);

    // The owner points the connection somewhere else.
    w.owner
        .shares
        .with_store(|store| {
            let Some(Node::Connection(mut c)) = store.document().node(w.web).cloned() else {
                panic!("connection");
            };
            c.host = "elsewhere.example.net".into();
            store.upsert_node(Node::Connection(c))?;
            Ok(())
        })
        .expect("change host");
    w.owner.shares.republish(w.link).expect("republish");
    let synced = w.sub.shares.sync(None).await.expect("sync");
    assert_eq!(synced[0].pending_targets, 1);
    assert_eq!(synced[0].pending_hosts.len(), 1);
    assert_eq!(synced[0].pending_hosts[0].host, "elsewhere.example.net");
    assert_eq!(synced[0].pending_hosts[0].node_id, web);
    assert_eq!(
        credential(&w.sub),
        CredentialRef::None,
        "pending: no credential"
    );

    // "Save to vault" for a pending host explains that the host must be confirmed first, and
    // leaves nothing behind in the vault.
    let typed = SavedCredential {
        username: "ann".into(),
        domain: None,
        secret: SavedSecret::Password(SecretString::from("fake-pw".to_owned())),
    };
    let refused =
        crate::commands::prompt::save_remote(&w.sub.vault, &w.sub.shares, web, "web", typed)
            .expect_err("pending");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(refused.message.contains("Share info"));
    assert!(w
        .sub
        .vault
        .lock()
        .expect("vault")
        .list()
        .expect("list")
        .is_empty());

    // A stale revision is refused; the one the user saw is accepted.
    assert_eq!(
        w.sub
            .shares
            .confirm_targets(summary.id, synced[0].revision + 1)
            .expect_err("stale")
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(credential(&w.sub), CredentialRef::None);
    w.sub
        .shares
        .confirm_targets(summary.id, synced[0].revision)
        .expect("confirm");
    assert_eq!(credential(&w.sub), wanted);
    assert_eq!(summaries(&w.sub)[0].pending_targets, 0);
    assert!(summaries(&w.sub)[0].pending_hosts.is_empty());
}

#[tokio::test]
async fn cached_copy_is_used_at_startup_offline() {
    let w = world();
    let summary = subscribe(&w).await;
    fs::remove_file(&w.file).expect("offline");
    let restarted = AppState::new(w._sub_dir.path().to_path_buf());
    assert!(find(&restarted, "web").is_some());
    let shown = &summaries(&restarted)[0];
    assert_eq!(shown.id, summary.id);
    assert_eq!(shown.last_error, None);
    assert_eq!(shown.revision, 1);
}

#[tokio::test]
async fn unusable_cached_copy_is_dropped_with_an_error() {
    let w = world();
    let summary = subscribe(&w).await;
    let cache = w.sub.shares.cache_path(summary.id);
    fs::write(&cache, b"garbage").expect("corrupt");
    let restarted = AppState::new(w._sub_dir.path().to_path_buf());
    assert!(find(&restarted, "web").is_none());
    assert!(!cache.exists(), "the bad copy is removed");
    let shown = &summaries(&restarted)[0];
    assert!(shown.last_error.is_some());
    assert_eq!(shown.status, ShareStatus::Error);
    assert_eq!(
        shown.root_node_id, None,
        "nothing to mount until the next sync"
    );
    // The next sync brings the share back (the file is still there).
    let synced = restarted.shares.sync(None).await.expect("sync");
    assert_eq!(synced[0].last_error, None);
    assert!(find(&restarted, "web").is_some());
    assert!(cache.exists());
}

#[tokio::test]
async fn no_usable_copy_and_source_offline_is_offline_without_a_root() {
    let w = world();
    let summary = subscribe(&w).await;
    fs::write(w.sub.shares.cache_path(summary.id), b"garbage").expect("corrupt");
    fs::remove_file(&w.file).expect("remove");
    let restarted = AppState::new(w._sub_dir.path().to_path_buf());
    let synced = restarted.shares.sync(None).await.expect("sync");
    assert_eq!(synced[0].status, ShareStatus::Offline);
    assert_eq!(synced[0].root_node_id, None);
    // Still listed, so the UI can offer to sync or unsubscribe it.
    assert_eq!(summaries(&restarted).len(), 1);
    restarted
        .shares
        .unsubscribe(summary.id)
        .expect("unsubscribe");
    assert!(summaries(&restarted).is_empty());
}

#[tokio::test]
async fn unsubscribe_removes_link_cache_and_mount() {
    let w = world();
    let summary = subscribe(&w).await;
    w.sub.shares.unsubscribe(summary.id).expect("unsubscribe");
    assert!(find(&w.sub, "web").is_none());
    assert!(!w.sub.shares.cache_path(summary.id).exists());
    assert!(summaries(&w.sub).is_empty());
    assert_eq!(
        w.sub.shares.unsubscribe(summary.id).expect_err("gone").code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn deleted_owner_folder_shows_folder_deleted() {
    let w = world();
    w.owner
        .shares
        .with_store(|store| Ok(store.delete_node(w.team)?))
        .expect("delete");
    let shown = &summaries(&w.owner)[0];
    assert_eq!(shown.last_error.as_deref(), Some("folder deleted"));
    assert_eq!(shown.status, ShareStatus::FolderDeleted);
    assert_eq!(
        w.owner.shares.republish(w.link).expect_err("gone").code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn subscribe_into_a_folder_and_not_into_a_remote_one() {
    let w = world();
    let (target, _) = {
        let folder = super::testkit::folder("Shared", None);
        let id = folder.id();
        w.sub
            .shares
            .with_store(|store| {
                store.upsert_node(folder)?;
                Ok(())
            })
            .expect("folder");
        (id, ())
    };
    let preview = w.sub.shares.preview(at(&w.file)).await.expect("preview");
    let summary = w
        .sub
        .shares
        .subscribe(preview.preview_id, Some(target))
        .expect("subscribe");
    let root = find(&w.sub, "Team").expect("root");
    assert_eq!(root.parent_id(), Some(target));
    assert_eq!(summary.root_node_id, Some(root.id()));
    // Mounted content cannot be a destination.
    let again = w.sub.shares.preview(at(&w.file)).await.expect("preview");
    assert_eq!(
        w.sub
            .shares
            .subscribe(again.preview_id, Some(root.id()))
            .expect_err("remote parent")
            .code,
        ErrorCode::ReadOnly
    );
}

#[test]
fn rfc3339_has_the_expected_shape() {
    let now = now_rfc3339();
    assert_eq!(now.len(), 20, "{now}");
    assert!(now.starts_with("20") && now.ends_with('Z'));
    assert_eq!(&now[10..11], "T");
}

#[tokio::test]
async fn publishing_a_shared_folder_again_is_a_conflict() {
    let w = world();
    let dir = scratch();
    let other = dir.path().join("second.portolano-share");
    let error = w
        .owner
        .shares
        .publish(w.team, other.to_str().expect("path"))
        .expect_err("already shared");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(!other.exists(), "nothing is written for a refused publish");
    let owners = summaries(&w.owner)
        .iter()
        .filter(|s| s.role == ShareRole::Owner)
        .count();
    assert_eq!(owners, 1);
}

#[test]
fn a_folder_edited_while_signing_is_not_published_with_old_content() {
    let w = world();
    let (signed, mut edited) = w
        .owner
        .shares
        .with_store(|store| {
            let doc = store.document();
            let signed = prepare_for_publish(doc, w.team).map_err(|e| share_error(&e))?;
            Ok((signed, doc.clone()))
        })
        .expect("snapshot");
    ShareService::unchanged_since_signing(&edited, w.team, &signed).expect("same content");

    let renamed = edited
        .nodes
        .iter_mut()
        .find(|n| n.id() == w.web)
        .expect("web connection");
    let Node::Connection(c) = renamed else {
        panic!("connection")
    };
    c.name = "renamed while signing".into();
    let err = ShareService::unchanged_since_signing(&edited, w.team, &signed)
        .expect_err("changed content");
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[test]
fn overlapping_republishes_get_distinct_revisions() {
    let w = world();
    let shares = &w.owner.shares;
    let mut revisions: Vec<u64> = std::thread::scope(|s| {
        let a = s.spawn(|| shares.republish(w.link));
        let b = s.spawn(|| shares.republish(w.link));
        [a, b]
            .map(|h| h.join().expect("thread").expect("republish").revision)
            .into()
    });
    revisions.sort_unstable();
    assert_eq!(revisions, vec![2, 3]);
    let share = decode(&fs::read(&w.file).expect("file")).expect("decode");
    assert_eq!(share.payload.revision, 3);
}

#[test]
fn a_publish_that_cannot_write_the_file_leaves_no_link() {
    let dir = scratch();
    let state = state_with_vault(dir.path(), true);
    let (team, _) = team_folder(&state);
    let file = dir.path().join("missing").join("x.portolano-share");
    let err = state
        .shares
        .publish(team, &file.to_string_lossy())
        .expect_err("no folder");
    assert_eq!(err.code, ErrorCode::Io);
    assert!(summaries(&state).is_empty());
    let file = dir.path().join("x.portolano-share");
    state
        .shares
        .publish(team, &file.to_string_lossy())
        .expect("publish after the failure");
}
