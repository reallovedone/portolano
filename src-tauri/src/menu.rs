//! Native macOS menu bar (WP13c). Tauri's default menu stays as it is (app, Edit, View, Window,
//! Help, with their predefined items and shortcuts); the only addition is "Check for Updates…"
//! right below "About Portolano" in the application menu, plus "Release Notes…" in Help. Windows
//! and Linux have no menu bar and this module is not compiled there.

use std::sync::Mutex;

use tauri::menu::{
    Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, WindowEvent, Wry};

/// Id of the "Check for Updates…" menu item.
const CHECK_FOR_UPDATES_ID: &str = "portolano.check-for-updates";

/// Event (to the main webview, no payload) emitted when the menu item is chosen. The UI opens the
/// same "Software update" dialog as the sidebar entry.
pub const CHECK_FOR_UPDATES_EVENT: &str = "menu-check-for-updates";

/// Id of the "Settings…" menu item (⌘,).
const SETTINGS_ID: &str = "portolano.settings";

/// Event (to the main webview, no payload) emitted when "Settings…" is chosen. The UI opens the
/// Settings window.
pub const SETTINGS_EVENT: &str = "menu-settings";

/// Id of the "Release Notes…" menu item (Help menu).
const RELEASE_NOTES_ID: &str = "portolano.release-notes";

/// Event (to the main webview, no payload) emitted when "Release Notes…" is chosen. The UI opens
/// the Release notes window.
pub const RELEASE_NOTES_EVENT: &str = "menu-release-notes";

/// Id of the "Close Tab" menu item (⌘W), which replaces the predefined "Close Window".
const CLOSE_TAB_ID: &str = "portolano.close-tab";

/// Event (to the main webview, no payload) emitted when "Close Tab" is chosen. The UI closes the
/// active tab, asking first when it holds a connected session.
pub const CLOSE_TAB_EVENT: &str = "menu-close-tab";

/// Title of the menu that holds the predefined Cut, Copy, Paste... items.
const EDIT_MENU_TEXT: &str = "Edit";

/// Text of the predefined item that "Close Tab" replaces.
const CLOSE_WINDOW_TEXT: &str = "Close Window";

/// Position of the new item in the application menu: right after "About".
const INSERT_POSITION: usize = 1;

/// Position of "Settings…": after "About", "Check for Updates…" and the separator that follows.
const SETTINGS_POSITION: usize = 3;

/// The webview event a menu item id stands for, if the id is one of ours. Predefined items
/// (Quit, Copy, Paste...) have ids of their own and are handled natively, never by us.
fn event_for_menu_id(id: &str) -> Option<&'static str> {
    match id {
        CHECK_FOR_UPDATES_ID => Some(CHECK_FOR_UPDATES_EVENT),
        CLOSE_TAB_ID => Some(CLOSE_TAB_EVENT),
        SETTINGS_ID => Some(SETTINGS_EVENT),
        RELEASE_NOTES_ID => Some(RELEASE_NOTES_EVENT),
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
    if let Err(e) = add_settings(app, &menu) {
        tracing::warn!(error = %e, "could not add \"Settings…\" to the menu");
    }
    if let Err(e) = add_release_notes(app, &menu) {
        tracing::warn!(error = %e, "could not add \"Release Notes…\" to the Help menu");
    }
    if let Err(e) = replace_close_window(app, &menu) {
        tracing::warn!(error = %e, "could not replace \"Close Window\" with \"Close Tab\"");
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

/// Adds "Settings…" (⌘,) to the application menu, in the usual macOS place: after the "Check for
/// Updates…" group, before "Services", with a separator on each side.
fn add_settings(app: &AppHandle<Wry>, menu: &Menu<Wry>) -> tauri::Result<()> {
    let Some(app_menu) = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) => Some(submenu),
        _ => None,
    }) else {
        return Ok(());
    };
    let item = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
    // [About, Check for Updates…, separator, <here>, Services, ...]
    app_menu.insert(&item, SETTINGS_POSITION)?;
    app_menu.insert(&PredefinedMenuItem::separator(app)?, SETTINGS_POSITION + 1)?;
    Ok(())
}

/// Adds "Release Notes…" to the Help menu (empty by default on macOS), found by its id.
fn add_release_notes(app: &AppHandle<Wry>, menu: &Menu<Wry>) -> tauri::Result<()> {
    let Some(MenuItemKind::Submenu(help)) = menu.get(HELP_SUBMENU_ID) else {
        tracing::warn!("no Help menu found; \"Release Notes…\" not added");
        return Ok(());
    };
    let item = MenuItem::with_id(app, RELEASE_NOTES_ID, "Release Notes…", true, None::<&str>)?;
    help.append(&item)
}

