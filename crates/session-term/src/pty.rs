//! Local shell through `portable-pty`.
//!
//! Threads (the PTY API is blocking): one reads the master and feeds the forwarder queue, one
//! waits for the child. The exit event is queued only after the reader reached end of file, so
//! it always follows the last byte of output. If a grandchild keeps the PTY open after the
//! shell exited, the master is dropped after a grace period so the exit is still reported.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use portolano_core_model::{AppError, ErrorCode};
use tokio::sync::mpsc;

use crate::flow::Source;
use crate::session::LocalSpec;

const READ_CHUNK: usize = 16 * 1024;
/// How long the waiter lets the reader drain after the child exited.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

type SharedMaster = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

/// Control side of a running local shell.
pub(crate) struct PtyControl {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: SharedMaster,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn poisoned() -> AppError {
    AppError::internal("terminal state is poisoned")
}

fn io_error(what: &str, e: &std::io::Error) -> AppError {
    let code = if e.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::Io
    };
    AppError::new(code, format!("{what}: {e}"))
}

/// The user's login shell on macOS/Linux (`$SHELL`, zsh as fallback), PowerShell on Windows.
fn default_program() -> (String, Vec<String>) {
    default_program_for(cfg!(windows), std::env::var("SHELL").ok().as_deref())
}

fn default_program_for(windows: bool, shell_env: Option<&str>) -> (String, Vec<String>) {
    if windows {
        ("powershell.exe".to_string(), vec!["-NoLogo".to_string()])
    } else {
        let shell = shell_env
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("/bin/zsh")
            .to_string();
        // `-l`: a login shell, like the system Terminal, so profiles are read.
        (shell, vec!["-l".to_string()])
    }
}

fn utf8_locale_missing() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            !(v.contains("utf-8") || v.contains("utf8"))
        })
        .unwrap_or(true)
}

fn command(spec: &LocalSpec) -> Result<CommandBuilder, AppError> {
    let (program, default_args) = match spec.program.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => (p.to_string(), spec.args.clone()),
        _ => {
            let (p, a) = default_program();
            // Explicit arguments replace the default ones.
            (
                p,
                if spec.args.is_empty() {
                    a
                } else {
                    spec.args.clone()
                },
            )
        }
    };
    let mut cmd = CommandBuilder::new(program);
    cmd.args(default_args);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if !cfg!(windows) && utf8_locale_missing() {
        // A GUI application does not inherit the shell's locale: without this, accented
        // characters break in the shell and in line editors.
        cmd.env("LANG", "en_US.UTF-8");
    }
    let cwd: Option<PathBuf> = match spec.cwd.as_deref().map(str::trim) {
        Some(c) if !c.is_empty() => {
            let path = PathBuf::from(c);
            if !path.is_dir() {
                return Err(AppError::invalid_input(format!(
                    "the working directory {c} does not exist"
                )));
            }
            Some(path)
        }
        _ => dirs::home_dir().filter(|p| p.is_dir()),
    };
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    Ok(cmd)
}

