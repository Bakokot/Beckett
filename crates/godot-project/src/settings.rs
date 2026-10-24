//! Parser for Godot's `project.godot` (ConfigFile) format.
//!
//! Values are kept as raw Godot variant text; accessors decode the common
//! cases. Multi-line values (dictionaries, arrays) are joined until their
//! brackets balance.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSettings {
    values: BTreeMap<String, String>,
}

impl ProjectSettings {
    pub fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut section = String::new();
        let mut lines = text.lines();

        while let Some(line) = lines.next() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let mut value = value.trim().to_string();
            while bracket_depth(&value) > 0 {
                let Some(next) = lines.next() else { break };
                value.push('\n');
                value.push_str(next);
            }
            let key = key.trim();
            let key = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}/{key}")
            };
            values.insert(key, value);
        }

        Self { values }
    }

    /// Raw variant text for a `section/key` setting.
    pub fn get_raw(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// A string setting, unquoted and unescaped.
    pub fn get_string(&self, key: &str) -> Option<String> {
        let raw = self.get_raw(key)?;
        let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
        Some(unescape(inner))
    }

    /// Every quoted string in a setting, e.g. a `PackedStringArray(...)`.
    pub fn get_string_array(&self, key: &str) -> Vec<String> {
        self.get_raw(key).map(quoted_strings).unwrap_or_default()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }
}

fn bracket_depth(value: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for c in value.chars() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn quoted_strings(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        let mut escaped = false;
        for c in chars.by_ref() {
            match c {
                _ if escaped => {
                    s.push('\\');
                    s.push(c);
                    escaped = false;
                }
                '\\' => escaped = true,
                '"' => break,
                _ => s.push(c),
            }
        }
        out.push(unescape(&s));
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"; Engine configuration file.

config_version=5

[application]

config/name="My \"Cool\" Game"
run/main_scene="res://scenes/main.tscn"
config/features=PackedStringArray("4.3", "Forward Plus")

[input]

jump={
"deadzone": 0.5,
"events": [Object(InputEventKey,"keycode":32)]
}

[rendering]

textures/canvas_textures/default_texture_filter=0
"#;

    #[test]
    fn parses_sections_and_strings() {
        let s = ProjectSettings::parse(SAMPLE);
        assert_eq!(s.get_raw("config_version"), Some("5"));
        assert_eq!(
            s.get_string("application/config/name").as_deref(),
            Some(r#"My "Cool" Game"#)
        );
        assert_eq!(
            s.get_string_array("application/config/features"),
            vec!["4.3", "Forward Plus"]
        );
        assert_eq!(
            s.get_raw("rendering/textures/canvas_textures/default_texture_filter"),
            Some("0")
        );
    }

    #[test]
    fn joins_multiline_values() {
        let s = ProjectSettings::parse(SAMPLE);
        let jump = s.get_raw("input/jump").unwrap();
        assert!(jump.starts_with('{') && jump.ends_with('}'));
        assert!(jump.contains("deadzone"));
        assert!(s.keys().all(|k| !k.starts_with('"')));
    }
}
