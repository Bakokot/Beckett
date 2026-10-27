//! Talks to Godot's debug adapter and prints everything it says:
//! `cargo run -p lsp-client --example dap_probe -- <godot.exe> <project> <script.gd> <line>`
//! (a scratch project: Godot imports it and runs its main scene).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn send(stream: &mut TcpStream, seq: &mut i64, command: &str, arguments: Value) -> i64 {
    *seq += 1;
    let message = json!({ "seq": *seq, "type": "request", "command": command, "arguments": arguments });
    let body = serde_json::to_vec(&message).unwrap();
    write!(stream, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    stream.write_all(&body).unwrap();
    println!(">>> {}", serde_json::to_string(&message).unwrap());
    *seq
}

fn read(reader: &mut BufReader<TcpStream>) -> Option<Value> {
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse().ok()?;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let godot = args.next().expect("godot executable");
    let root = args.next().expect("project folder");
    let script = args.next().expect("script");
    let line: i64 = args.next().and_then(|l| l.to_str()?.parse().ok()).expect("line");
    let port = 6111;

    let mut child = Command::new(&godot)
        .args(["--headless", "--editor", "--path"])
        .arg(&root)
        .args(["--dap-port", &port.to_string(), "--lsp-port", "6112"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn godot");

    let start = Instant::now();
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        if start.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            panic!("no debug adapter on {port}");
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    println!("connected after {:?}", start.elapsed());
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    reader.get_ref().set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut seq = 0;

    send(&mut writer, &mut seq, "initialize", json!({ "clientID": "beckett", "adapterID": "godot", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path" }));
    send(&mut writer, &mut seq, "launch", json!({ "project": root, "noDebug": false }));
    let mut stops = 0;
    while let Some(message) = read(&mut reader) {
        let text = serde_json::to_string(&message).unwrap();
        println!("<<< [{:.1}s] {}", start.elapsed().as_secs_f32(), text.chars().take(700).collect::<String>());
        let kind = message["type"].as_str().unwrap_or_default();
        let name = message["event"].as_str().or(message["command"].as_str()).unwrap_or_default();
        match (kind, name) {
            ("event", "initialized") => {
                send(&mut writer, &mut seq, "setBreakpoints", json!({
                    "source": { "path": script, "name": script.file_name().unwrap().to_string_lossy() },
                    "breakpoints": [{ "line": line }],
                    "lines": [line],
                }));
                send(&mut writer, &mut seq, "configurationDone", json!({}));
            }
            ("event", "stopped") => {
                stops += 1;
                send(&mut writer, &mut seq, "threads", json!({}));
                send(&mut writer, &mut seq, "stackTrace", json!({ "threadId": 1 }));
            }
            ("response", "stackTrace") => {
                if let Some(frame) = message["body"]["stackFrames"].get(0) {
                    send(&mut writer, &mut seq, "scopes", json!({ "frameId": frame["id"] }));
                }
            }
            ("response", "scopes") => {
                for scope in message["body"]["scopes"].as_array().into_iter().flatten() {
                    send(&mut writer, &mut seq, "variables", json!({ "variablesReference": scope["variablesReference"] }));
                }
            }
            ("response", "variables") => {
                if stops < 3 && message["request_seq"].as_i64() == Some(seq) {
                    std::thread::sleep(Duration::from_millis(300));
                    send(&mut writer, &mut seq, "continue", json!({ "threadId": 1 }));
                } else if stops >= 3 && message["request_seq"].as_i64() == Some(seq) {
                    send(&mut writer, &mut seq, "disconnect", json!({}));
                }
            }
            ("event", "terminated") | ("event", "exited") => {}
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(120) {
            break;
        }
    }
    let _ = child.kill();
    println!("done");
}
