//! Personal settings that are not part of a theme, kept in
//! `<config>/beckett/settings.json`, and a project's overrides of some of
//! them, kept in `<project>/.beckett/settings.json` (Godot skips dot folders).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::SidebarView;

pub const ZOOM_STEP: f32 = 0.1;
/// The sidebar's width until it is dragged (double-clicking its edge comes back to it).
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 260.0;
pub const MIN_SIDEBAR_WIDTH: f32 = 160.0;
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Scale of the whole interface (Ctrl+= / Ctrl+- / Ctrl+0).
    pub ui_scale: f32,
    /// Format GDScript files (gdformat, else a light tidy) when saving.
    pub format_on_save: bool,
    /// Typing an opening bracket or quote adds the closing one.
    pub auto_close_brackets: bool,
    /// The completion list opens by itself while typing a name.
    pub completion_while_typing: bool,
    /// The cursor blinks (in the editor and the terminal).
    pub cursor_blink: bool,
    pub indent_guides: bool,
    /// Diagnostic messages written at the end of their line.
    pub inline_diagnostics: bool,
    /// How long the mouse rests on a word before its documentation shows.
    pub hover_delay_ms: u64,
    /// What the sidebar shows: files, outline or project.
    pub sidebar: SidebarView,
    /// Name of the theme in use, restored at startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Switch between `dark_theme` and `light_theme` as the OS goes dark or
    /// light.
    pub follow_os_theme: bool,
    /// The themes used with the OS in dark and light mode (default: Godot
    /// Dark / Godot Light).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    /// Width of the sidebar (files, outline, project), in pixels.
    pub sidebar_width: f32,
    /// Order of the theme list, by name (themes not in it come after).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub theme_order: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            format_on_save: false,
            auto_close_brackets: true,
            completion_while_typing: true,
            cursor_blink: true,
            indent_guides: true,
            inline_diagnostics: true,
            hover_delay_ms: 500,
            sidebar: SidebarView::Files,
            theme: None,
            follow_os_theme: false,
            dark_theme: None,
            light_theme: None,
            theme_order: Vec::new(),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl Settings {
    /// The saved settings; defaults when there are none or they can't be read.
    pub fn load() -> Self {
        let text = file().and_then(|path| std::fs::read_to_string(path).ok());
        // Hand-edited files may start with a byte order mark.
        let settings: Self =
            text.and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
        settings.sanitized()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = file().ok_or("No user config folder")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Zooms by `steps` (negative zooms out), within the allowed range.
    pub fn zoom(&mut self, steps: i32) {
        // Rounded to whole percents so repeated steps don't drift.
        let scale = ((self.ui_scale + steps as f32 * ZOOM_STEP) * 100.0).round() / 100.0;
        self.ui_scale = scale.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    }

    fn sanitized(mut self) -> Self {
        if !self.ui_scale.is_finite() {
            self.ui_scale = 1.0;
        }
        self.ui_scale = self.ui_scale.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        if !self.sidebar_width.is_finite() || self.sidebar_width < MIN_SIDEBAR_WIDTH {
            self.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
        }
        self.hover_delay_ms = self.hover_delay_ms.clamp(HOVER_DELAY_RANGE.0, HOVER_DELAY_RANGE.1);
        self
    }

    /// The value of the setting named `key` (as in the JSON file).
    pub fn value(&self, key: &str) -> Option<Value> {
        serde_json::to_value(self).ok()?.get(key).cloned()
    }

    /// These settings with `key` set to `value`; `None` when it isn't a
    /// value that setting takes. Out-of-range numbers are brought in range.
    pub fn with(&self, key: &str, value: Value) -> Option<Self> {
        let Ok(Value::Object(mut map)) = serde_json::to_value(self) else {
            return None;
        };
        // Only replace a value by one of the same type (`#[serde(default)]`
        // would silently turn a bad one into the default).
        let kind = |v: &Value| std::mem::discriminant(v);
        if map.get(key).map(kind) != Some(kind(&value)) {
            return None;
        }
        map.insert(key.to_string(), value);
        serde_json::from_value::<Self>(Value::Object(map)).ok().map(Self::sanitized)
    }

    /// The settings in effect in a project: these, with the project's
    /// overrides of the settings that allow them. Bad overrides are skipped.
    pub fn with_project(&self, overrides: &Map<String, Value>) -> Self {
        overrides
            .iter()
            .filter(|(key, _)| item(key).is_some_and(|i| i.project))
            .fold(self.clone(), |settings, (key, value)| settings.with(key, value.clone()).unwrap_or(settings))
    }
}

fn file() -> Option<PathBuf> {
    Some(theme::config_dir()?.join("settings.json"))
}

/// The user settings file (it may not exist yet).
pub fn user_file() -> Option<PathBuf> {
    file()
}

pub const HOVER_DELAY_RANGE: (u64, u64) = (100, 3000);

/// The project's settings file (it may not exist).
pub fn project_file(root: &Path) -> PathBuf {
    root.join(".beckett").join("settings.json")
}

/// The project's overrides; empty when there are none or they can't be read.
pub fn load_project(root: &Path) -> Map<String, Value> {
    std::fs::read_to_string(project_file(root))
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

/// Writes the project's overrides; none left removes the file.
pub fn save_project(root: &Path, overrides: &Map<String, Value>) -> Result<(), String> {
    let path = project_file(root);
    if overrides.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => {
                // The folder goes too when nothing else is in it.
                let _ = path.parent().map(std::fs::remove_dir);
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(overrides).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| e.to_string())
}

/// How a setting is changed on the settings page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Toggle,
    /// A number between `min` and `max` by `step`, shown times `scale`
    /// followed by `unit`.
    Number { min: f64, max: f64, step: f64, scale: f64, unit: &'static str },
}

/// A setting as the settings page shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    /// Its name in the JSON files.
    pub key: &'static str,
    pub section: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub kind: Kind,
    /// A project can override it.
    pub project: bool,
}

pub const ITEMS: &[Item] = &[
    Item {
        key: "auto_close_brackets",
        section: "Editor",
        title: "Auto-close brackets and quotes",
        description: "Typing ( [ { \" or ' adds the closing one; typing it again steps over it, Backspace removes both",
        kind: Kind::Toggle,
        project: true,
    },
    Item {
        key: "completion_while_typing",
        section: "Editor",
        title: "Suggest while typing",
        description: "Open the completion list as you type a name (else Ctrl+Space, or after . $ and quotes)",
        kind: Kind::Toggle,
        project: true,
    },
    Item {
        key: "hover_delay_ms",
        section: "Editor",
        title: "Hover documentation delay",
        description: "How long the mouse rests on a word before its documentation shows",
        kind: Kind::Number { min: HOVER_DELAY_RANGE.0 as f64, max: HOVER_DELAY_RANGE.1 as f64, step: 100.0, scale: 1.0, unit: " ms" },
        project: false,
    },
    Item {
        key: "inline_diagnostics",
        section: "Editor",
        title: "Inline diagnostics",
        description: "Write error and warning messages at the end of their line (the underlines stay)",
        kind: Kind::Toggle,
        project: true,
    },
    Item {
        key: "indent_guides",
        section: "Editor",
        title: "Indentation guides",
        description: "Vertical lines at each indentation level",
        kind: Kind::Toggle,
        project: false,
    },
    Item {
        key: "cursor_blink",
        section: "Editor",
        title: "Blinking cursor",
        description: "In the editor and the terminal",
        kind: Kind::Toggle,
        project: false,
    },
    Item {
        key: "format_on_save",
        section: "Files",
        title: "Format on save",
        description: "Format GDScript and shaders when saving (gdformat when installed, else trailing spaces and the final newline)",
        kind: Kind::Toggle,
        project: true,
    },
    Item {
        key: "ui_scale",
        section: "Interface",
        title: "Zoom",
        description: "Size of the whole interface (Ctrl+= / Ctrl+- / Ctrl+0)",
        kind: Kind::Number { min: 0.5, max: 3.0, step: ZOOM_STEP as f64, scale: 100.0, unit: "%" },
        project: false,
    },
];

pub fn item(key: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_stay_in_range_without_drift() {
        let mut settings = Settings::default();
        for _ in 0..3 {
            settings.zoom(1);
        }
        assert_eq!(settings.ui_scale, 1.3);
        settings.zoom(-3);
        assert_eq!(settings.ui_scale, 1.0);
        settings.zoom(-100);
        assert_eq!(settings.ui_scale, 0.5);
        settings.zoom(100);
        assert_eq!(settings.ui_scale, 3.0);
    }

    #[test]
    fn missing_or_wild_values_fall_back() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, Settings::default());
        let parsed: Settings = serde_json::from_str(r#"{"ui_scale": 40, "future_setting": true}"#).unwrap();
        assert_eq!(parsed.sanitized().ui_scale, 3.0);
    }

    #[test]
    fn every_item_is_a_setting() {
        let settings = Settings::default();
        for item in ITEMS {
            let value = settings.value(item.key).unwrap_or_else(|| panic!("{} is not a setting", item.key));
            match item.kind {
                Kind::Toggle => assert!(value.is_boolean(), "{}", item.key),
                Kind::Number { min, max, .. } => {
                    let n = value.as_f64().unwrap();
                    assert!((min..=max).contains(&n), "{}", item.key);
                }
            }
        }
    }

    #[test]
    fn values_are_set_by_name_and_checked() {
        let settings = Settings::default();
        let changed = settings.with("cursor_blink", Value::Bool(false)).unwrap();
        assert!(!changed.cursor_blink);
        assert!(settings.with("cursor_blink", Value::from(3)).is_none());
        assert!(settings.with("no_such_setting", Value::Bool(true)).is_none());
        assert_eq!(settings.with("hover_delay_ms", Value::from(99_999)).unwrap().hover_delay_ms, 3000);
    }

    #[test]
    fn projects_override_only_what_they_may() {
        let user = Settings { format_on_save: true, ..Settings::default() };
        let overrides: Map<String, Value> = serde_json::from_str(
            r#"{"format_on_save": false, "auto_close_brackets": "yes", "ui_scale": 2.0, "unknown": 1}"#,
        )
        .unwrap();
        let effective = user.with_project(&overrides);
        assert!(!effective.format_on_save);
        // A bad value and a user-only setting are ignored.
        assert!(effective.auto_close_brackets);
        assert_eq!(effective.ui_scale, 1.0);
    }

    #[test]
    fn project_files_come_and_go() {
        let root = std::env::temp_dir().join(format!("beckett-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut overrides = Map::new();
        overrides.insert("format_on_save".into(), Value::Bool(true));
        save_project(&root, &overrides).unwrap();
        assert_eq!(load_project(&root), overrides);
        save_project(&root, &Map::new()).unwrap();
        assert!(!root.join(".beckett").exists());
        assert!(load_project(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
