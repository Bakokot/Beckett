//! Personal settings that are not part of a theme, kept in
//! `<config>/beckett/settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
        self
    }
}

fn file() -> Option<PathBuf> {
    Some(theme::config_dir()?.join("settings.json"))
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
}
