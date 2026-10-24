//! UI-independent editor logic: text buffer, language detection, lexing.

pub mod buffer;
pub mod fuzzy;
pub mod gdscript;
pub mod search;

pub use buffer::{Buffer, Command, Motion, Pos, Selection};

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    GDScript,
    Plain,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("gd") => Language::GDScript,
            _ => Language::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::GDScript => "GDScript",
            Language::Plain => "Plain Text",
        }
    }
}
