//! Native macOS menu bar (WP13c). Tauri's default menu stays as it is (app, Edit, View, Window,
//! Help, with their predefined items and shortcuts); the only addition is "Check for Updates…"
//! right below "About Portolano" in the application menu. Windows and Linux have no menu bar and
//! this module is not compiled there.

use tauri::menu::{Menu, MenuEvent, MenuItem, MenuItemKind};
use tauri::{AppHandle, Emitter, Wry};

/// Id of the "Check for Updates…" menu item.
const CHECK_FOR_UPDATES_ID: &str = "portolano.check-for-updates";

/// Event (to the main webview, no payload) emitted when the menu item is chosen. The UI opens the
/// same "Software update" dialog as the sidebar entry.
pub const CHECK_FOR_UPDATES_EVENT: &str = "menu-check-for-updates";

/// Position of the new item in the application menu: right after "About".
const INSERT_POSITION: usize = 1;

/// The webview event a menu item id stands for, if the id is one of ours. Predefined items
/// (Quit, Copy, Paste...) have ids of their own and are handled natively, never by us.
fn event_for_menu_id(id: &str) -> Option<&'static str> {
    match id {
        CHECK_FOR_UPDATES_ID => Some(CHECK_FOR_UPDATES_EVENT),
        _ => None,
    }
}

/// The default menu with "Check for Updates…" added; the plain default menu when that fails.
/// Only a failure of the default menu itself is returned (Tauri then reports it as it always
/// did).
pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    match add_check_for_updates(app, &menu) {
        Ok(true) => {}
        Ok(false) => tracing::warn!("no application submenu found; using the default menu"),
        Err(e) => tracing::warn!(
            error = %e,
            "could not add \"Check for Updates…\" to the menu; using the default menu"
        ),
    }
    Ok(menu)
}

fn add_check_for_updates(app: &AppHandle<Wry>, menu: &Menu<Wry>) -> tauri::Result<bool> {
    // The application menu is the first one of the default menu on macOS.
    let Some(app_menu) = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) => Some(submenu),
        _ => None,
    }) else {
        return Ok(false);
    };
    let item = MenuItem::with_id(
        app,
        CHECK_FOR_UPDATES_ID,
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    app_menu.insert(&item, INSERT_POSITION)?;
    Ok(true)
}

/// Global menu event handler: forwards our items to the main webview.
pub fn on_event(app: &AppHandle<Wry>, event: MenuEvent) {
    if let Some(name) = event_for_menu_id(event.id().as_ref()) {
        if let Err(e) = app.emit_to("main", name, ()) {
            tracing::debug!(error = %e, event = name, "menu event not delivered");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_item_is_mapped_to_an_event() {
        assert_eq!(
            event_for_menu_id(CHECK_FOR_UPDATES_ID),
            Some(CHECK_FOR_UPDATES_EVENT)
        );
        assert_eq!(event_for_menu_id("quit"), None);
        assert_eq!(event_for_menu_id(""), None);
    }

    #[test]
    fn the_event_name_is_the_one_the_ui_listens_to() {
        let ui = include_str!("../../ui/src/lib/ipc/tauri.ts");
        assert!(ui.contains(&format!("\"{CHECK_FOR_UPDATES_EVENT}\"")));
    }
}
