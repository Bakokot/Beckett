//! Theme format: a single TOML file with UI and syntax colors.

mod color;

pub use color::{Color, ParseColorError};

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default)]
    pub kind: ThemeKind,
    pub ui: UiColors,
    pub syntax: SyntaxColors,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiColors {
    /// Editor background.
    pub background: Color,
    /// Sidebars, tab bar, status bar.
    pub surface: Color,
    pub text: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub border: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub text: Color,
    pub keyword: Color,
    pub control_flow: Color,
    pub r#type: Color,
    pub function: Color,
    pub annotation: Color,
    pub string: Color,
    pub string_name: Color,
    pub node_path: Color,
    pub number: Color,
    pub comment: Color,
    pub constant: Color,
    pub symbol: Color,
}

impl Theme {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

const BUILTIN: &[&str] = &[
    include_str!("../themes/godot-dark.toml"),
    include_str!("../themes/godot-light.toml"),
];

/// Themes shipped with the IDE.
pub fn builtin() -> Vec<Theme> {
    BUILTIN
        .iter()
        .map(|text| Theme::from_toml(text).expect("built-in theme is valid"))
        .collect()
}

/// Directory users drop shared `.toml` themes into.
pub fn user_themes_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("godot-ide").join("themes"))
}

#[derive(Debug)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// Loads every `*.toml` theme in `dir`. A missing directory is not an error.
pub fn load_dir(dir: &Path) -> (Vec<Theme>, Vec<LoadError>) {
    let mut themes = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (themes, errors);
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    for path in paths {
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| Theme::from_toml(&text).map_err(|e| e.to_string()));
        match result {
            Ok(theme) => themes.push(theme),
            Err(message) => errors.push(LoadError { path, message }),
        }
    }
    (themes, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse_and_round_trip() {
        for theme in builtin() {
            let text = theme.to_toml().unwrap();
            assert_eq!(Theme::from_toml(&text).unwrap(), theme);
        }
    }

    #[test]
    fn rejects_bad_color() {
        let text = include_str!("../themes/godot-dark.toml")
            .replacen("#1d2229", "#12345", 1);
        assert!(Theme::from_toml(&text).is_err());
    }
}
