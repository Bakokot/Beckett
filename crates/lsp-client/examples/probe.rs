//! Smoke test against a real Godot: `cargo run -p lsp-client --example probe -- <project> <file.gd>`

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lsp_client::{godot, Event, Position, StartOptions};

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let root = args.next().expect("project path");
    let file = args.next().expect("script path");

    let installs = godot::discover();
    for install in &installs {
        println!("found Godot {:?} at {}", install.version_string(), install.path.display());
    }
    let options = StartOptions {
        godot: godot::pick(&installs, None).map(|i| i.path.clone()),
        ..StartOptions::default()
    };

    let started = Instant::now();
    let (client, reader) = lsp_client::start(&root, &options).expect("start");
    println!("connected to {:?} in {:?}", client.server(), started.elapsed());

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || reader.run(move |e| drop(tx.send(e))));

    let text = std::fs::read_to_string(&file).unwrap() + "\nfunc broken(:\n";
    client.did_open(&file, 1, &text).unwrap();
    let id = client.definition(&file, Position { line: 6, character: 8 }).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        match event {
            Event::Diagnostics { path, diagnostics } => {
                println!("diagnostics for {}:", path.display());
                for d in diagnostics {
                    println!("  {:?} {}:{} {}", d.severity, d.range.start.line + 1, d.range.start.character + 1, d.message);
                }
            }
            Event::Response { id: rid, result } if rid == id => {
                println!("definition: {:?}", result.map(|v| lsp_client::locations(&v)));
            }
            other => println!("event: {other:?}"),
        }
    }
}
