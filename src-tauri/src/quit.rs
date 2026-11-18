//! Confirmation before the application quits or its window closes.
//!
//! The UI knows how many sessions are connected, so Rust only stops the exit, asks the UI
//! (`quit-requested`) and waits for its answer (`quit_reply`). The UI answers at once when it has
//! nothing to ask; if it does not answer within [`REPLY_TIMEOUT`] (frozen or reloading
//! interface) the application quits anyway, so a stuck UI can never keep it open.

use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// Event sent to the UI (no payload) when the user asks to quit or to close the window.
pub const QUIT_REQUESTED_EVENT: &str = "quit-requested";

/// How long the UI has to answer before the application quits anyway.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether an `ExitRequested` is the user's quit (`None`) and so goes through the confirmation.
/// `Some(code)` is our own `exit(0)` or the updater's restart (`i32::MAX`): never blocked.
pub fn intercepts_exit(code: Option<i32>) -> bool {
    code.is_none()
}

/// What the UI answers to `quit-requested`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reply {
    /// The UI shows a confirmation: stop the timer and wait for the user.
    Asking,
    /// Nothing to ask, or the user confirmed: quit.
    Quit,
    /// The user cancelled: stay open.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// No request in progress.
    #[default]
    Idle,
    /// The UI was asked; the timer runs.
    Waiting,
    /// The UI showed its confirmation; the timer is stopped.
    Asking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    pub phase: Phase,
    /// Number of the current request: a timer only counts for the request that started it.
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// Quit or window close requested by the user.
    Request,
    Reply(Reply),
    /// The timer of the request with this number expired.
    Timeout(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Emit `quit-requested` and start the timer of this request.
    Ask(u64),
    /// Quit the application for good.
    Exit,
}

/// The whole decision logic, without Tauri. Anything that does not fit the current phase (a
/// reply with no request, a late timer, a second request) changes nothing.
pub fn next(state: State, input: Input) -> (State, Action) {
    let idle = State {
        phase: Phase::Idle,
        ..state
    };
    match (state.phase, input) {
        // While asking, a new request means the UI may have been reloaded or hung: ask again.
        (Phase::Idle | Phase::Asking, Input::Request) => {
            let generation = state.generation.wrapping_add(1);
            (
                State {
                    phase: Phase::Waiting,
                    generation,
                },
                Action::Ask(generation),
            )
        }
        (Phase::Waiting, Input::Reply(Reply::Asking)) => (
            State {
                phase: Phase::Asking,
                ..state
            },
            Action::None,
        ),
        (Phase::Waiting | Phase::Asking, Input::Reply(Reply::Quit)) => (idle, Action::Exit),
        (Phase::Waiting | Phase::Asking, Input::Reply(Reply::Cancel)) => (idle, Action::None),
        (Phase::Waiting, Input::Timeout(generation)) if generation == state.generation => {
            (idle, Action::Exit)
        }
        _ => (state, Action::None),
    }
}

/// The state machine plus its effects. Managed by Tauri.
#[derive(Default)]
pub struct Quit {
    state: Mutex<State>,
}

impl Quit {
    /// The user asked to quit or to close the window; the caller has stopped it already.
    pub fn request(&self, app: &AppHandle) {
        self.apply(app, Input::Request);
    }

    fn reply(&self, app: &AppHandle, reply: Reply) {
        self.apply(app, Input::Reply(reply));
    }

    fn apply(&self, app: &AppHandle, input: Input) {
        let action = {
            // A poisoned lock still holds a usable state: quitting must keep working.
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let (new_state, action) = next(*state, input);
            *state = new_state;
            action
        };
        match action {
            Action::None => {}
            // `exit(0)` comes back as `ExitRequested { code: Some(0) }`, which is let through.
            Action::Exit => {
                #[cfg(target_os = "macos")]
                crate::quit_macos::allow_terminate();
                app.exit(0);
            }
            Action::Ask(generation) => {
                // The timer first, so that a UI that never answers cannot keep the app open.
                start_timer(app.clone(), generation);
                if let Err(e) = app.emit_to("main", QUIT_REQUESTED_EVENT, ()) {
                    tracing::warn!(error = %e, "quit request not delivered; the timer will quit");
                }
            }
        }
    }
}

fn start_timer(app: AppHandle, generation: u64) {
    let spawned = std::thread::Builder::new()
        .name("quit-timer".into())
        .spawn({
            let app = app.clone();
            move || {
                std::thread::sleep(REPLY_TIMEOUT);
                if let Some(quit) = app.try_state::<Quit>() {
                    quit.apply(&app, Input::Timeout(generation));
                }
            }
        });
    if let Err(e) = spawned {
        // No timer means no safety net: do not ask, quit as before.
        tracing::error!(error = %e, "could not start the quit timer; quitting");
        app.exit(0);
    }
}

