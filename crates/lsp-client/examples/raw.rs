//! Prints raw server answers for completion/hover/signature help:
//! `cargo run -p lsp-client --example raw -- <project> <file.gd>`

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use lsp_client::{godot, uri, Event, StartOptions};
use serde_json::json;

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let root = args.next().expect("project path");
    let file = args.next().expect("script path");

    let installs = godot::discover();
    let options = StartOptions {
        godot: godot::pick(&installs, None).map(|i| i.path.clone()),
        ..StartOptions::default()
    };
    let (client, reader) = lsp_client::start(&root, &options).expect("start");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || reader.run(move |e| drop(tx.send(e))));

    // Appended test code; positions below are zero-based.
    let base = std::fs::read_to_string(&file).unwrap();
    let text = format!("{base}\nfunc probe():\n\tvelocity.\n\tprint(speed, \n");
    let line_of = |needle: &str| text.lines().position(|l| l == needle).unwrap() as u32;
    let base_lines = line_of("\tvelocity.") - 1;
    client.did_open(&file, 1, &text).unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let doc = json!({ "uri": uri::path_to_uri(&file) });
    let at = |line: u32, character: u32| json!({ "textDocument": doc, "position": { "line": line, "character": character } });
    let requests = [
        ("completion", "textDocument/completion", at(base_lines + 1, 10)),
        ("hover", "textDocument/hover", at(15, 6)),
        ("signature", "textDocument/signatureHelp", at(base_lines + 2, 14)),
    ];
    let mut pending = Vec::new();
    for (name, method, params) in requests {
        pending.push((client.request(method, params).unwrap(), name));
    }

    while !pending.is_empty() {
        let Ok(event) = rx.recv_timeout(Duration::from_secs(10)) else { break };
        if let Event::Response { id, result } = event {
            if let Some(i) = pending.iter().position(|(p, _)| *p == id) {
                let (_, name) = pending.remove(i);
                let text = match result {
                    Ok(v) => serde_json::to_string(&v).unwrap(),
                    Err(e) => format!("ERROR {e}"),
                };
                let cut: String = text.chars().take(1500).collect();
                println!("=== {name} ({} chars) ===\n{cut}\n", text.len());
                if name == "completion" {
                    // Resolve the first item to see where docs come from.
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let items = v.get("items").cloned().unwrap_or(v.clone());
                        if let Some(first) = items.as_array().and_then(|a| a.iter().find(|i| i["label"].as_str().is_some_and(|l| l.starts_with("length")))) {
                            let id = client.request("completionItem/resolve", first.clone()).unwrap();
                            pending.push((id, "resolve(length)"));
                        }
                    }
                }
            }
        }
    }
}
