//! Language server connection as an iced subscription.
//!
//! Connecting blocks (discovery, spawning Godot, handshake), so it runs on a
//! dedicated thread that then becomes the message reader.

use std::path::PathBuf;

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, StreamExt};
use iced::Subscription;
use lsp_client::{godot, Client, Event, Server, StartOptions};

#[derive(Debug, Clone)]
pub enum LspEvent {
    /// `warning` says when the spawned Godot doesn't match the project's version.
    Connected { client: Client, label: String, warning: Option<String> },
    Failed(String),
    Event(Event),
}

/// Connection state shown in the status bar.
pub enum Lsp {
    Off,
    Starting,
    Ready { client: Client, label: String, warning: Option<String> },
    /// The error itself is shown in the status message.
    Failed,
    /// Seconds until the next automatic reconnect, if one is scheduled.
    Disconnected { retry_in: Option<u64> },
}

impl Lsp {
    pub fn client(&self) -> Option<&Client> {
        match self {
            Lsp::Ready { client, .. } => Some(client),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Lsp::Off => "LSP off".to_string(),
            Lsp::Starting => "LSP starting...".to_string(),
            Lsp::Ready { label, warning: None, .. } => format!("LSP: {label}"),
            Lsp::Ready { label, warning: Some(_), .. } => format!("LSP: {label} (version mismatch)"),
            Lsp::Failed => "LSP failed (click to retry)".to_string(),
            Lsp::Disconnected { retry_in: Some(seconds) } => format!("LSP disconnected, retrying in {seconds}s"),
            Lsp::Disconnected { retry_in: None } => "LSP disconnected (click to retry)".to_string(),
        }
    }

    pub fn warning(&self) -> Option<&str> {
        match self {
            Lsp::Ready { warning, .. } => warning.as_deref(),
            _ => None,
        }
    }
}

/// Identifies a connection; changing any field (e.g. `generation` on
/// restart) replaces it with a fresh one.
#[derive(Debug, Clone, Hash)]
struct Connection {
    root: PathBuf,
    wanted_version: Option<String>,
    generation: u64,
}

pub fn connect(root: PathBuf, wanted_version: Option<String>, generation: u64) -> Subscription<LspEvent> {
    let connection = Connection { root, wanted_version, generation };
    Subscription::run_with(connection, |connection| {
        let Connection { root, wanted_version, .. } = connection.clone();
        iced::stream::channel(64, async move |mut output| {
            let (tx, mut rx) = mpsc::unbounded();
            std::thread::spawn(move || {
                let installs = godot::discover();
                let install = godot::pick(&installs, wanted_version.as_deref()).cloned();
                let options = StartOptions {
                    godot: install.as_ref().map(|i| i.path.clone()),
                    ..StartOptions::default()
                };
                match lsp_client::start(&root, &options) {
                    Ok((client, reader)) => {
                        let label = describe(client.server(), install.as_ref());
                        let warning = version_warning(client.server(), install.as_ref(), wanted_version.as_deref());
                        if tx.unbounded_send(LspEvent::Connected { client, label, warning }).is_ok() {
                            reader.run(|event| {
                                let _ = tx.unbounded_send(LspEvent::Event(event));
                            });
                        }
                    }
                    Err(error) => {
                        let _ = tx.unbounded_send(LspEvent::Failed(error));
                    }
                }
            });
            while let Some(event) = rx.next().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
            std::future::pending::<()>().await;
        })
    })
}

/// An attached editor already has the project open, so only a spawned
/// server can be the wrong version.
fn version_warning(server: &Server, install: Option<&godot::GodotInstall>, wanted: Option<&str>) -> Option<String> {
    let (Server::Spawned { .. }, Some(install), Some(wanted)) = (server, install, wanted) else {
        return None;
    };
    if godot::matches(install, Some(wanted)) {
        return None;
    }
    let used = install.version_string().unwrap_or_else(|| "?".to_string());
    Some(format!("No Godot {wanted} found, using {used} (set GODOT_BIN to choose)"))
}

fn describe(server: &Server, install: Option<&godot::GodotInstall>) -> String {
    match server {
        Server::Attached { port } => format!("Godot editor :{port}"),
        Server::Spawned { port, .. } => {
            let version = install
                .and_then(godot::GodotInstall::version_string)
                .unwrap_or_else(|| "?".to_string());
            format!("Godot {version} headless :{port}")
        }
    }
}
