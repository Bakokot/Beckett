//! Client for Godot's built-in GDScript language server.
//!
//! [`start`] attaches to a running Godot editor for the same project, or
//! spawns a headless one. It returns a [`Client`] for sending and a
//! [`Reader`] that turns incoming messages into [`Event`]s on its thread.

pub mod dap;
pub mod godot;
#[cfg(windows)]
pub mod job;
mod protocol;
mod types;
pub mod uri;

pub use types::{
    completion_item, completion_items, document_symbols, hover_text, locations, native_classes, native_symbol, range,
    signature_help, workspace_edit, NativeClass, NativeSymbol, Symbol, SymbolKind,
    CompletionItem, CompletionKind, Diagnostic, Location, Position, Range, Severity, SignatureHelp,
    TextEdit,
};

use std::fmt;
use std::io::{self, BufReader};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::uri::{path_to_uri, same_path};

/// The Godot editor's default language server port.
pub const DEFAULT_PORT: u16 = 6005;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const SPAWN_TIMEOUT: Duration = Duration::from_secs(60);
/// Ports tried for a spawned server, clear of the editor default.
const SPAWN_PORTS: std::ops::Range<u16> = 6010..6200;

#[derive(Debug, Clone)]
pub struct StartOptions {
    /// Try a running Godot editor on this port first.
    pub attach_port: Option<u16>,
    /// Executable used to spawn a headless server when attaching fails.
    pub godot: Option<PathBuf>,
    /// Keep trying to attach for this long (a Godot editor that was just
    /// started), and never spawn: a second Godot on the same project
    /// fights it over files such as GDExtension libraries.
    pub wait_for_editor: Option<Duration>,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            attach_port: Some(DEFAULT_PORT),
            godot: None,
            wait_for_editor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    /// A Godot editor the user already had open.
    Attached { port: u16 },
    /// A headless Godot we started; it dies with the last [`Client`].
    /// `dap_port` is its debug adapter.
    Spawned { port: u16, dap_port: u16, godot: PathBuf },
}

/// The Godot editor's default debug adapter port (Editor Settings >
/// Network > Debug Adapter).
pub const EDITOR_DAP_PORT: u16 = 6006;

