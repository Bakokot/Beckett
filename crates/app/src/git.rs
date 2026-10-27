//! Git through the `git` command: the repository of the project, file
//! statuses for the explorer, and a file's staged text for the gutter's
//! change markers. Without git (or outside a repository) nothing is shown.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use editor_core::diff::{self, Hunk};

/// How a file differs from the last commit, as the explorer shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    /// New in the index, or not tracked yet.
    Added,
    Conflicted,
}

/// The statuses of a repository's changed files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// By normalized path: the path as git gave it, and its status.
    files: HashMap<PathBuf, (PathBuf, FileStatus)>,
    /// Folders holding at least one changed file.
    folders: HashSet<PathBuf>,
}

impl Status {
    pub fn file(&self, path: &Path) -> Option<FileStatus> {
        self.files.get(&normalize(path)).map(|(_, status)| *status)
    }

    /// Every changed file, sorted by path.
    pub fn changed(&self) -> Vec<(PathBuf, FileStatus)> {
        let mut files: Vec<_> = self.files.values().cloned().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    pub fn folder_changed(&self, path: &Path) -> bool {
        self.folders.contains(&normalize(path))
    }
}

/// Paths compared case-insensitively with `/` separators on Windows.
fn normalize(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        PathBuf::from(text.replace('\\', "/").to_lowercase())
    } else {
        PathBuf::from(text.into_owned())
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// The top folder of the repository holding `dir`.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    let root = String::from_utf8(out).ok()?;
    // git prints `C:/…`: rebuilt with the native separators.
    Some(Path::new(root.trim()).components().collect())
}

/// The text of `path` as staged (the index), which the working file is
/// compared with; `None` for files git doesn't track.
pub fn staged_text(repo: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(repo).ok().or_else(|| strip_prefix_ignoring_case(path, repo))?;
    let spec = format!(":{}", relative.to_string_lossy().replace('\\', "/"));
    let bytes = git(repo, &["show", &spec])?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text.trim_start_matches('\u{feff}').replace("\r\n", "\n"))
}

/// Windows paths may differ from git's in case (`c:` / `C:`).
fn strip_prefix_ignoring_case<'a>(path: &'a Path, prefix: &Path) -> Option<&'a Path> {
    if !cfg!(windows) {
        return None;
    }
    let prefix_len = prefix.components().count();
    let mut components = path.components();
    let head: PathBuf = components.by_ref().take(prefix_len).collect();
    (normalize(&head) == normalize(prefix)).then(|| components.as_path())
}

/// A changed file as the CHANGES panel shows it.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub status: FileStatus,
    /// The staged text (`None`: new file).
    pub base: Option<String>,
    /// The text on disk and its hunks; `None` for files open in a tab,
    /// whose current text is the tab's.
    pub current: Option<(String, Vec<Hunk>)>,
}

/// What a refresh reads: the status, the staged text of the open files,
/// and the changed files.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub status: Status,
    pub bases: Vec<(PathBuf, Option<String>)>,
    pub files: Vec<ChangedFile>,
}

/// Files past this size aren't diffed in the CHANGES panel.
const MAX_DIFF_BYTES: u64 = 2_000_000;
/// Nor more than this many files.
const MAX_CHANGED_FILES: usize = 200;

/// Reads everything the gutter, explorer and CHANGES panel show; `open`
/// are the files open in tabs. `None` when git can't be asked.
pub fn snapshot(repo: &Path, open: &[PathBuf]) -> Option<Snapshot> {
    let status = status(repo)?;
    let is_open = |path: &Path| open.iter().any(|o| normalize(o) == normalize(path));
    let bases = open.iter().map(|p| (p.clone(), staged_text(repo, p))).collect();
    let files = status
        .changed()
        .into_iter()
        .take(MAX_CHANGED_FILES)
        .map(|(path, status)| {
            let base = staged_text(repo, &path);
            let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_DIFF_BYTES);
            let current = (!is_open(&path) && small)
                .then(|| std::fs::read_to_string(&path).ok())
                .flatten()
                .map(|text| text.trim_start_matches('\u{feff}').replace("\r\n", "\n"))
                .map(|text| {
                    let hunks = diff::hunks(base.as_deref().unwrap_or(""), &text);
                    (text, hunks)
                });
            ChangedFile { path, status, base, current }
        })
        .collect();
    Some(Snapshot { status, bases, files })
}

/// What Godot 4 writes into a new project's `.gitignore` and
/// `.gitattributes`.
const GODOT_IGNORE: &[&str] = &["# Godot 4+ specific ignores", ".godot/", "/android/"];
const GODOT_ATTRIBUTES: &[&str] = &["# Normalize EOL for all files that Git considers text files.", "* text=auto eol=lf"];

/// Adds Godot's `.gitignore` and `.gitattributes` lines to the project
/// folder, keeping what is there and skipping lines already present.
/// Returns the files changed.
pub fn add_godot_ignores(root: &Path) -> Result<Vec<&'static str>, String> {
    let mut changed = Vec::new();
    for (name, lines) in [(".gitignore", GODOT_IGNORE), (".gitattributes", GODOT_ATTRIBUTES)] {
        let path = root.join(name);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let Some(text) = with_lines(&existing, lines) else {
            continue;
        };
        std::fs::write(&path, text).map_err(|e| format!("Cannot write {name}: {e}"))?;
        changed.push(name);
    }
    Ok(changed)
}

