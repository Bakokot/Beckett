//! Theme format: a single TOML file with UI, syntax and terminal colors.

mod color;
mod share;
mod slots;

pub use color::{Color, ParseColorError};
pub use share::{from_share_code, to_share_code};
pub use slots::{Section, Slot, SLOTS};

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
    /// ANSI colors for the integrated terminal.
    #[serde(default)]
    pub terminal: TerminalColors,
}

/// The 16 ANSI colors; foreground/background come from `ui`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TerminalColors {
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

impl TerminalColors {
    /// In ANSI order: the 8 base colors, then their bright variants.
    pub fn ansi(&self) -> [Color; 16] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
            self.bright_black,
            self.bright_red,
            self.bright_green,
            self.bright_yellow,
            self.bright_blue,
            self.bright_magenta,
            self.bright_cyan,
            self.bright_white,
        ]
    }
}

impl Default for TerminalColors {
    /// A dark palette, for themes that don't define one.
    fn default() -> Self {
        Self {
            black: Color::rgb(0x4c, 0x56, 0x6a),
            red: Color::rgb(0xff, 0x70, 0x85),
            green: Color::rgb(0x8e, 0xef, 0x97),
            yellow: Color::rgb(0xff, 0xed, 0xa1),
            blue: Color::rgb(0x57, 0xb3, 0xff),
            magenta: Color::rgb(0xff, 0x8c, 0xcc),
            cyan: Color::rgb(0x42, 0xff, 0xc2),
            white: Color::rgb(0xcd, 0xcf, 0xd2),
            bright_black: Color::rgb(0x6b, 0x71, 0x7c),
            bright_red: Color::rgb(0xff, 0x9a, 0xb0),
            bright_green: Color::rgb(0xb6, 0xff, 0xbe),
            bright_yellow: Color::rgb(0xff, 0xf4, 0xc4),
            bright_blue: Color::rgb(0x8f, 0xd0, 0xff),
            bright_magenta: Color::rgb(0xff, 0xb3, 0xe0),
            bright_cyan: Color::rgb(0x8f, 0xff, 0xdb),
            bright_white: Color::rgb(0xff, 0xff, 0xff),
        }
    }
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

/// A file name for a theme: lowercase letters, digits and dashes.
pub fn file_name(theme_name: &str) -> String {
    let mut slug = String::new();
    for c in theme_name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    format!("{}.toml", if slug.is_empty() { "theme" } else { slug })
}

/// Loads every `*.toml` theme in `dir`, with the file it came from. A
/// missing directory is not an error.
pub fn load_dir(dir: &Path) -> (Vec<(PathBuf, Theme)>, Vec<LoadError>) {
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
            Ok(theme) => themes.push((path, theme)),
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
    fn file_names_are_safe() {
        assert_eq!(file_name("Godot Dark (custom)"), "godot-dark-custom.toml");
        assert_eq!(file_name("Thé été!"), "thé-été.toml");
        assert_eq!(file_name("../.."), "theme.toml");
    }

    #[test]
    fn rejects_bad_color() {
        let text = include_str!("../themes/godot-dark.toml")
            .replacen("#1d2229", "#12345", 1);
        assert!(Theme::from_toml(&text).is_err());
    }
}
