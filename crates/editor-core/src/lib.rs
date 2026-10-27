//! UI-independent editor logic: text buffer, language detection, lexing.

pub mod buffer;
pub mod diff;
pub mod folding;
pub mod fuzzy;
pub mod gdscript;
pub mod highlight;
pub mod search;
pub mod snippet;
pub mod symbols;

pub use buffer::{Buffer, Command, Motion, Pos, Selection};

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    GDScript,
    /// `.gdshader`, `.gdshaderinc`.
    Shader,
    /// Scenes, resources and Godot config files (`.tscn`, `.tres`, `project.godot`, `.cfg`…).
    Resource,
    Json,
    Markdown,
    Plain,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let ext = path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("gd") => Language::GDScript,
            Some("gdshader" | "gdshaderinc" | "shader") => Language::Shader,
            Some("tscn" | "tres" | "godot" | "cfg" | "import" | "gdextension" | "remap") => Language::Resource,
            Some("json") => Language::Json,
            Some("md" | "markdown") => Language::Markdown,
            _ => Language::Plain,
        }
    }

    /// What Toggle Comment puts at the start of a line (none for JSON and
    /// Markdown, which have no line comments).
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::GDScript | Language::Plain => Some("#"),
            Language::Shader => Some("//"),
            Language::Resource => Some(";"),
            Language::Json | Language::Markdown => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::GDScript => "GDScript",
            Language::Shader => "Godot Shader",
            Language::Resource => "Godot Resource",
            Language::Json => "JSON",
            Language::Markdown => "Markdown",
            Language::Plain => "Plain Text",
        }
    }
}
