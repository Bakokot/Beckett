//! Runs a scratch project under the debugger through `lsp_client::dap`:
//! `cargo run -p lsp-client --example dap_client -- <godot.exe> <project> <script.gd> <line> [res://scene]`

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lsp_client::dap::{self, DapMessage};

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let godot = args.next().expect("godot executable");
    let root = args.next().expect("project folder");
    let script = args.next().expect("script");
    let line: usize = args.next().and_then(|l| l.to_str()?.parse().ok()).expect("line");
    let scene = args.next().map(|s| s.to_string_lossy().into_owned());
    let port = 6121;

    let mut child = Command::new(&godot)
        .args(["--headless", "--editor", "--path"])
        .arg(&root)
        .args(["--dap-port", &port.to_string(), "--lsp-port", "6122"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn godot");
    let start = Instant::now();
    let (client, reader) = loop {
        match dap::connect(port) {
            Ok(connection) => break connection,
            Err(_) if start.elapsed() < Duration::from_secs(90) => std::thread::sleep(Duration::from_millis(300)),
            Err(error) => {
                let _ = child.kill();
                panic!("{error}");
            }
        }
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || reader.run(move |m| drop(tx.send(m))));

    client.initialize().unwrap();
    client.launch(&root, scene.as_deref()).unwrap();
    let mut stops = 0;
    while let Ok(message) = rx.recv_timeout(Duration::from_secs(20)) {
        match message {
            DapMessage::Event { event, body } => {
                println!("event {event}: {}", body.to_string().chars().take(160).collect::<String>());
                match event.as_str() {
                    "initialized" => {
                        client.set_breakpoints(&script, &[line]).unwrap();
                        client.configuration_done().unwrap();
                    }
                    "stopped" => {
                        stops += 1;
                        client.stack_trace().unwrap();
                    }
                    "terminated" => break,
                    _ => {}
                }
            }
            DapMessage::Response { command, success, body, message, .. } => {
                println!("response {command} ok={success} {}", message.unwrap_or_default());
                if command == "stackTrace" {
                    let frames = dap::stack_frames(&body);
                    println!("  frames: {frames:?}");
                    if let Some(frame) = frames.first() {
                        client.scopes(frame.id).unwrap();
                    }
                }
                if command == "scopes" {
                    if let Some(scope) = dap::scopes(&body).first() {
                        client.variables(scope.reference).unwrap();
                    }
                }
                if command == "variables" && !success {
                    // Godot fetches the frame's variables from the game first.
                    std::thread::sleep(Duration::from_millis(100));
                    println!("  (retrying)");
                    client.variables(1).unwrap();
                    continue;
                }
                if command == "variables" {
                    println!("  variables: {:?}", dap::variables(&body));
                    if stops >= 2 {
                        client.disconnect().unwrap();
                    } else {
                        client.step("next").unwrap();
                    }
                }
            }
        }
    }
    let _ = child.kill();
    println!("done");
}
