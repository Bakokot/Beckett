//! Importing VS Code color themes (`.json`, often with comments): their
//! `colors` give the UI and terminal colors, their TextMate `tokenColors`
//! the syntax colors. What a theme doesn't set comes from Godot Dark or
//! Godot Light, after its `type`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::{builtin, Color, Theme, ThemeKind};

/// A VS Code theme found in VS Code's extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// As VS Code lists it, e.g. "Dark+" or "Monokai".
    pub label: String,
    pub path: PathBuf,
}

/// Reads a VS Code theme file (and the themes it `include`s).
pub fn import(path: &Path) -> Result<Theme, String> {
    let fallback_name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let json = load(path, 0)?;
    Ok(convert(&json, &fallback_name))
}

/// The theme JSON with its `include` chain merged (the includer wins).
fn load(path: &Path, depth: usize) -> Result<Map<String, Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&strip_jsonc(&text)).map_err(|e| format!("{}: {e}", path.display()))?;
    let Value::Object(mut json) = value else {
        return Err(format!("{} is not a VS Code theme", path.display()));
    };
    let include = json.get("include").and_then(Value::as_str).map(str::to_string);
    let (Some(include), true) = (include, depth < 8) else {
        return Ok(json);
    };
    let Ok(mut base) = load(&path.with_file_name("").join(include), depth + 1) else {
        return Ok(json);
    };
    // Colors merge key by key; token rules of the base come first, so the
    // includer's override them.
    if let (Some(Value::Object(colors)), Some(Value::Object(base_colors))) = (json.remove("colors"), base.get_mut("colors")) {
        base_colors.extend(colors);
    }
    if let (Some(Value::Array(rules)), Some(Value::Array(base_rules))) = (json.remove("tokenColors"), base.get_mut("tokenColors")) {
        base_rules.extend(rules);
    }
    base.extend(json);
    Ok(base)
}

