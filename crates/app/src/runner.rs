//! Running the game: spawns Godot on the project (or one scene), streams
//! its output line by line and finds `res://file:line` references in it.

use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::{self, UnboundedSender};
use iced::futures::{SinkExt, StreamExt};
use iced::Subscription;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Error,
    Warning,
    /// Messages from the IDE itself ("Running…", exit codes).
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputLine {
    pub text: String,
    pub kind: LineKind,
    /// `res://` path and 1-based line, when the text points at one.
    pub link: Option<(String, u32)>,
}

impl OutputLine {
    pub fn info(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: LineKind::Info, link: None }
    }
}

#[derive(Debug, Clone)]
pub enum RunEvent {
    Started(Arc<RunHandle>),
    Line(OutputLine),
    Exited(Option<i32>),
    Failed(String),
}

/// A running game; dropping the last handle kills it.
pub struct RunHandle {
    child: Mutex<Child>,
    #[cfg(windows)]
    _job: Option<lsp_client::job::Job>,
}

impl fmt::Debug for RunHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunHandle").finish_non_exhaustive()
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// One run; a new `id` starts a new process.
#[derive(Debug, Clone, Hash)]
pub struct RunRequest {
    pub id: u64,
    pub godot: PathBuf,
    pub root: PathBuf,
    /// `res://` scene to run instead of the main scene.
    pub scene: Option<String>,
}

impl RunRequest {
    pub fn command_line(&self) -> String {
        let exe = self.godot.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut line = format!("{exe} --path \"{}\"", self.root.display());
        if let Some(scene) = &self.scene {
            line.push(' ');
            line.push_str(scene);
        }
        line
    }
}

pub fn run(request: RunRequest) -> Subscription<RunEvent> {
    Subscription::run_with(request, |request| {
        let request = request.clone();
        iced::stream::channel(256, async move |mut output| {
            let (tx, mut rx) = mpsc::unbounded();
            std::thread::spawn(move || run_blocking(&request, &tx));
            while let Some(event) = rx.next().await {
                if output.send(event).await.is_err() {
                    return;
                }
            }
            std::future::pending::<()>().await;
        })
    })
}

fn run_blocking(request: &RunRequest, tx: &UnboundedSender<RunEvent>) {
    let mut command = Command::new(&request.godot);
    command.arg("--path").arg(&request.root);
    if let Some(scene) = &request.scene {
        command.arg(scene);
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = tx.unbounded_send(RunEvent::Failed(format!("Cannot start Godot: {error}")));
            return;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let handle = Arc::new(RunHandle {
        #[cfg(windows)]
        _job: lsp_client::job::Job::assign(&child),
        child: Mutex::new(child),
    });
    // Only the app holds the handle, so dropping it there stops the game.
    let weak = Arc::downgrade(&handle);
    if tx.unbounded_send(RunEvent::Started(handle)).is_err() {
        return;
    }

    let readers: Vec<_> = [stdout.map(|s| Box::new(s) as Box<dyn Read + Send>), stderr.map(|s| Box::new(s) as _)]
        .into_iter()
        .flatten()
        .map(|stream| {
            let tx = tx.clone();
            std::thread::spawn(move || forward_lines(stream, &tx))
        })
        .collect();
    for reader in readers {
        let _ = reader.join();
    }
    let _ = tx.unbounded_send(RunEvent::Exited(exit_code(&weak)));
}

/// The exit status once the pipes have closed; `None` if the game was stopped.
fn exit_code(handle: &Weak<RunHandle>) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let handle = handle.upgrade()?;
        let status = handle.child.lock().unwrap_or_else(PoisonError::into_inner).try_wait();
        match status {
            Ok(Some(status)) => return status.code(),
            Ok(None) if Instant::now() < deadline => {}
            _ => return None,
        }
        drop(handle);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn forward_lines(stream: Box<dyn Read + Send>, tx: &UnboundedSender<RunEvent>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut last = LineKind::Normal;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let raw = String::from_utf8_lossy(&buf);
                let text = strip_ansi(raw.trim_end_matches(['\n', '\r']));
                let line = classify(text, last);
                last = line.kind;
                if tx.unbounded_send(RunEvent::Line(line)).is_err() {
                    return;
                }
            }
        }
    }
}

/// Classifies a line of Godot output. Indented lines (`at: …`, backtraces)
/// continue the error or warning above them.
pub fn classify(text: String, previous: LineKind) -> OutputLine {
    let trimmed = text.trim_start();
    let kind = if ["ERROR:", "SCRIPT ERROR:", "USER ERROR:", "Parse Error:"]
        .iter()
        .any(|p| trimmed.starts_with(p))
    {
        LineKind::Error
    } else if ["WARNING:", "USER WARNING:", "SCRIPT WARNING:"].iter().any(|p| trimmed.starts_with(p)) {
        LineKind::Warning
    } else if text.starts_with([' ', '\t']) && matches!(previous, LineKind::Error | LineKind::Warning) {
        previous
    } else {
        LineKind::Normal
    };
    let link = find_link(&text);
    OutputLine { text, kind, link }
}

/// Finds the first `res://path:line` in a line of output.
pub fn find_link(text: &str) -> Option<(String, u32)> {
    let start = text.find("res://")?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, ')' | '(' | '"' | '\'' | ','))
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    let (path, tail) = candidate["res://".len()..]
        .find(':')
        .map(|i| candidate.split_at("res://".len() + i))?;
    let digits: String = tail[1..].chars().take_while(char::is_ascii_digit).collect();
    let line = digits.parse().ok().filter(|&n| n > 0)?;
    Some((path.to_string(), line))
}

/// Removes ANSI escape sequences (Godot colors its console output).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte in '@'..='~'.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: until BEL or ST (ESC \).
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_colors() {
        assert_eq!(strip_ansi("\x1b[91mERROR:\x1b[0m boom"), "ERROR: boom");
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn finds_res_links() {
        assert_eq!(
            find_link("   at: _ready (res://scenes/player.gd:12)"),
            Some(("res://scenes/player.gd".into(), 12))
        );
        assert_eq!(find_link("res://a.gd:3:7 - Parse Error"), Some(("res://a.gd".into(), 3)));
        assert_eq!(find_link("Loading res://icon.svg"), None);
        assert_eq!(find_link("no link here"), None);
    }

    #[test]
    fn classifies_errors_and_continuations() {
        let error = classify("SCRIPT ERROR: Invalid call.".into(), LineKind::Normal);
        assert_eq!(error.kind, LineKind::Error);
        let at = classify("          at: _ready (res://player.gd:5)".into(), error.kind);
        assert_eq!(at.kind, LineKind::Error);
        assert_eq!(at.link, Some(("res://player.gd".into(), 5)));
        assert_eq!(classify("WARNING: careful".into(), LineKind::Normal).kind, LineKind::Warning);
        assert_eq!(classify("Hello world".into(), LineKind::Error).kind, LineKind::Normal);
    }
}