/// Spawns the shell and starts the reader and waiter threads.
pub(crate) fn spawn(
    spec: &LocalSpec,
    cols: u16,
    rows: u16,
    tx: mpsc::Sender<Source>,
) -> Result<PtyControl, AppError> {
    let cmd = command(spec)?;
    let pair = native_pty_system()
        .openpty(size(cols, rows))
        .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot open a pseudo-terminal: {e}")))?;
    let mut child =
        pair.slave
            .spawn_command(cmd)
            .map_err(|e| match e.downcast_ref::<std::io::Error>() {
                Some(io) => io_error("cannot start the shell", io),
                None => AppError::new(ErrorCode::Io, format!("cannot start the shell: {e}")),
            })?;
    // The parent must not keep the slave open, or the reader would never see end of file.
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot read the terminal: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot write the terminal: {e}")))?;
    let killer = child.clone_killer();
    let master: SharedMaster = Arc::new(Mutex::new(Some(pair.master)));

    let (done_tx, done_rx) = std_mpsc::channel::<()>();
    let reader_tx = tx.clone();
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; READ_CHUNK];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if reader_tx
                            .blocking_send(Source::Data(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        // EIO is how a PTY master reports that the slave side is gone.
                        tracing::debug!("pty read ended: {e}");
                        break;
                    }
                }
            }
            if done_tx.send(()).is_err() {
                tracing::debug!("pty waiter is gone");
            }
        })
        .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot start a thread: {e}")))?;

    let waiter_master = master.clone();
    std::thread::Builder::new()
        .name("pty-waiter".into())
        .spawn(move || {
            let code = match child.wait() {
                Ok(status) if status.signal().is_none() => i32::try_from(status.exit_code()).ok(),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!("waiting for the shell failed: {e}");
                    None
                }
            };
            if done_rx.recv_timeout(DRAIN_GRACE).is_err() {
                // Output is still open (a background process holds the PTY): close our side.
                if let Ok(mut m) = waiter_master.lock() {
                    m.take();
                }
                if done_rx.recv_timeout(DRAIN_GRACE).is_err() {
                    tracing::debug!("pty reader did not stop after the master was closed");
                }
            }
            if let Ok(mut m) = waiter_master.lock() {
                m.take();
            }
            if tx.blocking_send(Source::Exit(code)).is_err() {
                tracing::debug!("session closed before the exit status was delivered");
            }
        })
        .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot start a thread: {e}")))?;

    Ok(PtyControl {
        writer: Arc::new(Mutex::new(writer)),
        master,
        killer: Mutex::new(killer),
    })
}

impl PtyControl {
    /// Blocking write of the whole buffer (call from `spawn_blocking`).
    pub(crate) fn write_blocking(
        writer: &Arc<Mutex<Box<dyn Write + Send>>>,
        data: &[u8],
    ) -> Result<(), AppError> {
        let mut w = writer.lock().map_err(|_| poisoned())?;
        w.write_all(data)
            .and_then(|()| w.flush())
            .map_err(|e| io_error("cannot write to the terminal", &e))
    }

    pub(crate) fn writer(&self) -> Arc<Mutex<Box<dyn Write + Send>>> {
        self.writer.clone()
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        let guard = self.master.lock().map_err(|_| poisoned())?;
        match guard.as_ref() {
            Some(master) => master
                .resize(size(cols, rows))
                .map_err(|e| AppError::new(ErrorCode::Io, format!("cannot resize: {e}"))),
            None => Ok(()),
        }
    }

    /// Hangs up the child (SIGHUP, TerminateProcess on Windows) and closes the master, which
    /// also hangs up the foreground process group. An already exited child is not an error.
    pub(crate) fn kill(&self) {
        match self.killer.lock() {
            Ok(mut k) => {
                if let Err(e) = k.kill() {
                    tracing::debug!("kill: {e}");
                }
            }
            Err(_) => tracing::warn!("cannot kill the shell: state is poisoned"),
        }
        if let Ok(mut m) = self.master.lock() {
            m.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_is_the_login_shell_or_zsh() {
        assert_eq!(
            default_program_for(false, Some("/bin/bash")),
            ("/bin/bash".to_string(), vec!["-l".to_string()])
        );
        assert_eq!(default_program_for(false, Some("  ")).0, "/bin/zsh");
        assert_eq!(default_program_for(false, None).0, "/bin/zsh");
    }

    #[test]
    fn default_shell_on_windows_is_powershell() {
        assert_eq!(
            default_program_for(true, Some("/bin/bash")).0,
            "powershell.exe"
        );
    }

    #[test]
    fn explicit_program_and_args_are_used_as_given() {
        let spec = LocalSpec {
            program: Some("/bin/sh".into()),
            args: vec!["-c".into(), "true".into()],
            cwd: None,
        };
        assert!(command(&spec).is_ok());
    }

    #[test]
    fn a_missing_working_directory_is_invalid_input() {
        let spec = LocalSpec {
            program: Some("/bin/sh".into()),
            args: vec![],
            cwd: Some("/nonexistent/portolano-wp5".into()),
        };
        let err = command(&spec).expect_err("must fail");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }
}