/// A VS Code theme as one of ours.
pub fn convert(json: &Map<String, Value>, fallback_name: &str) -> Theme {
    // Without a `type` (VS Code then reads it from the extension), the
    // editor background tells.
    let background = json.get("colors").and_then(|c| c.get("editor.background")).and_then(Value::as_str).and_then(parse_color);
    let kind = match (json.get("type").and_then(Value::as_str), background) {
        (Some(t), _) if t.contains("light") => ThemeKind::Light,
        (Some(_), _) => ThemeKind::Dark,
        (None, Some(bg)) if luminance(bg) > 0.5 => ThemeKind::Light,
        (None, _) => ThemeKind::Dark,
    };
    let mut theme = builtin().into_iter().find(|t| t.kind == kind).expect("built-in themes cover both kinds");
    theme.name = json.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(fallback_name).to_string();
    theme.author = Some("Imported from VS Code".to_string());

    let empty = Map::new();
    let colors = json.get("colors").and_then(Value::as_object).unwrap_or(&empty);
    let color = |keys: &[&str]| keys.iter().find_map(|k| colors.get(*k).and_then(Value::as_str).and_then(parse_color));
    let set = |slot: &mut Color, keys: &[&str]| {
        if let Some(c) = color(keys) {
            *slot = c;
        }
    };
    let ui = &mut theme.ui;
    set(&mut ui.background, &["editor.background"]);
    set(&mut ui.surface, &["sideBar.background", "activityBar.background", "editorGroupHeader.tabsBackground", "statusBar.background"]);
    set(&mut ui.text, &["foreground", "editor.foreground", "sideBar.foreground"]);
    set(&mut ui.text_muted, &["descriptionForeground", "tab.inactiveForeground", "editorLineNumber.foreground"]);
    set(&mut ui.accent, &["focusBorder", "button.background", "textLink.foreground", "activityBarBadge.background"]);
    set(&mut ui.border, &["panel.border", "sideBar.border", "editorGroup.border", "contrastBorder"]);
    set(&mut ui.success, &["gitDecoration.addedResourceForeground", "terminal.ansiGreen"]);
    set(&mut ui.warning, &["editorWarning.foreground", "list.warningForeground", "terminal.ansiYellow"]);
    set(&mut ui.danger, &["editorError.foreground", "errorForeground", "terminal.ansiRed"]);

    let term = &mut theme.terminal;
    let ansi: [(&mut Color, &str); 16] = [
        (&mut term.black, "terminal.ansiBlack"),
        (&mut term.red, "terminal.ansiRed"),
        (&mut term.green, "terminal.ansiGreen"),
        (&mut term.yellow, "terminal.ansiYellow"),
        (&mut term.blue, "terminal.ansiBlue"),
        (&mut term.magenta, "terminal.ansiMagenta"),
        (&mut term.cyan, "terminal.ansiCyan"),
        (&mut term.white, "terminal.ansiWhite"),
        (&mut term.bright_black, "terminal.ansiBrightBlack"),
        (&mut term.bright_red, "terminal.ansiBrightRed"),
        (&mut term.bright_green, "terminal.ansiBrightGreen"),
        (&mut term.bright_yellow, "terminal.ansiBrightYellow"),
        (&mut term.bright_blue, "terminal.ansiBrightBlue"),
        (&mut term.bright_magenta, "terminal.ansiBrightMagenta"),
        (&mut term.bright_cyan, "terminal.ansiBrightCyan"),
        (&mut term.bright_white, "terminal.ansiBrightWhite"),
    ];
    for (slot, key) in ansi {
        set(slot, &[key]);
    }

    let rules = token_rules(json.get("tokenColors"));
    let syntax = &mut theme.syntax;
    set(&mut syntax.text, &["editor.foreground", "foreground"]);
    let pick = |scopes: &[&str]| scopes.iter().find_map(|scope| best_rule(&rules, scope));
    let slots: [(&mut Color, &[&str]); 12] = [
        (&mut syntax.keyword, &["keyword", "storage.type", "storage"]),
        (&mut syntax.control_flow, &["keyword.control", "keyword"]),
        (&mut syntax.r#type, &["entity.name.type", "entity.name.class", "support.class", "support.type", "storage.type"]),
        (&mut syntax.function, &["entity.name.function", "support.function"]),
        (&mut syntax.annotation, &["meta.decorator", "entity.name.function.decorator", "storage.type.annotation", "keyword"]),
        (&mut syntax.string, &["string.quoted", "string"]),
        (&mut syntax.string_name, &["constant.other.symbol", "string.other", "string"]),
        (&mut syntax.node_path, &["string.other.link", "markup.underline.link", "string.regexp", "string"]),
        (&mut syntax.number, &["constant.numeric", "constant"]),
        (&mut syntax.comment, &["comment"]),
        (&mut syntax.constant, &["constant.language", "variable.other.constant", "constant"]),
        (&mut syntax.symbol, &["keyword.operator", "punctuation"]),
    ];
    for (slot, scopes) in slots {
        if let Some(c) = pick(scopes) {
            *slot = c;
        }
    }
    theme
}

/// `(selector, color)` of every TextMate rule with a foreground.
fn token_rules(value: Option<&Value>) -> Vec<(String, Color)> {
    let mut rules = Vec::new();
    for rule in value.and_then(Value::as_array).into_iter().flatten() {
        let Some(color) = rule["settings"]["foreground"].as_str().and_then(parse_color) else {
            continue;
        };
        let selectors: Vec<String> = match &rule["scope"] {
            Value::String(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            Value::Array(list) => list.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect(),
            _ => continue,
        };
        // Descendant selectors ("source.gd keyword") are too narrow to use.
        rules.extend(selectors.into_iter().filter(|s| !s.is_empty() && !s.contains(' ')).map(|s| (s, color)));
    }
    rules
}

/// The color VS Code would give `scope`: the rule with the longest selector
/// that is `scope` or a prefix of it at a dot; the later on a tie.
fn best_rule(rules: &[(String, Color)], scope: &str) -> Option<Color> {
    rules
        .iter()
        .enumerate()
        .filter(|(_, (selector, _))| scope == selector || scope.strip_prefix(selector.as_str()).is_some_and(|rest| rest.starts_with('.')))
        .max_by_key(|(index, (selector, _))| (selector.len(), *index))
        .map(|(_, (_, color))| *color)
}

/// Perceived brightness, 0 (black) to 1 (white).
fn luminance(c: Color) -> f32 {
    (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
fn parse_color(text: &str) -> Option<Color> {
    let digits = text.trim().strip_prefix('#')?;
    let long = match digits.len() {
        3 | 4 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => digits.to_string(),
        _ => return None,
    };
    Color::from_hex(&format!("#{long}")).ok()
}

/// JSON with comments and trailing commas (as VS Code writes it) made
/// plain JSON.
fn strip_jsonc(text: &str) -> String {
    // Comments first, so a trailing comma before one is seen as trailing.
    let without_comments = outside_strings(text.trim_start_matches('\u{feff}'), |chars, i, out| match (chars[i], chars.get(i + 1)) {
        ('/', Some('/')) => chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |n| i + n),
        ('/', Some('*')) => (i + 2..chars.len().saturating_sub(1)).find(|&j| chars[j] == '*' && chars[j + 1] == '/').map_or(chars.len(), |j| j + 2),
        (c, _) => {
            out.push(c);
            i + 1
        }
    });
    outside_strings(&without_comments, |chars, i, out| {
        // A comma whose next non-space char closes the object or array.
        let trailing = chars[i] == ',' && matches!(chars[i + 1..].iter().find(|c| !c.is_whitespace()), Some('}' | ']'));
        if !trailing {
            out.push(chars[i]);
        }
        i + 1
    })
}

/// Copies `text`, string literals as they are and everything else through
/// `step`, which handles the char at an index and returns the next index.
fn outside_strings(text: &str, mut step: impl FnMut(&[char], usize, &mut String) -> usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '"' {
            i = step(&chars, i, &mut out);
            continue;
        }
        // A string, up to its closing quote (escapes skipped).
        out.push('"');
        i += 1;
        while i < chars.len() {
            out.push(chars[i]);
            match chars[i] {
                '\\' if i + 1 < chars.len() => {
                    out.push(chars[i + 1]);
                    i += 2;
                }
                '"' => {
                    i += 1;
                    break;
                }
                _ => i += 1,
            }
        }
    }
    out
}

/// The color themes of the VS Code installed here: its built-in themes
/// and those of its extensions, by the label VS Code shows.
pub fn installed() -> Vec<Installed> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".vscode").join("extensions"));
        roots.push(home.join(".vscode-oss").join("extensions"));
    }
    if let Some(local) = dirs::data_local_dir() {
        roots.push(local.join("Programs").join("Microsoft VS Code").join("resources").join("app").join("extensions"));
    }
    for program_files in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(program_files) {
            roots.push(PathBuf::from(dir).join("Microsoft VS Code").join("resources").join("app").join("extensions"));
        }
    }
    roots.push(PathBuf::from("/usr/share/code/resources/app/extensions"));
    roots.push(PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/extensions"));

    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for extension in entries.filter_map(Result::ok).map(|e| e.path()) {
            found.extend(themes_of_extension(&extension));
        }
    }
    found.sort_by(|a: &Installed, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    found.dedup_by(|a, b| a.label == b.label);
    found
}

