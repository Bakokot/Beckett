//! Debug Adapter Protocol client for Godot's debug adapter (the editor's,
//! or the one of the headless Godot started for the language server).
//!
//! Same framing as LSP. Requests carry a `seq`; answers come back as
//! responses with its `request_seq`, mixed with events (`stopped`,
//! `output`, `terminated`…), all handed to the reader's callback.

use std::io::{self, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::{read_message, write_message};

#[derive(Debug, Clone)]
pub enum DapMessage {
    Response { request_seq: i64, command: String, success: bool, body: Value, message: Option<String> },
    Event { event: String, body: Value },
}

/// Sends requests; clones share the connection.
#[derive(Clone)]
pub struct DapClient {
    writer: Arc<Mutex<TcpStream>>,
    seq: Arc<AtomicI64>,
}

impl std::fmt::Debug for DapClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DapClient")
    }
}

pub struct DapReader {
    reader: BufReader<TcpStream>,
}

/// Connects to a debug adapter on this machine.
pub fn connect(port: u16) -> Result<(DapClient, DapReader), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|e| format!("No Godot debug adapter on port {port}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let client = DapClient { writer: Arc::new(Mutex::new(stream)), seq: Arc::new(AtomicI64::new(0)) };
    Ok((client, DapReader { reader }))
}

impl DapReader {
    /// Reads until the connection closes, handing each message to `handle`.
    pub fn run(mut self, mut handle: impl FnMut(DapMessage)) {
        while let Ok(Some(message)) = read_message(&mut self.reader) {
            if let Some(message) = parse(message) {
                handle(message);
            }
        }
    }
}

fn parse(message: Value) -> Option<DapMessage> {
    let text = |key: &str| message[key].as_str().map(str::to_string);
    match message["type"].as_str()? {
        "response" => Some(DapMessage::Response {
            request_seq: message["request_seq"].as_i64()?,
            command: text("command").unwrap_or_default(),
            success: message["success"].as_bool().unwrap_or(false),
            message: text("message"),
            body: message["body"].clone(),
        }),
        "event" => Some(DapMessage::Event { event: text("event")?, body: message["body"].clone() }),
        _ => None,
    }
}

impl DapClient {
    /// Sends a request and returns its `seq`, which its response carries.
    pub fn request(&self, command: &str, arguments: Value) -> io::Result<i64> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let message = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        let mut writer = self.writer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        write_message(&mut *writer, &message)?;
        Ok(seq)
    }

    pub fn initialize(&self) -> io::Result<i64> {
        self.request(
            "initialize",
            json!({ "clientID": "beckett", "clientName": "Beckett", "adapterID": "godot",
                    "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path" }),
        )
    }

    /// Runs the project (its main scene, or `scene`, a `res://` path).
    pub fn launch(&self, project: &Path, scene: Option<&str>) -> io::Result<i64> {
        self.request("launch", json!({ "project": project, "scene": scene.unwrap_or("main"), "noDebug": false }))
    }

    /// Replaces the breakpoints of one file (one-based lines).
    pub fn set_breakpoints(&self, path: &Path, lines: &[usize]) -> io::Result<i64> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let breakpoints: Vec<Value> = lines.iter().map(|line| json!({ "line": line })).collect();
        self.request(
            "setBreakpoints",
            json!({ "source": { "path": path, "name": name }, "breakpoints": breakpoints, "lines": lines }),
        )
    }

    pub fn configuration_done(&self) -> io::Result<i64> {
        self.request("configurationDone", json!({}))
    }

    pub fn stack_trace(&self) -> io::Result<i64> {
        self.request("stackTrace", json!({ "threadId": 1 }))
    }

    pub fn scopes(&self, frame: i64) -> io::Result<i64> {
        self.request("scopes", json!({ "frameId": frame }))
    }

    pub fn variables(&self, reference: i64) -> io::Result<i64> {
        self.request("variables", json!({ "variablesReference": reference }))
    }

    /// Evaluates a GDScript expression in a paused frame (`context`:
    /// "hover" or "watch"). Godot answers `<null>` for unknown names.
    pub fn evaluate(&self, expression: &str, frame: i64, context: &str) -> io::Result<i64> {
        self.request("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }))
    }

    /// `continue`, `next` (step over), `stepIn`, `stepOut` or `pause`.
    pub fn step(&self, command: &str) -> io::Result<i64> {
        self.request(command, json!({ "threadId": 1 }))
    }

    /// Ends the game and the session.
    pub fn disconnect(&self) -> io::Result<i64> {
        self.request("disconnect", json!({ "terminateDebuggee": true }))
    }
}

