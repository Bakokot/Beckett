//! Explorer file operations: new folder, rename and delete, with the bar
//! that asks for the name (or confirms the delete). Godot's companion files
//! (`x.gd.uid`, `x.png.import`) follow the file they belong to, so its
//! `uid://` keeps working.

use std::path::{Path, PathBuf};

use iced::widget::tooltip::Position;
use iced::widget::{button, container, row, space, text, text_input};
use iced::{Alignment, Element, Length};

use crate::style;

pub const INPUT: &str = "file-op-input";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
}

#[derive(Debug, Clone)]
pub enum FileOpMessage {
    Input(String),
    Submit,
    Close,
}

pub struct FileOpBar {
    pub op: FileOp,
    /// The name typed (unused for a delete).
    pub name: String,
    /// How the target is shown, e.g. `res://player/player.gd`.
    pub label: String,
    pub error: Option<String>,
}

impl FileOpBar {
    pub fn new(op: FileOp, label: String) -> Self {
        let name = match &op {
            FileOp::Rename(path) => file_name(path),
            FileOp::NewFolder(_) | FileOp::Delete(_) => String::new(),
        };
        Self { op, name, label, error: None }
    }

    /// Chars of the name to select when the bar opens: the file name
    /// without its extension.
    pub fn initial_selection(&self) -> (usize, usize) {
        let stem = match &self.op {
            FileOp::Rename(path) if !path.is_dir() => self.name.rfind('.').filter(|&i| i > 0).unwrap_or(self.name.len()),
            _ => self.name.len(),
        };
        (0, self.name[..stem].chars().count())
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors) -> Element<'a, FileOpMessage> {
        let muted = style::to_color(ui.text_muted);
        let hint = match &self.error {
            Some(error) => text(error.as_str()).size(12).color(style::to_color(ui.danger)),
            None if matches!(self.op, FileOp::Delete(_)) => {
                text("Its open tabs close; unsaved edits are lost").size(12).color(muted)
            }
            None => text("Enter to confirm, Escape to cancel").size(12).color(muted),
        };
        let close = button(text("x").size(12)).on_press(FileOpMessage::Close).style(button::text).padding([2, 6]);
        let input = |placeholder: &'a str| {
            text_input(placeholder, &self.name)
                .id(INPUT)
                .on_input(FileOpMessage::Input)
                .on_submit(FileOpMessage::Submit)
                .size(13)
                .padding([3, 6])
                .width(280)
        };
        let mut content = row![].spacing(8).align_y(Alignment::Center);
        content = match &self.op {
            FileOp::NewFolder(_) => content.push(text(format!("New folder in {}", self.label)).size(12)).push(input("Folder name")),
            FileOp::Rename(_) => content.push(text(format!("Rename {}", self.label)).size(12)).push(input("New name")),
            FileOp::Delete(_) => content
                .push(text(format!("Move {} to the Recycle Bin?", self.label)).size(12))
                .push(button(text("Delete").size(12)).on_press(FileOpMessage::Submit).style(button::danger).padding([3, 10])),
        };
        let content = content.push(hint).push(space::horizontal()).push(style::tip(close, "Cancel (Escape)", Position::Bottom, ui));
        container(content).padding([4, 8]).width(Length::Fill).style(style::surface(ui)).into()
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A file or folder name valid on every system Godot runs on.
pub fn validate_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name".into());
    }
    if name == "." || name == ".." {
        return Err("Not a valid name".into());
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err(format!("Names can't contain {c:?}"));
    }
    if name.ends_with('.') {
        return Err("Names can't end with a dot".into());
    }
    Ok(name)
}

/// Godot's bookkeeping files next to `path`: `path.uid` (scripts,
/// shaders) and `path.import` (imported assets), when they exist.
pub fn companions(path: &Path) -> Vec<PathBuf> {
    let Some(name) = path.file_name() else {
        return Vec::new();
    };
    [".uid", ".import"]
        .into_iter()
        .map(|suffix| {
            let mut companion = name.to_os_string();
            companion.push(suffix);
            path.with_file_name(companion)
        })
        .filter(|p| p.is_file())
        .collect()
}

