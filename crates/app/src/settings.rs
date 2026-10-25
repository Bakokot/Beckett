//! Personal settings that are not part of a theme, kept in
//! `<config>/godot-ide/settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const ZOOM_STEP: f32 = 0.1;
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Scale of the whole interface (Ctrl+= / Ctrl+- / Ctrl+0).
    pub ui_scale: f32,
    /// Name of the theme in use, restored at startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { ui_scale: 1.0, theme: None }
    }
}

impl Settings {
    /// The saved settings; defaults when there are none or they can't be read.
    pub fn load() -> Self {
        let text = file().and_then(|path| std::fs::read_to_string(path).ok());
        let settings: Self = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
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