impl Server {
    /// Where this Godot's debug adapter listens.
    pub fn dap_port(&self) -> u16 {
        match self {
            Server::Attached { .. } => EDITOR_DAP_PORT,
            Server::Spawned { dap_port, .. } => *dap_port,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// The engine classes (sent once, after the handshake).
    NativeClasses(Vec<NativeClass>),
    Disconnected(Option<String>),
}

/// Cheap to clone; dropping the last clone closes the connection and stops
/// a spawned server.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    writer: Arc<Mutex<TcpStream>>,
    next_id: AtomicU64,
    server: Server,
    process: Option<ServerProcess>,
}

struct ServerProcess {
    child: Mutex<Child>,
    #[cfg(windows)]
    _job: Option<job::Job>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("server", &self.inner.server)
            .finish_non_exhaustive()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writer.shutdown(Shutdown::Both);
        if let Some(process) = &self.process {
            let mut child = process.child.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Client {
    pub fn server(&self) -> &Server {
        &self.inner.server
    }

    pub fn request(&self, method: &str, params: Value) -> io::Result<u64> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> io::Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    pub fn did_open(&self, path: &Path, version: i32, text: &str) -> io::Result<()> {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": path_to_uri(path), "languageId": "gdscript", "version": version, "text": text,
            }}),
        )
    }

    /// Full-document sync, which is what Godot advertises.
    pub fn did_change(&self, path: &Path, version: i32, text: &str) -> io::Result<()> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": path_to_uri(path), "version": version },
                "contentChanges": [{ "text": text }],
            }),
        )
    }

    pub fn did_save(&self, path: &Path, text: &str) -> io::Result<()> {
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": path_to_uri(path) }, "text": text }),
        )
    }

    pub fn did_close(&self, path: &Path) -> io::Result<()> {
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": path_to_uri(path) } }),
        )
    }

    /// Returns the request id; the answer arrives as [`Event::Response`]
    /// and decodes with [`locations`].
    pub fn definition(&self, path: &Path, position: Position) -> io::Result<u64> {
        self.request(
            "textDocument/definition",
            json!({ "textDocument": { "uri": path_to_uri(path) }, "position": position.to_json() }),
        )
    }

    /// `trigger` is the character that opened completion, if any (e.g. `.`).
    pub fn completion(&self, path: &Path, position: Position, trigger: Option<char>) -> io::Result<u64> {
        let context = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": 1 }),
        };
        self.request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": path_to_uri(path) },
                "position": position.to_json(),
                "context": context,
            }),
        )
    }

    /// Asks for an item's documentation; decode the answer with [`completion_item`].
    pub fn resolve_completion(&self, item: &CompletionItem) -> io::Result<u64> {
        self.request("completionItem/resolve", item.raw.clone())
    }

    /// Decode the answer with [`hover_text`].
    pub fn hover(&self, path: &Path, position: Position) -> io::Result<u64> {
        self.request(
            "textDocument/hover",
            json!({ "textDocument": { "uri": path_to_uri(path) }, "position": position.to_json() }),
        )
    }

    /// Decode the answer with [`signature_help`].
    pub fn signature_help(&self, path: &Path, position: Position) -> io::Result<u64> {
        self.request(
            "textDocument/signatureHelp",
            json!({ "textDocument": { "uri": path_to_uri(path) }, "position": position.to_json() }),
        )
    }

    /// An engine class (`symbol` = `class`) or one of its members, with its
    /// documentation. Decode the answer with [`native_symbol`].
    pub fn native_symbol(&self, class: &str, symbol: &str) -> io::Result<u64> {
        self.request("textDocument/nativeSymbol", json!({ "native_class": class, "symbol_name": symbol }))
    }

    /// The file's symbols (functions, variables, signals…). Decode the
    /// answer with [`document_symbols`].
    pub fn document_symbols(&self, path: &Path) -> io::Result<u64> {
        self.request("textDocument/documentSymbol", json!({ "textDocument": { "uri": path_to_uri(path) } }))
    }

    /// Every use of the symbol at `position`, declaration included. Decode
    /// the answer with [`locations`].
    pub fn references(&self, path: &Path, position: Position) -> io::Result<u64> {
        self.request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": path_to_uri(path) },
                "position": position.to_json(),
                "context": { "includeDeclaration": true },
            }),
        )
    }

    /// Whether the symbol at `position` can be renamed: the answer is its
    /// range (decode with [`range`]), or null.
    pub fn prepare_rename(&self, path: &Path, position: Position) -> io::Result<u64> {
        self.request(
            "textDocument/prepareRename",
            json!({ "textDocument": { "uri": path_to_uri(path) }, "position": position.to_json() }),
        )
    }

    /// Decode the answer with [`workspace_edit`].
    pub fn rename(&self, path: &Path, position: Position, new_name: &str) -> io::Result<u64> {
        self.request(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": path_to_uri(path) },
                "position": position.to_json(),
                "newName": new_name,
            }),
        )
    }

    fn send(&self, message: &Value) -> io::Result<()> {
        let mut writer = self.inner.writer.lock().unwrap_or_else(PoisonError::into_inner);
        protocol::write_message(&mut *writer, message)
    }
}

/// Reads server messages. Holds no [`Client`], so it never keeps a spawned
/// server alive on its own.
pub struct Reader {
    reader: BufReader<TcpStream>,
    writer: Arc<Mutex<TcpStream>>,
}

impl Reader {
    /// Blocks until the connection closes, reporting each event.
    pub fn run(mut self, mut on_event: impl FnMut(Event)) {
        loop {
            match protocol::read_message(&mut self.reader) {
                Ok(Some(message)) => {
                    if let Some(event) = self.dispatch(message) {
                        on_event(event);
                    }
                }
                Ok(None) => return on_event(Event::Disconnected(None)),
                Err(error) => return on_event(Event::Disconnected(Some(error.to_string()))),
            }
        }
    }

