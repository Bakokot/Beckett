//! Spawning the integrated terminal: shell choice, a `godot` command on
//! PATH, and the subscription that carries its events.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, StreamExt};
use iced::Subscription;
use lsp_client::godot;
use terminal::{Options, Palette, Rgb, Terminal};

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Ready(Arc<Terminal>),
    /// New output to draw.
    Wakeup,
    Title(String),
    Exit,
    Failed(String),
}

/// Identifies one shell session; a new `generation` starts a fresh one.
#[derive(Debug, Clone)]
struct Session {
    generation: u64,
    cwd: PathBuf,
    wanted_version: Option<String>,
    /// The Godot chosen for the project. Not part of the identity: choosing
    /// another doesn't end the running shell; the next one uses it.
    godot: Option<PathBuf>,
}

impl std::hash::Hash for Session {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.generation, &self.cwd, &self.wanted_version).hash(state);
    }
}

pub fn spawn(generation: u64, cwd: PathBuf, wanted_version: Option<String>, godot: Option<PathBuf>) -> Subscription<TerminalEvent> {
    let session = Session { generation, cwd, wanted_version, godot };
    Subscription::run_with(session, |session| {
        let session = session.clone();
        iced::stream::channel(64, async move |mut output| {
            let (tx, mut rx) = mpsc::unbounded();
            std::thread::spawn(move || {
                let options = shell_options(&session);
                let events = tx.clone();
                let spawned = Terminal::spawn(options, 80, 24, move |event| {
                    let event = match event {
                        terminal::Event::Wakeup => TerminalEvent::Wakeup,
                        terminal::Event::Title(title) => TerminalEvent::Title(title),
                        terminal::Event::Exit => TerminalEvent::Exit,
                    };
                    let _ = events.unbounded_send(event);
                });
                let _ = tx.unbounded_send(match spawned {
                    Ok(terminal) => TerminalEvent::Ready(Arc::new(terminal)),
                    Err(error) => TerminalEvent::Failed(error.to_string()),
                });
            });

            while let Some(first) = rx.next().await {
                // Coalesce bursts of output into a single redraw.
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                let mut woke = false;
                for event in batch {
                    if matches!(event, TerminalEvent::Wakeup) {
                        woke = true;
                    } else if output.send(event).await.is_err() {
                        return;
                    }
                }
                if woke && output.send(TerminalEvent::Wakeup).await.is_err() {
                    return;
                }
            }
            std::future::pending::<()>().await;
        })
    })
}

pub fn palette(theme: &theme::Theme) -> Palette {
    let rgb = |c: theme::Color| Rgb::new(c.r, c.g, c.b);
    Palette {
        ansi: theme.terminal.ansi().map(rgb),
        foreground: rgb(theme.ui.text),
        background: rgb(theme.ui.background),
        cursor: rgb(theme.ui.text),
    }
}

fn shell_options(session: &Session) -> Options {
    let mut env_vars = HashMap::new();
    if let Some(install) = godot::resolve(session.godot.as_deref(), session.wanted_version.as_deref()) {
        let exe = godot::console_variant(&install.path);
        env_vars.insert("GODOT".to_string(), exe.display().to_string());
        if let Some(dir) = write_shim(&exe) {
            let path = env::var_os("PATH").unwrap_or_default();
            let mut dirs = vec![dir];
            dirs.extend(env::split_paths(&path));
            if let Ok(joined) = env::join_paths(dirs) {
                env_vars.insert("PATH".to_string(), joined.to_string_lossy().into_owned());
            }
        }
    }
    Options {
        working_directory: Some(session.cwd.clone()),
        shell: default_shell(),
        env: env_vars,
    }
}

/// PowerShell 7 when installed, Windows PowerShell otherwise; the user's
/// login shell elsewhere.
fn default_shell() -> Option<(String, Vec<String>)> {
    if !cfg!(windows) {
        return None;
    }
    let path = env::var_os("PATH").unwrap_or_default();
    let has_pwsh = env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file());
    let program = if has_pwsh { "pwsh.exe" } else { "powershell.exe" };
    Some((program.to_string(), vec!["-NoLogo".to_string()]))
}

/// Writes a `godot` launcher into the IDE's data folder and returns that
/// folder, so `godot` works in the terminal whatever the binary is called.
fn write_shim(exe: &Path) -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("godot-ide").join("bin");
    std::fs::create_dir_all(&dir).ok()?;
    if cfg!(windows) {
        let script = format!("@echo off\r\n\"{}\" %*\r\n", exe.display());
        std::fs::write(dir.join("godot.cmd"), script).ok()?;
    } else {
        let shim = dir.join("godot");
        std::fs::write(&shim, format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", exe.display())).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).ok()?;
        }
    }
    Some(dir)
}