/// Replaces every predefined "Close Window" (the File and Window menus both have one, with ⌘W)
/// by "Close Tab", which asks the UI to close the active tab. The red button and ⌘Q are not
/// touched.
fn replace_close_window(app: &AppHandle<Wry>, menu: &Menu<Wry>) -> tauri::Result<()> {
    for item in menu.items()? {
        let MenuItemKind::Submenu(submenu) = item else {
            continue;
        };
        for (index, child) in submenu.items()?.into_iter().enumerate() {
            let is_close_window = match &child {
                MenuItemKind::Predefined(p) => p.text()? == CLOSE_WINDOW_TEXT,
                _ => false,
            };
            if !is_close_window {
                continue;
            }
            let close_tab =
                MenuItem::with_id(app, CLOSE_TAB_ID, "Close Tab", true, Some("CmdOrCtrl+W"))?;
            submenu.remove_at(index)?;
            submenu.insert(&close_tab, index)?;
        }
    }
    Ok(())
}

/// Where the Edit menu is, and what is wanted of it. Generic over the submenu type so the
/// transitions can be tested without a menu.
///
/// THREADING RULE: the lock around this state is only ever held for the few lines that read or
/// change it, never across a call into Tauri's menu API. Those calls block a non-main thread until
/// the main thread has run them, and the main thread itself calls `set_session_keys` (window and
/// page-load events): holding the lock across them would deadlock the two.
struct Slot<T> {
    /// The Edit menu while it is out of the menu bar, with the position it had.
    out: Option<(usize, T)>,
    /// What the latest caller asked for (`true` = taken out).
    want: bool,
    /// One caller is applying changes; others only leave their wish in `want` and return, so
    /// the menu is never changed by two threads at once (no loss, no double insert).
    busy: bool,
}

impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            out: None,
            want: false,
            busy: false,
        }
    }
}

/// The next thing the caller that owns the slot has to do.
#[derive(Debug, PartialEq, Eq)]
enum Step<T> {
    Remove,
    Restore(usize, T),
    Done,
}

/// Records the wish. Returns whether this caller now owns the slot and must apply it.
fn request<T>(slot: &mut Slot<T>, captured: bool) -> bool {
    slot.want = captured;
    if slot.busy {
        return false;
    }
    slot.busy = true;
    true
}

/// What the owner does next. `Done` also releases the slot, in the same step, so a wish recorded
/// meanwhile is either seen here or starts a new owner.
fn next_step<T>(slot: &mut Slot<T>) -> Step<T> {
    match (slot.out.is_some(), slot.want) {
        (false, true) => Step::Remove,
        (true, false) => match slot.out.take() {
            Some((position, menu)) => Step::Restore(position, menu),
            None => {
                slot.busy = false;
                Step::Done
            }
        },
        _ => {
            slot.busy = false;
            Step::Done
        }
    }
}

/// The owner gives up after a failed change: the wish stays, the next call tries again.
fn abandon<T>(slot: &mut Slot<T>) {
    slot.busy = false;
}

static EDIT_MENU: Mutex<Slot<Submenu<Wry>>> = Mutex::new(Slot::new());

fn lock_slot() -> std::sync::MutexGuard<'static, Slot<Submenu<Wry>>> {
    EDIT_MENU
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// While the keyboard belongs to a graphical session (with the Mac shortcuts on), the Edit menu
/// leaves the menu bar: its key equivalents (⌘C, ⌘V, ⌘X, ⌘A, ⌘Z) would be taken by the menu
/// before the webview sees the key, and never reach the remote. `captured = false` puts the menu
/// back where it was. Calling it again with the same value does nothing. Failures are logged;
/// the menu is then left as it is and the next call tries again. Safe to call from any thread,
/// the main thread included (see `Slot`).
pub fn set_session_keys(app: &AppHandle<Wry>, captured: bool) {
    if !request(&mut lock_slot(), captured) {
        return;
    }
    loop {
        let step = next_step(&mut lock_slot());
        match step {
            Step::Done => return,
            Step::Remove => {
                let result = app
                    .menu()
                    .ok_or_else(|| tauri::Error::WindowNotFound)
                    .and_then(|menu| remove_edit_menu(&menu));
                match result {
                    Ok(Some(removed)) => lock_slot().out = Some(removed),
                    Ok(None) => {
                        tracing::warn!("no Edit menu found; nothing to take out");
                        abandon(&mut lock_slot());
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "could not take the Edit menu out");
                        abandon(&mut lock_slot());
                        return;
                    }
                }
            }
            Step::Restore(position, submenu) => {
                if let Err(e) = restore_edit_menu(app, position, &submenu) {
                    tracing::warn!(error = %e, "could not put the Edit menu back");
                    let mut slot = lock_slot();
                    slot.out = Some((position, submenu));
                    abandon(&mut slot);
                    return;
                }
            }
        }
    }
}

