//! Spawns a shell, runs one command and prints the screen:
//! `cargo run -p terminal --example smoke`

use std::sync::mpsc;
use std::time::{Duration, Instant};

use terminal::{Event, Options, Palette, Rgb, Terminal};

fn main() {
    let (tx, rx) = mpsc::channel();
    let shell = if cfg!(windows) {
        Some(("powershell.exe".to_string(), vec!["-NoLogo".to_string(), "-NoProfile".to_string()]))
    } else {
        None
    };
    let options = Options { shell, ..Options::default() };
    let term = Terminal::spawn(options, 80, 12, move |e| drop(tx.send(e))).expect("spawn");

    let wait = |ms| {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            if let Event::Exit = event {
                println!("(shell exited)");
            }
        }
    };
    wait(2500);
    term.write(b"echo \"hello from $($PSVersionTable.PSVersion)\"\r".to_vec());
    wait(1500);

    let palette = Palette {
        ansi: [Rgb::new(0, 0, 0); 16],
        foreground: Rgb::new(255, 255, 255),
        background: Rgb::new(0, 0, 0),
        cursor: Rgb::new(255, 255, 255),
    };
    let snapshot = term.snapshot(&palette);
    for row in &snapshot.rows {
        let line: String = row.iter().map(|(_, cell)| cell.c).collect();
        println!("|{}", line.trim_end());
    }
    println!("cursor: {:?}", snapshot.cursor);
}
