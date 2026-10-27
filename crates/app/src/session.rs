//! Per-project sessions: the open tabs (cursor, folds, and the contents of
//! unsaved ones — "hot exit"), kept in `<config>/sessions/<project>.json`.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<TabState>,
    pub active: Option<usize>,
    /// Share of the width the left side of the split view takes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_ratio: Option<f32>,
    /// The Godot executable chosen for the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub godot: Option<PathBuf>,
    /// Breakpoint lines (zero-based) by file.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub breakpoints: std::collections::BTreeMap<PathBuf, Vec<usize>>,
    /// Watch expressions of the debugger.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub watches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    pub path: PathBuf,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
    /// Side of the split view (0 left, 1 right).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub group: usize,
    /// Header lines of folded blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<usize>,
    /// Contents of a tab with unsaved edits, as it would be saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsaved: Option<String>,
    /// Hash of the file on disk when `unsaved` was taken: if the file
    /// changed since, the restored tab is flagged "changed on disk".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_hash: Option<u64>,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// A stable hash of file contents (the same across runs).
pub fn content_hash(text: &str) -> u64 {
    // FNV-1a: tiny, deterministic, good enough to notice a change.
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

fn file(root: &Path) -> Option<PathBuf> {
    let key = lsp_client::uri::path_key(root);
    let mut hasher = FnvHasher::default();
    key.hash(&mut hasher);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).take(40).collect();
    Some(theme::config_dir()?.join("sessions").join(format!("{name}-{:016x}.json", hasher.finish())))
}

pub fn load(root: &Path) -> Session {
    file(root)
        .and_then(|path| std::fs::read_to_string(path).ok())
        // Hand-edited files may start with a byte order mark.
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(root: &Path, session: &Session) -> Result<(), String> {
    let path = file(root).ok_or("No user config folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// FNV-1a as a `Hasher`, for a file name that stays the same across runs
/// (std's default hasher makes no such promise).
struct FnvHasher(u64);

impl Default for FnvHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_round_trip_and_tolerate_missing_fields() {
        let session = Session {
            tabs: vec![
                TabState { path: "C:/game/a.gd".into(), line: 3, column: 1, group: 1, folds: vec![5], unsaved: None, disk_hash: None },
                TabState {
                    path: "C:/game/b.gd".into(),
                    line: 0,
                    column: 0,
                    group: 0,
                    folds: Vec::new(),
                    unsaved: Some("extends Node\n".into()),
                    disk_hash: Some(content_hash("extends Node2D\n")),
                },
            ],
            active: Some(1),
            split_ratio: Some(0.25),
            godot: Some("C:/Godot/Godot_v4.5.1-stable_win64.exe".into()),
            breakpoints: [("C:/game/a.gd".into(), vec![2, 9])].into_iter().collect(),
            watches: vec!["velocity.length()".into()],
        };
        let text = serde_json::to_string(&session).unwrap();
        assert!(!text.contains("\"folds\":[]"), "{text}");
        assert_eq!(serde_json::from_str::<Session>(&text).unwrap(), session);
        let old: Session = serde_json::from_str(r#"{"tabs": [{"path": "C:/game/a.gd"}]}"#).unwrap();
        assert_eq!((old.tabs[0].line, old.active, old.split_ratio, old.godot), (0, None, None, None));
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(content_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(content_hash("a"), content_hash("b"));
        let a = file(Path::new("C:/game")).unwrap();
        assert_eq!(a, file(Path::new("C:/game")).unwrap());
        assert!(a.to_string_lossy().contains("game-"));
    }
}
