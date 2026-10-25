//! Finding installed Godot executables.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::uri::path_key;

pub type Version = (u32, u32, u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotInstall {
    pub path: PathBuf,
    /// Parsed from the file name (e.g. `Godot_v4.5.1-stable_win64.exe`).
    pub version: Option<Version>,
}

impl GodotInstall {
    pub fn version_string(&self) -> Option<String> {
        self.version.map(|(major, minor, patch)| {
            if patch == 0 {
                format!("{major}.{minor}")
            } else {
                format!("{major}.{minor}.{patch}")
            }
        })
    }
}

/// On Windows, `Godot_..._win64.exe` doesn't print to a console; its
/// `_console.exe` sibling does. Elsewhere, and if there is none, `exe` itself.
pub fn console_variant(exe: &Path) -> PathBuf {
    let (Some(stem), Some(dir)) = (exe.file_stem(), exe.parent()) else {
        return exe.to_path_buf();
    };
    let console = dir.join(format!("{}_console.exe", stem.to_string_lossy()));
    if cfg!(windows) && console.is_file() {
        console
    } else {
        exe.to_path_buf()
    }
}

/// Godot executables from `GODOT_BIN`/`GODOT`, `PATH`, and the usual
/// download and install folders.
pub fn discover() -> Vec<GodotInstall> {
    let mut candidates = Vec::new();
    for var in ["GODOT_BIN", "GODOT"] {
        if let Some(path) = env::var_os(var) {
            candidates.push(PathBuf::from(path));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            for name in PATH_NAMES {
                candidates.push(dir.join(name));
            }
        }
    }
    for dir in search_dirs() {
        walk(&dir, 2, &mut candidates);
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file() && seen.insert(path_key(path)))
        .map(|path| GodotInstall {
            version: path.file_name().and_then(|n| version_from_name(&n.to_string_lossy())),
            path,
        })
        .collect()
}

/// Picks the best install for a project saved with `wanted` (e.g. "4.5"):
/// the newest matching major.minor, otherwise the newest overall.
pub fn pick<'a>(installs: &'a [GodotInstall], wanted: Option<&str>) -> Option<&'a GodotInstall> {
    let wanted = wanted.and_then(version_from_str);
    let newest = |filter: &dyn Fn(&GodotInstall) -> bool| {
        installs
            .iter()
            .filter(|i| filter(i))
            .max_by_key(|i| i.version)
    };
    wanted
        .and_then(|(major, minor, _)| {
            newest(&|i| matches!(i.version, Some((ma, mi, _)) if ma == major && mi == minor))
        })
        .or_else(|| newest(&|_| true))
}

/// Whether `install` has the major.minor of `wanted` (e.g. "4.5"). Unknown
/// versions on either side count as a match: there is nothing to warn about.
pub fn matches(install: &GodotInstall, wanted: Option<&str>) -> bool {
    match (install.version, wanted.and_then(version_from_str)) {
        (Some((major, minor, _)), Some((wanted_major, wanted_minor, _))) => {
            (major, minor) == (wanted_major, wanted_minor)
        }
        _ => true,
    }
}

const PATH_NAMES: &[&str] = if cfg!(windows) {
    &["godot.exe", "godot4.exe"]
} else {
    &["godot", "godot4"]
};

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.extend(dirs::desktop_dir());
    dirs.extend(dirs::download_dir());
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("scoop").join("apps"));
        dirs.push(home.join("Applications"));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = env::var_os(var) {
                dirs.push(PathBuf::from(&dir));
                dirs.push(PathBuf::from(dir).join("Steam").join("steamapps").join("common"));
            }
        }
        if let Some(local) = dirs::data_local_dir() {
            dirs.push(local.join("Programs"));
            dirs.push(local.join("Microsoft").join("WinGet").join("Packages"));
        }
    } else {
        dirs.push(PathBuf::from("/Applications"));
    }
    dirs
}

/// Collects Godot binaries in `dir`, descending only into Godot-named folders
/// so large folders like Downloads stay cheap to scan.
fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && is_godot_binary(&name) {
            out.push(entry.path());
        } else if file_type.is_dir() && name.contains("godot") {
            if name.ends_with(".app") {
                out.push(entry.path().join("Contents").join("MacOS").join("Godot"));
            } else if depth > 0 {
                walk(&entry.path(), depth - 1, out);
            }
        }
    }
}

fn is_godot_binary(lower_name: &str) -> bool {
    if !lower_name.starts_with("godot") || lower_name.starts_with("godot-ide") {
        return false;
    }
    if cfg!(windows) {
        lower_name.ends_with(".exe") && !lower_name.contains("console")
    } else {
        lower_name.ends_with(".x86_64")
            || lower_name.ends_with(".arm64")
            || matches!(lower_name, "godot" | "godot4")
    }
}

/// Finds a `v4.5.1`-style version in a file name.
fn version_from_name(name: &str) -> Option<Version> {
    name.match_indices(['v', 'V'])
        .find_map(|(i, _)| version_from_str(&name[i + 1..]))
}

/// Parses a leading `major.minor[.patch]`.
fn version_from_str(s: &str) -> Option<Version> {
    let mut parts = s.split('.');
    let number = |part: Option<&str>| -> Option<u32> {
        let digits: String = part?.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(parts.next())?;
    let minor = number(parts.next())?;
    let patch = number(parts.next()).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_from_names() {
        assert_eq!(version_from_name("Godot_v4.7.1-stable_win64.exe"), Some((4, 7, 1)));
        assert_eq!(version_from_name("Godot_v4.5-stable_linux.x86_64"), Some((4, 5, 0)));
        assert_eq!(version_from_name("godot.exe"), None);
        assert_eq!(version_from_str("4.3"), Some((4, 3, 0)));
    }

    #[test]
    fn matches_on_major_minor() {
        let install = |version| GodotInstall { path: PathBuf::from("godot"), version };
        assert!(matches(&install(Some((4, 5, 1))), Some("4.5")));
        assert!(!matches(&install(Some((4, 7, 1))), Some("4.5")));
        assert!(matches(&install(None), Some("4.5")));
        assert!(matches(&install(Some((4, 7, 1))), None));
    }

    #[test]
    fn recognizes_binaries() {
        if cfg!(windows) {
            assert!(is_godot_binary("godot_v4.7.1-stable_win64.exe"));
            assert!(!is_godot_binary("godot_v4.7.1-stable_win64_console.exe"));
        }
        assert!(!is_godot_binary("godot-ide.exe"));
        assert!(!is_godot_binary("notes.txt"));
    }

    #[test]
    fn picks_matching_version_then_newest() {
        let install = |v| GodotInstall { path: PathBuf::from("g"), version: Some(v) };
        let installs = [install((4, 5, 1)), install((4, 7, 1)), install((4, 5, 0))];
        assert_eq!(pick(&installs, Some("4.5")).unwrap().version, Some((4, 5, 1)));
        assert_eq!(pick(&installs, Some("4.2")).unwrap().version, Some((4, 7, 1)));
        assert_eq!(pick(&installs, None).unwrap().version, Some((4, 7, 1)));
        assert_eq!(pick(&[], Some("4.5")), None);
    }
}