/// The UI's answer to `quit-requested`.
#[tauri::command]
pub fn quit_reply(app: AppHandle, quit: TauriState<'_, Quit>, reply: Reply) {
    quit.reply(&app, reply);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> State {
        State::default()
    }

    #[test]
    fn only_a_user_quit_is_intercepted_not_our_exit_nor_the_updater_restart() {
        assert!(intercepts_exit(None));
        assert!(!intercepts_exit(Some(0)));
        assert!(!intercepts_exit(Some(i32::MAX)));
    }

    #[test]
    fn a_request_asks_the_ui_and_starts_waiting() {
        let (state, action) = next(idle(), Input::Request);
        assert_eq!(state.phase, Phase::Waiting);
        assert_eq!(action, Action::Ask(state.generation));
    }

    #[test]
    fn a_second_request_while_waiting_is_ignored() {
        let (waiting, _) = next(idle(), Input::Request);
        let (again, action) = next(waiting, Input::Request);
        assert_eq!((again, action), (waiting, Action::None));
    }

    #[test]
    fn a_request_while_asking_is_a_fresh_request() {
        // The UI may have been reloaded (no dialog any more) or hung: ask again with a new
        // timer; the old timer is stale.
        let (waiting, _) = next(idle(), Input::Request);
        let (asking, _) = next(waiting, Input::Reply(Reply::Asking));
        let (again, action) = next(asking, Input::Request);
        assert_eq!(again.phase, Phase::Waiting);
        assert_ne!(again.generation, asking.generation);
        assert_eq!(action, Action::Ask(again.generation));
        let (after, action) = next(again, Input::Timeout(asking.generation));
        assert_eq!((after, action), (again, Action::None));
        let (after, action) = next(again, Input::Timeout(again.generation));
        assert_eq!((after.phase, action), (Phase::Idle, Action::Exit));
    }

    #[test]
    fn asking_stops_the_timer_so_the_timeout_no_longer_quits() {
        let (waiting, action) = next(idle(), Input::Request);
        let Action::Ask(generation) = action else {
            panic!("expected Ask, got {action:?}");
        };
        let (asking, action) = next(waiting, Input::Reply(Reply::Asking));
        assert_eq!((asking.phase, action), (Phase::Asking, Action::None));
        let (after, action) = next(asking, Input::Timeout(generation));
        assert_eq!((after, action), (asking, Action::None));
    }

    #[test]
    fn quit_exits_from_waiting_and_from_asking() {
        let (waiting, _) = next(idle(), Input::Request);
        let (state, action) = next(waiting, Input::Reply(Reply::Quit));
        assert_eq!((state.phase, action), (Phase::Idle, Action::Exit));
        let (asking, _) = next(waiting, Input::Reply(Reply::Asking));
        let (state, action) = next(asking, Input::Reply(Reply::Quit));
        assert_eq!((state.phase, action), (Phase::Idle, Action::Exit));
    }

    #[test]
    fn cancel_goes_back_to_idle_and_a_later_request_asks_again() {
        let (waiting, _) = next(idle(), Input::Request);
        let (asking, _) = next(waiting, Input::Reply(Reply::Asking));
        let (state, action) = next(asking, Input::Reply(Reply::Cancel));
        assert_eq!((state.phase, action), (Phase::Idle, Action::None));
        let (state, action) = next(state, Input::Request);
        assert_eq!(state.phase, Phase::Waiting);
        assert_eq!(action, Action::Ask(state.generation));
    }

    #[test]
    fn the_timeout_quits_when_nobody_answered() {
        let (waiting, action) = next(idle(), Input::Request);
        let Action::Ask(generation) = action else {
            panic!("expected Ask, got {action:?}");
        };
        let (state, action) = next(waiting, Input::Timeout(generation));
        assert_eq!((state.phase, action), (Phase::Idle, Action::Exit));
    }

    #[test]
    fn a_timeout_of_an_older_request_does_nothing() {
        let (first, action) = next(idle(), Input::Request);
        let Action::Ask(old) = action else {
            panic!("expected Ask, got {action:?}");
        };
        let (cancelled, _) = next(first, Input::Reply(Reply::Cancel));
        let (second, _) = next(cancelled, Input::Request);
        assert_ne!(second.generation, old);
        let (after, action) = next(second, Input::Timeout(old));
        assert_eq!((after, action), (second, Action::None));
    }

    #[test]
    fn replies_without_a_request_are_ignored() {
        for reply in [Reply::Asking, Reply::Quit, Reply::Cancel] {
            let (state, action) = next(idle(), Input::Reply(reply));
            assert_eq!((state, action), (idle(), Action::None), "{reply:?}");
        }
        let (state, action) = next(idle(), Input::Timeout(0));
        assert_eq!((state, action), (idle(), Action::None));
    }

    #[test]
    fn replies_parse_from_the_names_the_ui_sends() {
        for (text, reply) in [
            ("\"asking\"", Reply::Asking),
            ("\"quit\"", Reply::Quit),
            ("\"cancel\"", Reply::Cancel),
        ] {
            assert_eq!(serde_json::from_str::<Reply>(text).expect(text), reply);
        }
        assert!(serde_json::from_str::<Reply>("\"later\"").is_err());
    }
}
