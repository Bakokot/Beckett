//! Persistent undo: each file's undo and redo history is kept in
//! `<config>/undo/` when its tab closes and with the session, and given back
//! when the file opens again holding the same text.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use editor_core::buffer::History;
use editor_core::Buffer;
use serde::{Deserialize, Serialize};

use crate::session::content_hash;

/// Steps kept of each stack (the most recent ones).
const STEPS: usize = 500;
/// Histories bigger than this aren't kept.
const MAX_BYTES: usize = 8 << 20;
/// Histories of files left alone this long are removed.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Serialize, Deserialize)]
struct Saved {
    path: PathBuf,
    /// Hash of the text the history ends at.
    hash: u64,
    history: History,
}

fn dir() -> Option<PathBuf> {
    Some(theme::config_dir()?.join("undo"))
}

fn file(dir: &Path, path: &Path) -> PathBuf {
    let key = lsp_client::uri::path_key(path);
    dir.join(format!("{:016x}.json", content_hash(&key)))
}

/// Keeps the buffer's history for `path`; an empty one removes the kept one.
pub fn save(path: &Path, buffer: &Buffer) -> Result<(), String> {
    save_in(&dir().ok_or("No user config folder")?, path, buffer)
}

/// Gives the buffer the history kept for `path`, if it ends at the
/// buffer's text. Returns whether it did.
pub fn load(path: &Path, buffer: &mut Buffer) -> bool {
    dir().is_some_and(|dir| load_in(&dir, path, buffer))
}

fn save_in(dir: &Path, path: &Path, buffer: &Buffer) -> Result<(), String> {
    let file = file(dir, path);
    let history = buffer.history(STEPS);
    if history.is_empty() {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let saved = Saved { path: path.to_path_buf(), hash: content_hash(&buffer.text()), history };
    let text = serde_json::to_string(&saved).map_err(|e| e.to_string())?;
    if text.len() > MAX_BYTES {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, text).map_err(|e| e.to_string())
}

fn load_in(dir: &Path, path: &Path, buffer: &mut Buffer) -> bool {
    let Ok(text) = std::fs::read_to_string(file(dir, path)) else {
        return false;
    };
    let Ok(saved) = serde_json::from_str::<Saved>(&text) else {
        return false;
    };
    let same = lsp_client::uri::same_path(&saved.path, path) && saved.hash == content_hash(&buffer.text());
    same && buffer.set_history(saved.history)
}

/// Removes the histories of files not edited for a month (in the background).
pub fn prune() {
    std::thread::spawn(|| {
        let Some(entries) = dir().and_then(|dir| std::fs::read_dir(dir).ok()) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let modified = entry.metadata().and_then(|m| m.modified());
            if modified.is_ok_and(|time| now.duration_since(time).unwrap_or_default() > MAX_AGE) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Command, Language};

    #[test]
    fn histories_come_back_only_for_the_same_text() {
        let dir = std::env::temp_dir().join(format!("beckett-undo-test-{}", std::process::id()));
        let path = Path::new("C:/game/player.gd");
        let mut buffer = Buffer::new("", Language::GDScript);
        buffer.apply(Command::Insert("x".into()));
        save_in(&dir, path, &buffer).unwrap();

        let mut reopened = Buffer::new("x", Language::GDScript);
        assert!(load_in(&dir, path, &mut reopened));
        reopened.apply(Command::Undo);
        assert_eq!(reopened.text(), "");

        // Changed since (or another file): nothing comes back.
        assert!(!load_in(&dir, path, &mut Buffer::new("y", Language::GDScript)));
        assert!(!load_in(&dir, Path::new("C:/game/enemy.gd"), &mut Buffer::new("x", Language::GDScript)));

        // Nothing left to undo or redo removes the kept history.
        save_in(&dir, path, &Buffer::new("x", Language::GDScript)).unwrap();
        assert!(!file(&dir, path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
