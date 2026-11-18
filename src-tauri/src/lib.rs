//! Portolano application crate: Tauri setup, shared state and IPC commands.

mod clipboard;
mod commands;
mod error;
#[cfg(target_os = "macos")]
mod menu;
mod quit;
#[cfg(target_os = "macos")]
mod quit_macos;
mod settings;
mod state;
mod update;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // The clipboard channel of IronRDP logs file names of the remote clipboard when it has to
    // fix or skip one. File names are never logged by Portolano, so that crate stays silent
    // whatever `RUST_LOG` says.
    let filter = match "ironrdp_cliprdr=off".parse() {
        Ok(directive) => filter.add_directive(directive),
        Err(_) => filter,
    };
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
    // Several TLS providers end up compiled in (the updater brings `ring`, the RDP and VNC code
    // `aws-lc-rs`): fix the process default explicitly so that no library has to guess.
    if tokio_rustls::rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a process-level TLS provider was already installed");
    }

    let builder = tauri::Builder::default();
    // macOS only: the default menu bar plus "Check for Updates…". Elsewhere there is no menu bar.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(menu::on_event);
    let app = builder
        // Native open/save file dialogs (import and export). Only the `open` and `save`
        // permissions are granted, in `capabilities/main.json`.
        .plugin(tauri_plugin_dialog::init())
        // Auto-update. Driven from Rust (`commands/update.rs`); no capability grants the webview
        // the plugin's own commands.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = portolano_core_model::data_dir()?;
            portolano_core_model::private_fs::harden_data_dir(&data_dir)?;
            tracing::info!(data_dir = %data_dir.display(), "starting");
            let state = AppState::new(data_dir.clone());
            // Notices from code outside a command (hooks of a session) reach the main webview.
            let handle = app.handle().clone();
            state.notifier.connect(move |event, payload| {
                if let Err(e) = handle.emit_to("main", event, payload) {
                    tracing::debug!(error = %e, event, "notice not delivered");
                }
            });
            app.manage(state);
            app.manage(quit::Quit::default());
            // ⌘Q, the Quit menu item and Dock → Quit do not raise `ExitRequested` on macOS.
            #[cfg(target_os = "macos")]
            quit_macos::install(app.handle());
            // One settings store for the whole app: the update service and the commands share it.
            let settings = Arc::new(settings::SettingsStore::load(&data_dir));
            app.manage(Arc::clone(&settings));
            // The saved auto-lock time must reach the vault before the timer's first tick.
            if let Err(e) =
                commands::vault::apply_saved_auto_lock(&app.state::<AppState>(), &settings)
            {
                tracing::warn!(error = %e, "could not apply the saved auto-lock time");
            }
            app.manage(update::UpdateService::new(
                settings,
                app.package_info().version.clone(),
            ));
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
            commands::vault::vault_get_auto_lock,
            commands::vault::vault_set_auto_lock,
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
            commands::fb::fb_clipboard_save,
            commands::fb::fb_clipboard_cancel,
            commands::web::web_open,
            commands::web::web_set_bounds,
            commands::web::web_navigate,
            commands::session_close,
            commands::session_credential,
            commands::session_credential_cancel,
            commands::transfer::import_preview,
            commands::transfer::import_apply,
            commands::transfer::export_document,
            commands::keyfile::read_private_key_file,
            commands::update::update_settings,
            commands::update::update_set_auto_check,
            commands::update::update_check,
            commands::update::update_skip,
            commands::update::update_install,
            commands::update::whats_new,
            commands::update::update_open_release_page,
            quit::quit_reply,
            // Not in the contract table: UI-to-backend log line.
            commands::ui_log,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| match event {
            // The user asked to quit (⌘Q, the menu, the Dock) or closed the last window:
            // the UI decides. `code: Some(_)` is our own `exit(0)` (or the updater's restart).
            RunEvent::ExitRequested { code, api, .. } if quit::intercepts_exit(code) => {
                api.prevent_exit();
                if let Some(quit) = handle.try_state::<quit::Quit>() {
                    quit.request(handle);
                }
            }
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                api.prevent_close();
                if let Some(quit) = handle.try_state::<quit::Quit>() {
                    quit.request(handle);
                }
            }
            RunEvent::Exit => {
                if let Some(state) = handle.try_state::<AppState>() {
                    state.sessions.close_all();
                    commands::vault::lock_on_exit(&state);
                }
            }
            _ => {}
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

    /// The updater configuration (WP13a): HTTPS endpoint only, a public key, signed update
    /// artifacts, a signature bound to the announced version, a passive (never silent) Windows
    /// installer, and no capability that would hand the updater plugin's commands to the webview.
    #[test]
    fn the_updater_is_configured_safely() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
        let updater = &conf["plugins"]["updater"];
        let endpoints = updater["endpoints"].as_array().expect("endpoints");
        assert_eq!(endpoints.len(), 1);
        for endpoint in endpoints {
            let url = endpoint.as_str().expect("endpoint text");
            assert!(url.starts_with("https://"), "{url}");
        }
        assert_eq!(
            endpoints[0],
            "https://github.com/reallovedone/portolano/releases/latest/download/latest.json"
        );
        assert!(!updater["pubkey"].as_str().expect("pubkey").is_empty());
        assert_eq!(updater["requireSignedVersion"], true);
        assert_eq!(updater["windows"]["installMode"], "passive");
        for key in [
            "dangerousInsecureTransportProtocol",
            "dangerousAcceptInvalidCerts",
            "dangerousAcceptInvalidHostnames",
            "allowDowngrades",
        ] {
            assert!(updater.get(key).is_none(), "{key} must not be set");
        }
        let capabilities = include_str!("../capabilities/main.json");
        assert!(
            !capabilities.contains("updater:") && !capabilities.contains("opener:"),
            "the webview must not receive the updater or opener commands"
        );
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
            "vault_get_auto_lock",
            "vault_set_auto_lock",
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
            "fb_clipboard_save",
            "fb_clipboard_cancel",
            "web_open",
            "web_set_bounds",
            "web_navigate",
            "session_close",
            "session_credential",
            "session_credential_cancel",
            "import_preview",
            "import_apply",
            "export_document",
            "read_private_key_file",
            "update_settings",
            "update_set_auto_check",
            "update_check",
            "update_skip",
            "update_install",
            "whats_new",
            "update_open_release_page",
            "quit_reply",
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