/// One frame of the call stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub id: i64,
    /// The function.
    pub name: String,
    pub path: Option<PathBuf>,
    /// One-based.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    pub reference: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    /// Non-zero when it has children (an object, array or dictionary).
    pub reference: i64,
}

/// Godot answers with `C:/…` paths: rebuilt with native separators.
fn native_path(path: &str) -> PathBuf {
    Path::new(path).components().collect()
}

pub fn stack_frames(body: &Value) -> Vec<StackFrame> {
    body["stackFrames"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|frame| StackFrame {
            id: frame["id"].as_i64().unwrap_or_default(),
            name: frame["name"].as_str().unwrap_or_default().to_string(),
            path: frame["source"]["path"].as_str().map(native_path),
            line: frame["line"].as_u64().unwrap_or(1) as usize,
        })
        .collect()
}

pub fn scopes(body: &Value) -> Vec<Scope> {
    body["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|scope| Scope {
            name: scope["name"].as_str().unwrap_or_default().to_string(),
            reference: scope["variablesReference"].as_i64().unwrap_or_default(),
        })
        .collect()
}

/// The variables of a scope or value; Godot's own bookkeeping (`@…`, e.g.
/// a loop's position) left out.
pub fn variables(body: &Value) -> Vec<Variable> {
    body["variables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variable| {
            let name = variable["name"].as_str()?;
            (!name.starts_with('@')).then(|| Variable {
                name: name.to_string(),
                value: variable["value"].as_str().unwrap_or_default().to_string(),
                reference: variable["variablesReference"].as_i64().unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes as Godot 4.7.1 sends them (from a probe run).
    #[test]
    fn reads_godot_answers() {
        let frames = stack_frames(&json!({ "stackFrames": [
            { "column": 0, "id": 0, "line": 9, "name": "_ready",
              "source": { "name": "main.gd", "path": "C:/game/main.gd" } }
        ] }));
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].name.as_str(), frames[0].line), ("_ready", 9));
        assert_eq!(frames[0].path.as_deref(), Some(native_path("C:/game/main.gd").as_path()));

        let scopes = scopes(&json!({ "scopes": [
            { "expensive": false, "name": "Locals", "presentationHint": "locals", "variablesReference": 1 },
            { "expensive": false, "name": "Members", "presentationHint": "members", "variablesReference": 2 }
        ] }));
        assert_eq!(scopes[1], Scope { name: "Members".into(), reference: 2 });

        let variables = variables(&json!({ "variables": [
            { "name": "message", "value": "hello", "variablesReference": 0 },
            { "name": "@counter_pos", "value": "0", "variablesReference": 0 },
            { "name": "self", "value": "<EncodedObjectAsID#-9223370534052357147>", "variablesReference": 4 }
        ] }));
        let names: Vec<&str> = variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["message", "self"]);
        assert_eq!(variables[1].reference, 4);

        let event = parse(json!({ "event": "stopped", "seq": 9, "type": "event",
            "body": { "description": "Breakpoint", "reason": "breakpoint", "threadId": 1 } }));
        assert!(matches!(event, Some(DapMessage::Event { ref event, .. }) if event == "stopped"));
        let response = parse(json!({ "command": "variables", "message": "unknown", "request_seq": 8, "seq": 14,
            "success": false, "type": "response", "body": {} }));
        assert!(matches!(response, Some(DapMessage::Response { request_seq: 8, success: false, .. })));
    }
}
