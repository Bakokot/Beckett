//! Godot project awareness: `project.godot` parsing and `res://` paths.

mod settings;

pub use settings::ProjectSettings;

use std::io;
use std::path::{Component, Path, PathBuf};

pub const PROJECT_FILE: &str = "project.godot";

#[derive(Debug, Clone)]
pub struct GodotProject {
    pub root: PathBuf,
    pub settings: ProjectSettings,
}

impl GodotProject {
    pub fn load(root: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(root.join(PROJECT_FILE))?;
        Ok(Self {
            root: root.to_path_buf(),
            settings: ProjectSettings::parse(&text),
        })
    }

    pub fn name(&self) -> Option<String> {
        self.settings.get_string("application/config/name")
    }

    pub fn main_scene(&self) -> Option<String> {
        self.settings.get_string("application/run/main_scene")
    }

    pub fn features(&self) -> Vec<String> {
        self.settings.get_string_array("application/config/features")
    }

    /// The engine version the project was last saved with, e.g. "4.3".
    pub fn godot_version(&self) -> Option<String> {
        self.features()
            .into_iter()
            .find(|f| f.starts_with(|c: char| c.is_ascii_digit()))
    }

    /// Maps `res://path/to/file` to a filesystem path inside the project.
    pub fn resolve(&self, res_path: &str) -> Option<PathBuf> {
        let rest = res_path.strip_prefix("res://")?;
        let mut path = self.root.clone();
        for part in rest.split('/').filter(|p| !p.is_empty()) {
            if part == ".." {
                return None;
            }
            path.push(part);
        }
        Some(path)
    }

    /// Maps a filesystem path inside the project to its `res://` form.
    pub fn to_res_path(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<_> = relative
            .components()
            .map(|c| match c {
                Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect::<Option<_>>()?;
        Some(format!("res://{}", parts.join("/")))
    }
}

impl GodotProject {
    /// Scene files (`.tscn`) that reference the resource at `res_path`,
    /// e.g. the scenes a script is attached to.
    pub fn scenes_using(&self, res_path: &str) -> Vec<PathBuf> {
        let mut scenes = Vec::new();
        collect_scenes(&self.root, &mut scenes);
        scenes.retain(|scene| {
            std::fs::read_to_string(scene).is_ok_and(|text| scene_references(&text, res_path))
        });
        scenes.sort();
        scenes
    }
}

fn collect_scenes(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(t) if t.is_dir() && !is_hidden(&name, true) => collect_scenes(&path, out),
            Ok(t) if t.is_file() && name.ends_with(".tscn") => out.push(path),
            _ => {}
        }
    }
}

/// Whether scene text has an `ext_resource` pointing at `res_path`.
fn scene_references(text: &str, res_path: &str) -> bool {
    let quoted = format!("path=\"{res_path}\"");
    text.lines()
        .filter(|line| line.starts_with("[ext_resource"))
        .any(|line| line.contains(&quoted))
}

/// Finds the nearest directory at or above `start` containing `project.godot`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(PROJECT_FILE).is_file())
        .map(Path::to_path_buf)
}

/// Whether an explorer entry is Godot/VCS bookkeeping that should be hidden.
pub fn is_hidden(name: &str, is_dir: bool) -> bool {
    if is_dir {
        matches!(name, ".godot" | ".git" | ".import")
    } else {
        name.ends_with(".import") || name.ends_with(".uid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> GodotProject {
        GodotProject {
            root: PathBuf::from("/games/demo"),
            settings: ProjectSettings::parse(
                "[application]\nconfig/name=\"Demo\"\nconfig/features=PackedStringArray(\"4.3\", \"Forward Plus\")\n",
            ),
        }
    }

    #[test]
    fn metadata() {
        let p = project();
        assert_eq!(p.name().as_deref(), Some("Demo"));
        assert_eq!(p.godot_version().as_deref(), Some("4.3"));
    }

    #[test]
    fn res_paths_round_trip() {
        let p = project();
        let path = p.resolve("res://scenes/main.tscn").unwrap();
        assert_eq!(path, Path::new("/games/demo").join("scenes").join("main.tscn"));
        assert_eq!(p.to_res_path(&path).as_deref(), Some("res://scenes/main.tscn"));
        assert_eq!(p.resolve("res://../secret"), None);
        assert_eq!(p.resolve("user://save.dat"), None);
    }

    #[test]
    fn detects_scene_references() {
        let scene = "[gd_scene load_steps=2 format=3 uid=\"uid://b1\"]\n\n\
            [ext_resource type=\"Script\" uid=\"uid://c2\" path=\"res://scenes/player.gd\" id=\"1_abc\"]\n\n\
            [node name=\"Player\" type=\"CharacterBody2D\"]\nscript = ExtResource(\"1_abc\")\n";
        assert!(scene_references(scene, "res://scenes/player.gd"));
        assert!(!scene_references(scene, "res://player.gd"));
        assert!(!scene_references("[node name=\"x\"]\n# path=\"res://scenes/player.gd\"", "res://scenes/player.gd"));
    }

    #[test]
    fn hidden_entries() {
        assert!(is_hidden(".godot", true));
        assert!(is_hidden("icon.svg.import", false));
        assert!(is_hidden("player.gd.uid", false));
        assert!(!is_hidden("player.gd", false));
    }
}
