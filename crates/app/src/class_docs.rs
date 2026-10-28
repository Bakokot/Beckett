//! The engine's class reference for the Inspector: written once per Godot
//! version by `godot --headless --doctool <dir>` into Beckett's cache
//! folder (about a second), then read from there.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use godot_project::ClassDb;
use lsp_client::godot::GodotInstall;

/// How far the class reference is.
#[derive(Debug, Clone, Default)]
pub enum ClassDocs {
    /// Not asked for yet (or the Godot used changed).
    #[default]
    Missing,
    Loading,
    Ready(Arc<ClassDb>),
    Failed(String),
}

/// Where the reference of this Godot is kept.
pub fn dir(install: &GodotInstall) -> Option<PathBuf> {
    let name = match install.version_string() {
        Some(version) => version,
        None => install.path.file_stem()?.to_string_lossy().into_owned(),
    };
    Some(dirs::cache_dir()?.join("beckett").join("class-reference").join(name))
}

/// Writes the reference into `dir` if it isn't there yet, then reads it.
pub fn load(exe: &Path, dir: &Path) -> Result<ClassDb, String> {
    let written = dir.join("doc").join("classes").join("Object.xml");
    if !written.is_file() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        let mut command = Command::new(lsp_client::godot::console_variant(exe));
        // Run outside any project, so only the engine's classes are written.
        command
            .args(["--headless", "--doctool"])
            .arg(dir)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let status = command.status().map_err(|e| format!("Cannot run Godot: {e}"))?;
        if !written.is_file() {
            return Err(format!("Godot didn't write its class reference ({status})"));
        }
    }
    let db = ClassDb::load(dir);
    if db.is_empty() {
        return Err(format!("No class in {}", dir.display()));
    }
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real Godot: `BECKETT_GODOT=exe BECKETT_DOCS_OUT=dir cargo test -p beckett -- --ignored`.
    #[test]
    #[ignore]
    fn writes_and_reads_the_reference() {
        let exe = PathBuf::from(std::env::var("BECKETT_GODOT").expect("BECKETT_GODOT"));
        let dir = PathBuf::from(std::env::var("BECKETT_DOCS_OUT").expect("BECKETT_DOCS_OUT"));
        let db = load(&exe, &dir).unwrap();
        assert!(db.class("Node2D").is_some());
        // Read again from what was written.
        assert!(load(&exe, &dir).unwrap().class("Control").is_some());
    }
}
