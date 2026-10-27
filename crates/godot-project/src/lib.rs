//! Godot project awareness: `project.godot` parsing and `res://` paths.

mod input;
mod settings;

pub use input::{Autoload, InputAction, BUILTIN_ACTIONS};
pub use settings::{Entry, ProjectSettings};

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

    /// Actions of the Input Map saved in the project (not the built-in
    /// `ui_*` ones it leaves unchanged), in file order.
    pub fn input_actions(&self) -> Vec<InputAction> {
        input::input_actions(&self.settings)
    }

    pub fn autoloads(&self) -> Vec<Autoload> {
        input::autoloads(&self.settings)
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

/// At most this many files are listed for `res://` path completion.
const MAX_RESOURCE_FILES: usize = 10_000;

impl GodotProject {
    /// Every file Godot can load (bookkeeping like `.import`/`.uid` files
    /// and the `.godot` folder left out), sorted.
    pub fn resource_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        walk_files(&self.root, &mut |path, name| {
            if !is_hidden(name, false) && files.len() < MAX_RESOURCE_FILES {
                files.push(path.to_path_buf());
            }
        });
        files.sort();
        files
    }

    /// The file with this `uid://`, found the way Godot records it: in a
    /// `.uid` file next to scripts and shaders, in the header of `.tscn` /
    /// `.tres` files, or in the `.import` file next to imported assets.
    pub fn find_uid(&self, uid: &str) -> Option<PathBuf> {
        self.find_uids(&[uid]).pop().flatten()
    }

    /// The files of several `uid://`s, found in one pass over the project.
    pub fn find_uids(&self, uids: &[&str]) -> Vec<Option<PathBuf>> {
        let mut found = vec![None; uids.len()];
        if uids.is_empty() {
            return found;
        }
        walk_files(&self.root, &mut |path, name| {
            if found.iter().all(Option::is_some) {
                return;
            }
            let (owner, recorded) = if let Some(owner) = name.strip_suffix(".uid") {
                (path.with_file_name(owner), std::fs::read_to_string(path).ok().map(|t| t.trim().to_string()))
            } else if let Some(owner) = name.strip_suffix(".import") {
                (path.with_file_name(owner), std::fs::read_to_string(path).ok().and_then(|t| import_uid(&t)))
            } else if name.ends_with(".tscn") || name.ends_with(".tres") {
                (path.to_path_buf(), first_line(path).and_then(|l| header_uid(&l)))
            } else {
                return;
            };
            let Some(recorded) = recorded else {
                return;
            };
            for (uid, slot) in uids.iter().zip(&mut found) {
                if slot.is_none() && *uid == recorded {
                    *slot = Some(owner.clone());
                }
            }
        });
        found
    }
}

/// Calls `visit` with every file under `dir`, skipping hidden folders.
fn walk_files(dir: &Path, visit: &mut dyn FnMut(&Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(t) if t.is_dir() && !is_hidden(&name, true) && !name.starts_with('.') => walk_files(&path, visit),
            Ok(t) if t.is_file() => visit(&path, &name),
            _ => {}
        }
    }
}

fn first_line(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let mut line = String::new();
    io::BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut line).ok()?;
    Some(line)
}

/// `uid="uid://…"` in a `[gd_scene …]` / `[gd_resource …]` header line.
fn header_uid(line: &str) -> Option<String> {
    if !line.starts_with("[gd_") {
        return None;
    }
    quoted_value(line, "uid")
}

/// `uid="uid://…"` in the `[remap]` section of an `.import` file.
fn import_uid(text: &str) -> Option<String> {
    text.lines().find_map(|line| line.strip_prefix("uid=").map(|v| v.trim().trim_matches('"').to_string()))
}

fn quoted_value(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!(" {key}=\""))? + key.len() + 3;
    let len = line[start..].find('"')?;
    Some(line[start..start + len].to_string())
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
    fn reads_uids_from_headers_and_import_files() {
        let scene = "[gd_scene load_steps=2 format=3 uid=\"uid://b1x\"]\n";
        assert_eq!(header_uid(scene).as_deref(), Some("uid://b1x"));
        assert_eq!(header_uid("[ext_resource uid=\"uid://c2\"]"), None);
        let import = "[remap]\n\nimporter=\"texture\"\ntype=\"CompressedTexture2D\"\nuid=\"uid://dq4\"\npath=\"res://.godot/imported/x.ctex\"\n";
        assert_eq!(import_uid(import).as_deref(), Some("uid://dq4"));
    }

    #[test]
    fn finds_files_by_uid() {
        let root = std::env::temp_dir().join(format!("godot-project-uid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scenes")).unwrap();
        std::fs::create_dir_all(root.join(".godot")).unwrap();
        std::fs::write(root.join("player.gd"), "extends Node\n").unwrap();
        std::fs::write(root.join("player.gd.uid"), "uid://aaa\n").unwrap();
        std::fs::write(root.join("scenes").join("main.tscn"), "[gd_scene format=3 uid=\"uid://bbb\"]\n").unwrap();
        std::fs::write(root.join("icon.svg"), "<svg/>").unwrap();
        std::fs::write(root.join("icon.svg.import"), "[remap]\nuid=\"uid://ccc\"\n").unwrap();
        std::fs::write(root.join(".godot").join("cache.tres"), "[gd_resource uid=\"uid://zzz\"]\n").unwrap();
        let project = GodotProject { root: root.clone(), settings: ProjectSettings::parse("") };

        assert_eq!(project.find_uid("uid://aaa"), Some(root.join("player.gd")));
        assert_eq!(project.find_uid("uid://bbb"), Some(root.join("scenes").join("main.tscn")));
        assert_eq!(project.find_uid("uid://ccc"), Some(root.join("icon.svg")));
        assert_eq!(project.find_uid("uid://zzz"), None);
        let files = project.resource_files();
        assert_eq!(files, [root.join("icon.svg"), root.join("player.gd"), root.join("scenes").join("main.tscn")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hidden_entries() {
        assert!(is_hidden(".godot", true));
        assert!(is_hidden("icon.svg.import", false));
        assert!(is_hidden("player.gd.uid", false));
        assert!(!is_hidden("player.gd", false));
    }
}
