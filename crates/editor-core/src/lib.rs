//! UI-independent editor logic: language detection and syntax lexing.

pub mod gdscript;

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
