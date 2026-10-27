//! Importing the Godot editor's own colors: the text editor highlighting
//! and interface colors of `editor_settings-4.x.tres` (one file per Godot
//! version, in Godot's config folder). Read only.

use std::path::{Path, PathBuf};

use crate::{builtin, Color, Theme, ThemeKind};

/// Godot's editor settings files here, newest Godot version first.
pub fn settings_files() -> Vec<PathBuf> {
    // ~/.config/godot on Linux; %APPDATA%\Godot on Windows, Application
    // Support/Godot on macOS (one folder: those file systems ignore case).
    let folder = if cfg!(target_os = "linux") { "godot" } else { "Godot" };
    let Some(dir) = dirs::config_dir().map(|config| config.join(folder)) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| version_of(path).is_some())
        .collect();
    files.sort_by_key(|path| std::cmp::Reverse(version_of(path)));
    files.dedup();
    files
}

/// The Godot version of an `editor_settings-4.7.tres` file: (4, 7).
pub fn version_of(path: &Path) -> Option<(u32, u32)> {
    let name = path.file_name()?.to_str()?;
    let version = name.strip_prefix("editor_settings-")?.strip_suffix(".tres")?;
    let (major, minor) = version.split_once('.').unwrap_or((version, "0"));
    Some((major.parse().ok()?, minor.parse().ok()?))
}

pub fn import(path: &Path) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = match version_of(path) {
        Some((major, minor)) => format!("Godot {major}.{minor} Editor"),
        None => "Godot Editor".to_string(),
    };
    Ok(convert(&text, &name))
}

/// The editor settings' colors as a theme; what they don't set comes
/// from Godot Dark or Godot Light, after the background.
pub fn convert(text: &str, name: &str) -> Theme {
    let get = |key: &str| -> Option<Color> {
        text.lines().find_map(|line| {
            let (k, value) = line.split_once('=')?;
            (k.trim() == key).then(|| parse_color(value.trim())).flatten()
        })
    };
    let highlighting = |key: &str| get(&format!("text_editor/theme/highlighting/{key}"));

    let background = highlighting("background_color");
    let light = background.is_some_and(|c| (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0 > 0.5);
    let kind = if light { ThemeKind::Light } else { ThemeKind::Dark };
    let mut theme = builtin().into_iter().find(|t| t.kind == kind).expect("built-in themes cover both kinds");
    theme.name = name.to_string();
    theme.author = Some("Imported from the Godot editor".to_string());

    let set = |slot: &mut Color, color: Option<Color>| {
        if let Some(color) = color {
            *slot = color;
        }
    };
    let ui = &mut theme.ui;
    set(&mut ui.background, background);
    set(&mut ui.surface, get("interface/theme/base_color"));
    set(&mut ui.text, highlighting("text_color").map(opaque));
    set(&mut ui.text_muted, highlighting("line_number_color"));
    set(&mut ui.accent, get("interface/theme/accent_color"));
    set(&mut ui.danger, highlighting("brace_mismatch_color"));

    let syntax = &mut theme.syntax;
    set(&mut syntax.text, highlighting("text_color"));
    set(&mut syntax.keyword, highlighting("keyword_color"));
    set(&mut syntax.control_flow, highlighting("control_flow_keyword_color"));
    set(&mut syntax.r#type, highlighting("engine_type_color").or_else(|| highlighting("base_type_color")));
    set(&mut syntax.function, highlighting("function_color"));
    set(&mut syntax.annotation, highlighting("gdscript/annotation_color"));
    set(&mut syntax.string, highlighting("string_color"));
    set(&mut syntax.string_name, highlighting("gdscript/string_name_color"));
    set(&mut syntax.node_path, highlighting("gdscript/node_path_color"));
    set(&mut syntax.number, highlighting("number_color"));
    set(&mut syntax.comment, highlighting("comment_color"));
    set(&mut syntax.symbol, highlighting("symbol_color"));
    theme
}

/// UI text drawn at full opacity (the editor dims code text with alpha).
fn opaque(color: Color) -> Color {
    Color { a: 255, ..color }
}

/// `Color(r, g, b)` or `Color(r, g, b, a)`, components 0 to 1 (Godot
/// allows overbright values; they are clamped).
fn parse_color(value: &str) -> Option<Color> {
    let inner = value.strip_prefix("Color(")?.strip_suffix(')')?;
    let parts: Vec<f32> = inner.split(',').map(|p| p.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    match parts[..] {
        [r, g, b] => Some(Color { r: byte(r), g: byte(g), b: byte(b), a: 255 }),
        [r, g, b, a] => Some(Color { r: byte(r), g: byte(g), b: byte(b), a: byte(a) }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = r#"[gd_resource type="EditorSettings" format=3]

[resource]
interface/theme/base_color = Color(0.1, 0.1, 0.12, 1)
interface/theme/accent_color = Color(0.45, 0.75, 1, 1)
text_editor/theme/highlighting/background_color = Color(0.11, 0.13, 0.16, 1)
text_editor/theme/highlighting/text_color = Color(1, 1, 1, 0.75)
text_editor/theme/highlighting/keyword_color = Color(1, 0.44, 0.52, 1)
text_editor/theme/highlighting/safe_line_number_color = Color(1, 1.2, 1, 0.75)
text_editor/theme/highlighting/gdscript/node_path_color = Color(0.72, 0.77, 0.49)
"#;

    #[test]
    fn reads_godot_colors() {
        let theme = convert(SETTINGS, "Godot 4.7 Editor");
        assert_eq!(theme.kind, ThemeKind::Dark);
        assert_eq!(theme.ui.background, Color { r: 28, g: 33, b: 41, a: 255 });
        assert_eq!(theme.ui.accent, Color { r: 115, g: 191, b: 255, a: 255 });
        // Code text keeps Godot's dimming; UI text is opaque.
        assert_eq!(theme.syntax.text.a, 191);
        assert_eq!(theme.ui.text.a, 255);
        assert_eq!(theme.syntax.keyword, Color { r: 255, g: 112, b: 133, a: 255 });
        assert_eq!(theme.syntax.node_path, Color { r: 184, g: 196, b: 125, a: 255 });
        assert_eq!(parse_color("Color(1, 1.2, 1, 0.75)"), Some(Color { r: 255, g: 255, b: 255, a: 191 }));
        assert_eq!(parse_color("Color(1, 1)"), None);
        assert_eq!(Theme::from_toml(&theme.to_toml().unwrap()).unwrap(), theme);
    }

    #[test]
    fn versions_come_from_the_file_name() {
        assert_eq!(version_of(Path::new("editor_settings-4.7.tres")), Some((4, 7)));
        assert_eq!(version_of(Path::new("editor_settings-4.tres")), Some((4, 0)));
        assert_eq!(version_of(Path::new("projects.cfg")), None);
    }
}
