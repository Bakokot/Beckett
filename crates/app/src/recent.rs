//! Recently opened projects, most recent first, kept in
//! `<config>/godot-ide/recent-projects.txt` (one folder per line).

use std::path::{Path, PathBuf};

use godot_project::GodotProject;
use lsp_client::uri;

const MAX: usize = 12;

/// A recent project as shown on the welcome screen.
#[derive(Debug, Clone)]
pub struct RecentProject {
    pub root: PathBuf,
    /// `config/name` from project.godot, else the folder name.
    pub name: String,
    pub godot_version: Option<String>,
    /// The folder is gone (moved, deleted, unplugged drive).
    pub missing: bool,
}

impl RecentProject {
    fn describe(root: PathBuf) -> Self {
        let project = GodotProject::load(&root).ok();
        let folder = root.file_name().map(|n| n.to_string_lossy().into_owned());
        let name = project
            .as_ref()
            .and_then(GodotProject::name)
            .or(folder)
            .unwrap_or_else(|| root.display().to_string());
        Self {
            godot_version: project.as_ref().and_then(GodotProject::godot_version),
            missing: !root.is_dir(),
            name,
            root,
        }
    }
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("godot-ide").join("recent-projects.txt"))
}

/// The saved list, with each project's name read from its project.godot.
pub fn load() -> Vec<RecentProject> {
    let text = file().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    parse(&text).into_iter().map(RecentProject::describe).collect()
}

/// Moves `root` to the top of the saved list and returns the new list.
pub fn add(root: &Path) -> Vec<RecentProject> {
    update(|roots| push(roots, root.to_path_buf()))
}

pub fn remove(root: &Path) -> Vec<RecentProject> {
    update(|roots| roots.retain(|r| !uri::same_path(r, root)))
}

fn update(change: impl FnOnce(&mut Vec<PathBuf>)) -> Vec<RecentProject> {
    let Some(path) = file() else {
        return Vec::new();
    };
    let mut roots = parse(&std::fs::read_to_string(&path).unwrap_or_default());
    change(&mut roots);
    // Failing to remember a project is not worth interrupting anyone.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format(&roots));
    roots.into_iter().map(RecentProject::describe).collect()
}

fn parse(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

fn format(roots: &[PathBuf]) -> String {
    roots.iter().map(|r| format!("{}\n", r.display())).collect()
}

fn push(roots: &mut Vec<PathBuf>, root: PathBuf) {
    roots.retain(|r| !uri::same_path(r, &root));
    roots.insert(0, root);
    roots.truncate(MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_moves_to_front_without_duplicates() {
        let mut roots = parse("C:\\games\\a\n\n  C:\\games\\b  \n");
        assert_eq!(roots, [PathBuf::from("C:\\games\\a"), PathBuf::from("C:\\games\\b")]);
        push(&mut roots, PathBuf::from("C:\\games\\b"));
        assert_eq!(roots, [PathBuf::from("C:\\games\\b"), PathBuf::from("C:\\games\\a")]);
        for i in 0..20 {
            push(&mut roots, PathBuf::from(format!("C:\\p{i}")));
        }
        assert_eq!(roots.len(), MAX);
        assert_eq!(roots[0], PathBuf::from("C:\\p19"));
        assert_eq!(parse(&format(&roots)), roots);
    }
}
