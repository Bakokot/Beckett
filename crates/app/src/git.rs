//! Git through the `git` command: the repository of the project, file
//! statuses for the explorer, and a file's staged text for the gutter's
//! change markers. Without git (or outside a repository) nothing is shown.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    files: HashMap<PathBuf, FileStatus>,
    /// Folders holding at least one changed file.
    folders: HashSet<PathBuf>,
}

impl Status {
    pub fn file(&self, path: &Path) -> Option<FileStatus> {
        self.files.get(&normalize(path)).copied()
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
    Some(PathBuf::from(root.trim()))
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
        let full = repo.join(path.as_ref());
        let mut folder = full.parent();
        while let Some(dir) = folder.filter(|d| d.starts_with(repo)) {
            if !status.folders.insert(normalize(dir)) {
                break;
            }
            folder = dir.parent();
        }
        status.files.insert(normalize(&full), kind);
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