/// `existing` with the rule lines of `lines` it lacks appended (comments
/// only come along with them); `None` when nothing is missing.
fn with_lines(existing: &str, lines: &[&str]) -> Option<String> {
    let present = |line: &str| existing.lines().any(|l| l.trim() == line);
    let missing: Vec<&str> = lines.iter().copied().filter(|l| !l.starts_with('#') && !present(l)).collect();
    if missing.is_empty() {
        return None;
    }
    let mut text = existing.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    for line in lines.iter().filter(|l| l.starts_with('#') || missing.contains(l)) {
        text.push_str(line);
        text.push('\n');
    }
    Some(text)
}

pub fn status(repo: &Path) -> Option<Status> {
    let out = git(repo, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    Some(parse_status(repo, &out))
}

/// Parses `git status --porcelain=v1 -z`: `XY path\0`, renames followed by
/// their old path.
fn parse_status(repo: &Path, out: &[u8]) -> Status {
    let mut status = Status::default();
    let mut entries = out.split(|&b| b == 0).filter(|e| e.len() > 3);
    while let Some(entry) = entries.next() {
        let (x, y) = (entry[0], entry[1]);
        let path = String::from_utf8_lossy(&entry[3..]);
        if x == b'R' || x == b'C' {
            entries.next(); // The old path.
        }
        let kind = match (x, y) {
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileStatus::Conflicted,
            (b'?', _) | (b'A', _) => FileStatus::Added,
            // Deleted files aren't in the explorer.
            (_, b'D') | (b'D', _) => continue,
            _ => FileStatus::Modified,
        };
        let full: PathBuf = repo.join(path.as_ref()).components().collect();
        let mut folder = full.parent();
        while let Some(dir) = folder.filter(|d| d.starts_with(repo)) {
            if !status.folders.insert(normalize(dir)) {
                break;
            }
            folder = dir.parent();
        }
        status.files.insert(normalize(&full), (full, kind));
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the IDE's own repository, when git is there to ask.
    #[test]
    fn reads_this_repository() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(repo) = repo_root(crate_dir) else {
            return;
        };
        assert!(repo.join("Cargo.toml").is_file());
        let staged = staged_text(&repo, &crate_dir.join("Cargo.toml")).unwrap();
        assert!(staged.contains("[package]") && !staged.contains('\r'));
        assert_eq!(staged_text(&repo, &crate_dir.join("no-such-file.gd")), None);
        assert!(status(&repo).is_some());

        // Changed files (if any right now) are real paths, with their text.
        let snapshot = snapshot(&repo, &[]).unwrap();
        for file in snapshot.files.iter().filter(|f| f.status == FileStatus::Modified) {
            assert!(file.path.is_file(), "{}", file.path.display());
            assert!(!file.path.to_string_lossy().contains('/') || !cfg!(windows));
            // Changes that are all staged have no hunks against the index.
            assert!(file.base.is_some() && file.current.is_some(), "{}", file.path.display());
        }
    }

    #[test]
    fn parses_porcelain_status() {
        let repo = Path::new("/r");
        let out = b" M player/player.gd\0?? new.gd\0R  b.gd\0a.gd\0 D gone.gd\0UU conflict.tscn\0A  added/x.gd\0";
        let s = parse_status(repo, out);
        assert_eq!(s.file(Path::new("/r/player/player.gd")), Some(FileStatus::Modified));
        assert_eq!(s.file(Path::new("/r/new.gd")), Some(FileStatus::Added));
        assert_eq!(s.file(Path::new("/r/b.gd")), Some(FileStatus::Modified));
        assert_eq!(s.file(Path::new("/r/a.gd")), None);
        assert_eq!(s.file(Path::new("/r/gone.gd")), None);
        assert_eq!(s.file(Path::new("/r/conflict.tscn")), Some(FileStatus::Conflicted));
        assert!(s.folder_changed(Path::new("/r/player")) && s.folder_changed(Path::new("/r/added")));
        assert!(!s.folder_changed(Path::new("/r/other")));
        let names: Vec<PathBuf> = s.changed().into_iter().map(|(p, _)| p).collect();
        assert_eq!(names.len(), 5);
        assert!(names.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn appends_only_missing_ignore_lines() {
        assert_eq!(with_lines("", GODOT_IGNORE).unwrap(), "# Godot 4+ specific ignores\n.godot/\n/android/\n");
        assert_eq!(with_lines("*.log\n.godot/", GODOT_IGNORE).unwrap(), "*.log\n.godot/\n\n# Godot 4+ specific ignores\n/android/\n");
        assert_eq!(with_lines("# mine\n.godot/\n/android/\n", GODOT_IGNORE), None);

        let root = std::env::temp_dir().join(format!("godot-ide-gitignore-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".gitignore"), "export/\n").unwrap();
        assert_eq!(add_godot_ignores(&root).unwrap(), [".gitignore", ".gitattributes"]);
        assert!(std::fs::read_to_string(root.join(".gitignore")).unwrap().starts_with("export/\n\n# Godot"));
        assert!(add_godot_ignores(&root).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn git_paths_match_explorer_paths() {
        // git prints `C:/Users/…`; the explorer has `C:\Users\…`.
        let s = parse_status(Path::new("C:/Repo"), b" M scenes/main.tscn\0");
        assert_eq!(s.file(Path::new(r"c:\repo\Scenes\main.tscn")), Some(FileStatus::Modified));
        assert!(s.folder_changed(Path::new(r"C:\Repo\scenes")));
    }
}