    fn dispatch(&mut self, message: Value) -> Option<Event> {
        let method = message.get("method").and_then(Value::as_str);
        match (method, message.get("id")) {
            // Server-to-client request: we support none, so answer null.
            (Some(_), Some(id)) => {
                let reply = json!({ "jsonrpc": "2.0", "id": id, "result": null });
                let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
                let _ = protocol::write_message(&mut *writer, &reply);
                None
            }
            (None, Some(id)) => {
                let result = match message.get("error") {
                    Some(error) => Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("request failed")
                        .to_string()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                Some(Event::Response { id: id.as_u64()?, result })
            }
            (Some("textDocument/publishDiagnostics"), None) => {
                let (path, diagnostics) = types::diagnostics(message.get("params")?)?;
                Some(Event::Diagnostics { path, diagnostics })
            }
            // Godot's full class list, for the class reference.
            (Some("gdscript/capabilities"), None) => Some(Event::NativeClasses(types::native_classes(message.get("params")?))),
            (Some(method), None) => Some(Event::Notification {
                method: method.to_string(),
                params: message.get("params").cloned().unwrap_or(Value::Null),
            }),
            (None, None) => None,
        }
    }
}

/// Connects a language server for the project at `root`. Blocks for the
/// handshake, and up to a minute when a headless Godot has to start.
pub fn start(root: &Path, options: &StartOptions) -> Result<(Client, Reader), String> {
    if let Some(port) = options.attach_port {
        let deadline = Instant::now() + options.wait_for_editor.unwrap_or_default();
        loop {
            if let Some(attached) = attach(root, port) {
                return Ok(attached);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }
        if options.wait_for_editor.is_some() {
            return Err(format!("The Godot editor's language server did not answer on port {port}"));
        }
    }

    let godot = options
        .godot
        .as_deref()
        .ok_or("No Godot executable found (set GODOT_BIN)")?;
    let (stream, server, process) = spawn_server(godot, root)?;
    let (client, reader, _) = handshake(stream, root, server, Some(process))?;
    Ok((client, reader))
}

/// Connects to a Godot editor's language server on `port`, if one is
/// listening and has the project at `root` open.
fn attach(root: &Path, port: u16) -> Option<(Client, Reader)> {
    let stream = TcpStream::connect_timeout(&local(port), Duration::from_millis(300)).ok()?;
    let (client, reader, workspace) = handshake(stream, root, Server::Attached { port }, None).ok()?;
    // The editor reports its own project; only reuse it for ours.
    workspace.is_none_or(|w| same_path(&w, root)).then_some((client, reader))
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn spawn_server(godot: &Path, root: &Path) -> Result<(TcpStream, Server, ServerProcess), String> {
    let mut free = SPAWN_PORTS.clone().filter(|&port| TcpListener::bind(local(port)).is_ok());
    let port = free.next().ok_or("No free port for the language server")?;
    let dap_port = free.next().ok_or("No free port for the debug adapter")?;

    let mut command = Command::new(godot);
    command
        .args(["--headless", "--editor", "--path"])
        .arg(root)
        .args(["--lsp-port", &port.to_string(), "--dap-port", &dap_port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", godot.display()))?;
    #[cfg(windows)]
    let job = job::Job::assign(&child);

    let deadline = Instant::now() + SPAWN_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Godot exited during startup ({status})"));
        }
        if let Ok(stream) = TcpStream::connect_timeout(&local(port), Duration::from_millis(500)) {
            let process = ServerProcess {
                child: Mutex::new(child),
                #[cfg(windows)]
                _job: job,
            };
            let server = Server::Spawned { port, dap_port, godot: godot.to_path_buf() };
            return Ok((stream, server, process));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("Timed out waiting for Godot's language server".to_string());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Runs `initialize`/`initialized`. Also returns the workspace path Godot
/// announces via `gdscript_client/changeWorkspace`, if any.
fn handshake(
    stream: TcpStream,
    root: &Path,
    server: Server,
    process: Option<ServerProcess>,
) -> Result<(Client, Reader, Option<PathBuf>), String> {
    let io_err = |e: io::Error| format!("Language server connection failed: {e}");
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().map_err(io_err)?);
    let writer = Arc::new(Mutex::new(stream.try_clone().map_err(io_err)?));
    let client = Client {
        inner: Arc::new(Inner {
            writer: Arc::clone(&writer),
            next_id: AtomicU64::new(1),
            server,
            process,
        }),
    };

    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).map_err(io_err)?;
    let id = client
        .request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": path_to_uri(root),
                "rootPath": root.to_string_lossy(),
                "clientInfo": { "name": "Godot IDE", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": {},
                    "definition": { "linkSupport": false },
                    "references": {},
                    "rename": { "prepareSupport": true },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "completion": { "completionItem": { "documentationFormat": ["markdown", "plaintext"] } },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "signatureHelp": { "signatureInformation": { "documentationFormat": ["markdown"] } },
                }},
            }),
        )
        .map_err(io_err)?;

    let mut workspace = None;
    loop {
        let message = protocol::read_message(&mut reader)
            .map_err(io_err)?
            .ok_or("Language server closed the connection")?;
        if message.get("method").and_then(Value::as_str) == Some("gdscript_client/changeWorkspace") {
            workspace = message["params"]["path"].as_str().map(PathBuf::from);
            continue;
        }
        if message.get("method").is_none() && message.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = message.get("error") {
                return Err(format!("Language server rejected initialize: {error}"));
            }
            break;
        }
    }
    stream.set_read_timeout(None).map_err(io_err)?;
    client.notify("initialized", json!({})).map_err(io_err)?;

    Ok((client, Reader { reader, writer }, workspace))
}