/// Renames `path` (and its companions) to `name` in the same folder;
/// returns the new path.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    let target = path.with_file_name(name);
    if target == path {
        return Ok(target);
    }
    // On Windows a case-only rename finds the file itself "already there".
    let same_file = file_name(path).eq_ignore_ascii_case(name);
    if target.exists() && !same_file {
        return Err(format!("{name} already exists"));
    }
    let companions = companions(path);
    std::fs::rename(path, &target).map_err(|e| format!("Cannot rename: {e}"))?;
    for companion in companions {
        let suffix = file_name(&companion)[file_name(path).len()..].to_string();
        let _ = std::fs::rename(&companion, target.with_file_name(format!("{name}{suffix}")));
    }
    Ok(target)
}

pub fn new_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let target = parent.join(name);
    if target.exists() {
        return Err(format!("{name} already exists"));
    }
    std::fs::create_dir(&target).map_err(|e| format!("Cannot create the folder: {e}"))?;
    Ok(target)
}

/// Moves `path` and its companions to the system's trash.
pub fn delete(path: &Path) -> Result<(), String> {
    let mut paths = vec![path.to_path_buf()];
    paths.extend(companions(path));
    trash::delete_all(&paths).map_err(|e| format!("Cannot delete: {e}"))
}

/// `path` after `from` was renamed to `to`: itself, or a file inside it.
pub fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(from).ok()?;
    Some(if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) })
}

/// How many of `files` mention `needle` (a `res://` path).
pub fn files_mentioning(files: &[PathBuf], needle: &str) -> usize {
    let text_file = |p: &&PathBuf| {
        p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e, "gd" | "tscn" | "tres" | "godot" | "cfg" | "gdshader"))
    };
    files.iter().filter(text_file).filter(|f| std::fs::read_to_string(f).is_ok_and(|t| t.contains(needle))).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("godot-ide-file-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn checks_names() {
        assert_eq!(validate_name("  player.gd "), Ok("player.gd"));
        for bad in ["", "..", "a/b", "a:b", "what?", "dot."] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn renames_with_companions() {
        let dir = scratch("rename");
        std::fs::write(dir.join("player.gd"), "extends Node\n").unwrap();
        std::fs::write(dir.join("player.gd.uid"), "uid://abc\n").unwrap();
        std::fs::write(dir.join("taken.gd"), "").unwrap();

        assert!(rename(&dir.join("player.gd"), "taken.gd").is_err());
        let target = rename(&dir.join("player.gd"), "hero.gd").unwrap();
        assert_eq!(target, dir.join("hero.gd"));
        assert!(dir.join("hero.gd").is_file() && dir.join("hero.gd.uid").is_file());
        assert!(!dir.join("player.gd").exists() && !dir.join("player.gd.uid").exists());
        // Changing only the case.
        assert_eq!(rename(&dir.join("hero.gd"), "Hero.gd").unwrap(), dir.join("Hero.gd"));

        let made = new_folder(&dir, "enemies").unwrap();
        assert!(made.is_dir() && new_folder(&dir, "enemies").is_err());
        std::fs::write(dir.join("taken.gd"), "const H = preload(\"res://player.gd\")\n").unwrap();
        // The .uid file mentions a uid but isn't a text resource.
        let files = [dir.join("Hero.gd.uid"), dir.join("taken.gd")];
        assert_eq!(files_mentioning(&files, "res://player.gd"), 1);
        assert_eq!(files_mentioning(&files, "uid://abc"), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn maps_paths_inside_a_renamed_folder() {
        let (from, to) = (Path::new("/p/enemies"), Path::new("/p/foes"));
        assert_eq!(moved(Path::new("/p/enemies/a/b.gd"), from, to), Some(PathBuf::from("/p/foes/a/b.gd")));
        assert_eq!(moved(from, from, to), Some(to.to_path_buf()));
        assert_eq!(moved(Path::new("/p/enemies2/x.gd"), from, to), None);
    }

    #[test]
    fn selects_the_stem() {
        let bar = FileOpBar::new(FileOp::Rename(PathBuf::from("/nonexistent/player.tscn")), String::new());
        assert_eq!(bar.initial_selection(), (0, 6));
    }
}