fn restore_edit_menu(
    app: &AppHandle<Wry>,
    position: usize,
    submenu: &Submenu<Wry>,
) -> tauri::Result<()> {
    let menu = app.menu().ok_or(tauri::Error::WindowNotFound)?;
    // Already there (never twice): nothing to insert.
    if menu.get(submenu.id()).is_some() {
        return Ok(());
    }
    // The position can be past the end if the menu changed meanwhile.
    let position = position.min(menu.items()?.len());
    menu.insert(submenu, position)
}

/// Safety net: the UI puts the Edit menu back itself, but a reload or a crash of the webview
/// (or the window losing the keyboard) would leave it out. These are the window events after
/// which the menu must be back, whatever the UI said last.
pub fn restores_on_window_event(label: &str, event: &WindowEvent) -> bool {
    label == "main" && matches!(event, WindowEvent::Focused(false) | WindowEvent::Destroyed)
}

/// Same, for a page load: a new page of the main webview knows nothing of an earlier capture.
pub fn restores_on_page_load(label: &str, event: PageLoadEvent) -> bool {
    label == "main" && matches!(event, PageLoadEvent::Started)
}

fn remove_edit_menu(menu: &Menu<Wry>) -> tauri::Result<Option<(usize, Submenu<Wry>)>> {
    for (index, item) in menu.items()?.into_iter().enumerate() {
        if let MenuItemKind::Submenu(submenu) = item {
            if submenu.text()? == EDIT_MENU_TEXT {
                menu.remove(&submenu)?;
                return Ok(Some((index, submenu)));
            }
        }
    }
    Ok(None)
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
        assert_eq!(event_for_menu_id(CLOSE_TAB_ID), Some(CLOSE_TAB_EVENT));
        assert_eq!(event_for_menu_id(SETTINGS_ID), Some(SETTINGS_EVENT));
        assert_eq!(
            event_for_menu_id(RELEASE_NOTES_ID),
            Some(RELEASE_NOTES_EVENT)
        );
        assert_eq!(event_for_menu_id("quit"), None);
        assert_eq!(event_for_menu_id("close_window"), None);
        assert_eq!(event_for_menu_id(""), None);
    }

    #[test]
    fn the_event_name_is_the_one_the_ui_listens_to() {
        let ui = include_str!("../../ui/src/lib/ipc/tauri.ts");
        assert!(ui.contains(&format!("\"{CHECK_FOR_UPDATES_EVENT}\"")));
        assert!(ui.contains(&format!("\"{CLOSE_TAB_EVENT}\"")));
        assert!(ui.contains(&format!("\"{SETTINGS_EVENT}\"")));
        assert!(ui.contains(&format!("\"{RELEASE_NOTES_EVENT}\"")));
    }

    #[test]
    fn the_edit_menu_comes_back_when_the_window_or_page_goes_away() {
        assert!(restores_on_window_event(
            "main",
            &WindowEvent::Focused(false)
        ));
        assert!(restores_on_window_event("main", &WindowEvent::Destroyed));
        assert!(!restores_on_window_event(
            "main",
            &WindowEvent::Focused(true)
        ));
        assert!(!restores_on_window_event(
            "web-1",
            &WindowEvent::Focused(false)
        ));
        assert!(restores_on_page_load("main", PageLoadEvent::Started));
        assert!(!restores_on_page_load("main", PageLoadEvent::Finished));
        assert!(!restores_on_page_load("web-1", PageLoadEvent::Started));
    }

    #[test]
    fn the_slot_moves_the_edit_menu_out_and_back_one_owner_at_a_time() {
        let mut slot: Slot<u8> = Slot::new();
        // Asking to take it out: this caller owns the slot, and a second one only leaves a wish.
        assert!(request(&mut slot, true));
        assert!(!request(&mut slot, true));
        assert_eq!(next_step(&mut slot), Step::Remove);
        slot.out = Some((3, 7));
        // A wish to put it back arrives while the owner is working: the owner sees it.
        assert!(!request(&mut slot, false));
        assert_eq!(next_step(&mut slot), Step::Restore(3, 7));
        // Back in the bar and nothing else wanted: done, the slot is free again.
        assert_eq!(next_step(&mut slot), Step::Done);
        assert!(request(&mut slot, false));
        assert_eq!(next_step(&mut slot), Step::Done);
    }

    #[test]
    fn a_wish_that_matches_the_state_does_nothing_and_a_failure_frees_the_slot() {
        let mut slot: Slot<u8> = Slot::new();
        assert!(request(&mut slot, false));
        assert_eq!(next_step(&mut slot), Step::Done);
        assert!(request(&mut slot, true));
        assert_eq!(next_step(&mut slot), Step::Remove);
        abandon(&mut slot);
        // The wish is still there: the next owner tries again.
        assert!(request(&mut slot, true));
        assert_eq!(next_step(&mut slot), Step::Remove);
        slot.out = Some((1, 1));
        assert_eq!(next_step(&mut slot), Step::Done);
        assert!(request(&mut slot, true));
    }
}