/// The themes an extension contributes (`contributes.themes` of its
/// package.json).
fn themes_of_extension(dir: &Path) -> Vec<Installed> {
    let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&strip_jsonc(&text)) else {
        return Vec::new();
    };
    json["contributes"]["themes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|theme| {
            let path = dir.join(theme["path"].as_str()?);
            let label = theme["label"].as_str().or(theme["id"].as_str())?.to_string();
            // Built-in labels are placeholders ("%colorTheme.dark%") resolved
            // from package.nls.json.
            let label = match label.strip_prefix('%').and_then(|l| l.strip_suffix('%')) {
                Some(key) => nls(dir, key).unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
                None => label,
            };
            path.is_file().then_some(Installed { label, path })
        })
        .collect()
}

fn nls(dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.nls.json")).ok()?;
    let json: Value = serde_json::from_str(&strip_jsonc(&text)).ok()?;
    json[key].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r##"{
        // A comment, and trailing commas below.
        "name": "Test Night",
        "type": "dark",
        "colors": {
            "editor.background": "#1e1e2e",
            "sideBar.background": "#181825",
            "focusBorder": "#89b4fa",
            "terminal.ansiRed": "#f38ba8",
        },
        "tokenColors": [
            { "scope": "comment", "settings": { "foreground": "#6c7086", "fontStyle": "italic" } },
            { "scope": ["keyword", "storage.type"], "settings": { "foreground": "#cba6f7" } },
            { "scope": "keyword.control", "settings": { "foreground": "#f5c2e7" } },
            { "scope": "string, constant.other.symbol", "settings": { "foreground": "#a6e3a1" } },
            { "scope": "source.gdscript keyword", "settings": { "foreground": "#ffffff" } },
            /* a block comment */
            { "scope": "constant.numeric", "settings": { "foreground": "#fab" } },
            { "settings": { "background": "#000000" } },
        ],
    }"##;

    fn parse(text: &str) -> Theme {
        let Value::Object(json) = serde_json::from_str(&strip_jsonc(text)).unwrap() else { panic!() };
        convert(&json, "fallback")
    }

    #[test]
    fn converts_ui_terminal_and_syntax_colors() {
        let theme = parse(THEME);
        assert_eq!(theme.name, "Test Night");
        assert_eq!(theme.kind, ThemeKind::Dark);
        assert_eq!(theme.ui.background, Color::from_hex("#1e1e2e").unwrap());
        assert_eq!(theme.ui.surface, Color::from_hex("#181825").unwrap());
        assert_eq!(theme.ui.accent, Color::from_hex("#89b4fa").unwrap());
        assert_eq!(theme.terminal.red, Color::from_hex("#f38ba8").unwrap());
        assert_eq!(theme.syntax.comment, Color::from_hex("#6c7086").unwrap());
        // The more specific scope wins; descendant selectors are ignored.
        assert_eq!(theme.syntax.keyword, Color::from_hex("#cba6f7").unwrap());
        assert_eq!(theme.syntax.control_flow, Color::from_hex("#f5c2e7").unwrap());
        assert_eq!(theme.syntax.string_name, Color::from_hex("#a6e3a1").unwrap());
        // Short hex colors.
        assert_eq!(theme.syntax.number, Color::from_hex("#ffaabb").unwrap());
        // Unset: from Godot Dark.
        let dark = builtin().into_iter().find(|t| t.kind == ThemeKind::Dark).unwrap();
        assert_eq!(theme.syntax.function, dark.syntax.function);
        // It saves and loads as one of ours.
        assert_eq!(Theme::from_toml(&theme.to_toml().unwrap()).unwrap(), theme);
    }

    #[test]
    fn includes_are_merged_and_light_themes_start_from_godot_light() {
        let dir = std::env::temp_dir().join(format!("beckett-vscode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.json"),
            r##"{ "colors": { "editor.background": "#ffffff", "focusBorder": "#0000ff" },
                  "tokenColors": [{ "scope": "comment", "settings": { "foreground": "#008000" } },
                                  { "scope": "string", "settings": { "foreground": "#a31515" } }] }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("light.json"),
            r##"{ "name": "Light+", "type": "light", "include": "./base.json",
                  "colors": { "focusBorder": "#ff00ff" },
                  "tokenColors": [{ "scope": "comment", "settings": { "foreground": "#999999" } }] }"##,
        )
        .unwrap();
        let theme = import(&dir.join("light.json")).unwrap();
        assert_eq!(theme.kind, ThemeKind::Light);
        assert_eq!(theme.ui.background, Color::from_hex("#ffffff").unwrap());
        assert_eq!(theme.ui.accent, Color::from_hex("#ff00ff").unwrap());
        assert_eq!(theme.syntax.comment, Color::from_hex("#999999").unwrap());
        assert_eq!(theme.syntax.string, Color::from_hex("#a31515").unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_type_the_background_decides_the_kind() {
        assert_eq!(parse(r##"{ "colors": { "editor.background": "#fdfdfd" } }"##).kind, ThemeKind::Light);
        assert_eq!(parse(r##"{ "colors": { "editor.background": "#202020" } }"##).kind, ThemeKind::Dark);
        assert_eq!(parse("{}").name, "fallback");
    }

    #[test]
    fn jsonc_keeps_strings_intact() {
        let plain = strip_jsonc(r#"{"a": "x // not /* a */ comment \" ,]", "b": [1, 2,], /* c */}"#);
        assert_eq!(plain, r#"{"a": "x // not /* a */ comment \" ,]", "b": [1, 2] }"#);
        assert!(serde_json::from_str::<Value>(&plain).is_ok());
    }
}
