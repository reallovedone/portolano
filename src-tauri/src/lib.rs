//! Portolano application crate: Tauri setup, shared state and IPC commands.

mod commands;
mod error;
mod state;

use tauri::{Emitter, Manager, RunEvent};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // `try_init` only fails when a global subscriber is already installed (for example in
    // tests); logging then goes to that subscriber, so there is nothing to recover.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .ok();
}

/// Starts the application.
pub fn run() {
    init_tracing();

    let app = tauri::Builder::default()
        // Native open/save file dialogs (import and export). Only the `open` and `save`
        // permissions are granted, in `capabilities/main.json`.
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = portolano_core_model::data_dir()?;
            portolano_core_model::private_fs::harden_data_dir(&data_dir)?;
            tracing::info!(data_dir = %data_dir.display(), "starting");
            let state = AppState::new(data_dir);
            // Notices from code outside a command (hooks of a session) reach the main webview.
            let handle = app.handle().clone();
            state.notifier.connect(move |event, payload| {
                if let Err(e) = handle.emit_to("main", event, payload) {
                    tracing::debug!(error = %e, event, "notice not delivered");
                }
            });
            app.manage(state);
            // Vault auto-lock timer (WP3).
            commands::vault::spawn_auto_lock(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Contract commands. Keep each group together.
            commands::model::doc_get,
            commands::model::doc_upsert_node,
            commands::model::doc_delete_node,
            commands::model::doc_move_node,
            commands::vault::vault_status,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_change_password,
            commands::vault::vault_lock,
            commands::vault::vault_list,
            commands::vault::vault_upsert,
            commands::vault::vault_delete,
            commands::vault::vault_reveal_secret,
            commands::term::term_open,
            commands::term::term_write,
            commands::term::term_ack,
            commands::term::term_resize,
            commands::term::ssh_host_key_decision,
            commands::term::known_hosts_list,
            commands::term::known_hosts_remove,
            commands::fb::fb_open,
            commands::fb::fb_pointer,
            commands::fb::fb_key,
            commands::fb::fb_resize,
            commands::fb::fb_set_visible,
            commands::fb::fb_cert_decision,
            commands::fb::fb_unencrypted_decision,
            commands::fb::fb_forget_certificate,
            commands::web::web_open,
            commands::web::web_set_bounds,
            commands::web::web_navigate,
            commands::session_close,
            commands::session_credential,
            commands::session_credential_cancel,
            commands::transfer::import_preview,
            commands::transfer::import_apply,
            commands::transfer::export_document,
            // Not in the contract table: UI-to-backend log line.
            commands::ui_log,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = handle.try_state::<AppState>() {
                    state.sessions.close_all();
                    commands::vault::lock_on_exit(&state);
                }
            }
        }),
        Err(err) => {
            tracing::error!(%err, "failed to build the application");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Release builds keep integer overflow checks (sizes and offsets come from remote servers),
    /// and every crate of the workspace declares the licence.
    #[test]
    fn the_release_profile_checks_overflow_and_every_crate_has_a_licence() {
        let workspace = include_str!("../../Cargo.toml");
        let release = workspace
            .split("[profile.release]")
            .nth(1)
            .expect("a [profile.release] section");
        assert!(release.contains("overflow-checks = true"));
        assert!(workspace.contains("license = \"MIT\""));
        for manifest in [
            include_str!("../Cargo.toml"),
            include_str!("../../crates/core-model/Cargo.toml"),
            include_str!("../../crates/importers/Cargo.toml"),
            include_str!("../../crates/session-rdp/Cargo.toml"),
            include_str!("../../crates/session-term/Cargo.toml"),
            include_str!("../../crates/session-vnc/Cargo.toml"),
            include_str!("../../crates/vault/Cargo.toml"),
        ] {
            assert!(manifest.contains("license.workspace = true"), "{manifest}");
        }
    }

    /// `default-src` does not cover `base-uri`, `form-action` or `object-src`: both policies (the
    /// release one and the one `tauri dev` uses) close them explicitly.
    #[test]
    fn the_content_security_policy_closes_what_default_src_does_not_cover() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let security = &conf["app"]["security"];
        for key in ["csp", "devCsp"] {
            let csp = security[key].as_str().expect(key);
            for directive in ["base-uri 'none'", "form-action 'none'", "object-src 'none'"] {
                assert!(csp.contains(directive), "{key} lacks {directive}: {csp}");
            }
            assert!(csp.contains("default-src 'self'"), "{key}: {csp}");
        }
    }

    /// Every command of §4.3 must be registered in the single `generate_handler!` list.
    #[test]
    fn all_contract_commands_are_registered() {
        const EXPECTED: &[&str] = &[
            "doc_get",
            "doc_upsert_node",
            "doc_delete_node",
            "doc_move_node",
            "vault_status",
            "vault_create",
            "vault_unlock",
            "vault_change_password",
            "vault_lock",
            "vault_list",
            "vault_upsert",
            "vault_delete",
            "vault_reveal_secret",
            "term_open",
            "term_write",
            "term_ack",
            "term_resize",
            "ssh_host_key_decision",
            "known_hosts_list",
            "known_hosts_remove",
            "fb_open",
            "fb_pointer",
            "fb_key",
            "fb_resize",
            "fb_set_visible",
            "fb_cert_decision",
            "fb_unencrypted_decision",
            "fb_forget_certificate",
            "web_open",
            "web_set_bounds",
            "web_navigate",
            "session_close",
            "session_credential",
            "session_credential_cancel",
            "import_preview",
            "import_apply",
            "export_document",
        ];
        let source = include_str!("lib.rs");
        let handler = source
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("generate_handler list");
        for name in EXPECTED {
            let needle = format!("::{name},");
            assert!(
                handler.contains(&needle),
                "command {name} is not registered"
            );
        }
    }
}
