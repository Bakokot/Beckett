//! The `[input]` (Input Map) and `[autoload]` sections of `project.godot`.

use crate::settings::ProjectSettings;

/// An action of the Input Map, as saved in the project.
#[derive(Debug, Clone, PartialEq)]
pub struct InputAction {
    pub name: String,
    pub deadzone: Option<f32>,
    /// Readable events, e.g. "Space", "Ctrl+S", "Joypad A", "Left Stick Left".
    pub events: Vec<String>,
    /// Zero-based line in `project.godot`.
    pub line: usize,
}

/// A script or scene loaded at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autoload {
    pub name: String,
    /// `res://` path of the script or scene.
    pub path: String,
    /// Reachable by its name from every script (the `*` prefix Godot writes).
    pub global: bool,
    /// Zero-based line in `project.godot`.
    pub line: usize,
}

/// Actions every project has (`ui_*`), unless it redefines them; the ones
/// scripts use, not the text-editing ones.
pub const BUILTIN_ACTIONS: &[&str] = &[
    "ui_accept",
    "ui_select",
    "ui_cancel",
    "ui_focus_next",
    "ui_focus_prev",
    "ui_left",
    "ui_right",
    "ui_up",
    "ui_down",
    "ui_page_up",
    "ui_page_down",
    "ui_home",
    "ui_end",
    "ui_cut",
    "ui_copy",
    "ui_paste",
    "ui_undo",
    "ui_redo",
    "ui_menu",
    "ui_text_submit",
];

pub fn input_actions(settings: &ProjectSettings) -> Vec<InputAction> {
    settings
        .section("input")
        .into_iter()
        .map(|entry| InputAction {
            name: entry.key.to_string(),
            deadzone: field(entry.raw, "deadzone").and_then(|v| v.parse().ok()),
            events: objects(entry.raw).iter().filter_map(|o| describe_event(o)).collect(),
            line: entry.line,
        })
        .collect()
}

pub fn autoloads(settings: &ProjectSettings) -> Vec<Autoload> {
    settings
        .section("autoload")
        .into_iter()
        .filter_map(|entry| {
            let value = entry.raw.strip_prefix('"')?.strip_suffix('"')?;
            let (global, path) = match value.strip_prefix('*') {
                Some(path) => (true, path),
                None => (false, value),
            };
            Some(Autoload { name: entry.key.to_string(), path: path.to_string(), global, line: entry.line })
        })
        .collect()
}

/// The inside of every `Object(…)` in a variant, e.g.
/// `InputEventKey,"keycode":32,…`.
fn objects(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("Object(") {
        let body = &rest[start + "Object(".len()..];
        let mut depth = 1;
        let mut in_string = false;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '"' => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(&body[..end]);
        rest = &body[end..];
    }
    out
}

/// The value of `"name": value` in a variant, unparsed (up to the next
/// `,`, `)` or `}`).
fn field<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\"");
    let after = &raw[raw.find(&key)? + key.len()..];
    let value = after.trim_start().strip_prefix(':')?.trim_start();
    let end = value.find([',', ')', '}', '\n']).unwrap_or(value.len());
    Some(value[..end].trim())
}

fn int(raw: &str, name: &str) -> i64 {
    field(raw, name).and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn flag(raw: &str, name: &str) -> bool {
    field(raw, name) == Some("true")
}

/// A readable name for one event, as the Godot editor shows it.
fn describe_event(object: &str) -> Option<String> {
    let class = object.split(',').next()?.trim();
    Some(match class {
        "InputEventKey" => {
            let keycode = int(object, "keycode");
            let physical = int(object, "physical_keycode");
            let (code, suffix) = if keycode != 0 { (keycode, "") } else { (physical, " (Physical)") };
            let mut name = String::new();
            for (modifier, label) in [("ctrl_pressed", "Ctrl+"), ("shift_pressed", "Shift+"), ("alt_pressed", "Alt+"), ("meta_pressed", "Meta+")] {
                if flag(object, modifier) {
                    name.push_str(label);
                }
            }
            name.push_str(&key_name(code));
            name.push_str(suffix);
            name
        }
        "InputEventMouseButton" => {
            let index = int(object, "button_index");
            match usize::try_from(index - 1).ok().and_then(|i| MOUSE_BUTTONS.get(i)) {
                Some(name) => format!("{name} Mouse Button"),
                None => format!("Mouse Button {index}"),
            }
        }
        "InputEventJoypadButton" => {
            let index = int(object, "button_index");
            match usize::try_from(index).ok().and_then(|i| JOY_BUTTONS.get(i)) {
                Some(name) => format!("Joypad {name}"),
                None => format!("Joypad Button {index}"),
            }
        }
        "InputEventJoypadMotion" => {
            let axis = int(object, "axis");
            let negative = field(object, "axis_value").and_then(|v| v.parse::<f32>().ok()).is_some_and(|v| v < 0.0);
            match axis {
                0..=3 => {
                    let stick = if axis < 2 { "Left Stick" } else { "Right Stick" };
                    let direction = match (axis % 2 == 0, negative) {
                        (true, true) => "Left",
                        (true, false) => "Right",
                        (false, true) => "Up",
                        (false, false) => "Down",
                    };
                    format!("{stick} {direction}")
                }
                4 => "Left Trigger".to_string(),
                5 => "Right Trigger".to_string(),
                _ => format!("Joypad Axis {axis}{}", if negative { " -" } else { " +" }),
            }
        }
        other => other.strip_prefix("InputEvent").unwrap_or(other).to_string(),
    })
}

const MOUSE_BUTTONS: &[&str] =
    &["Left", "Right", "Middle", "Wheel Up", "Wheel Down", "Wheel Left", "Wheel Right", "Extra 1", "Extra 2"];

const JOY_BUTTONS: &[&str] = &[
    "A", "B", "X", "Y", "Back", "Guide", "Start", "Left Stick", "Right Stick", "LB", "RB", "D-pad Up", "D-pad Down",
    "D-pad Left", "D-pad Right",
];

/// Godot's `Key` values: printable keys are their (uppercase) character,
/// the others `1 << 22` plus an index.
fn key_name(code: i64) -> String {
    const SPECIAL: i64 = 1 << 22;
    const NAMES: &[&str] = &[
        "", "Escape", "Tab", "Backtab", "Backspace", "Enter", "Kp Enter", "Insert", "Delete", "Pause", "Print",
        "SysReq", "Clear", "Home", "End", "Left", "Up", "Right", "Down", "PageUp", "PageDown", "Shift", "Ctrl",
        "Meta", "Alt", "CapsLock", "NumLock", "ScrollLock",
    ];
    match code {
        0 => "(Unset)".to_string(),
        32 => "Space".to_string(),
        33..=126 => char::from(code as u8).to_ascii_uppercase().to_string(),
        _ if code & SPECIAL != 0 => {
            let index = code & !SPECIAL;
            match index {
                1..=0x1B => NAMES[index as usize].to_string(),
                0x1C..=0x3E => format!("F{}", index - 0x1B),
                0x81 => "Kp Multiply".to_string(),
                0x82 => "Kp Divide".to_string(),
                0x83 => "Kp Subtract".to_string(),
                0x84 => "Kp Period".to_string(),
                0x85 => "Kp Add".to_string(),
                0x86..=0x8F => format!("Kp {}", index - 0x86),
                0x42 => "Menu".to_string(),
                _ => format!("Key {code}"),
            }
        }
        _ => char::from_u32(code as u32).map_or_else(|| format!("Key {code}"), |c| c.to_uppercase().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = r#"config_version=5

[autoload]

Game="*res://autoload/game.gd"
Music="res://audio/music.tscn"

[input]

move_left={
"deadzone": 0.2,
"events": [Object(InputEventKey,"resource_local_to_scene":false,"resource_name":"","device":-1,"window_id":0,"alt_pressed":false,"shift_pressed":false,"ctrl_pressed":false,"meta_pressed":false,"pressed":false,"keycode":0,"physical_keycode":65,"key_label":0,"unicode":97,"location":0,"echo":false,"script":null)
, Object(InputEventKey,"resource_local_to_scene":false,"resource_name":"","device":-1,"window_id":0,"alt_pressed":false,"shift_pressed":false,"ctrl_pressed":false,"meta_pressed":false,"pressed":false,"keycode":4194319,"physical_keycode":0,"key_label":0,"unicode":0,"location":0,"echo":false,"script":null)
, Object(InputEventJoypadMotion,"resource_local_to_scene":false,"resource_name":"","device":-1,"axis":0,"axis_value":-1.0,"script":null)
]
}
save={
"deadzone": 0.5,
"events": [Object(InputEventKey,"resource_local_to_scene":false,"resource_name":"","device":-1,"window_id":0,"alt_pressed":false,"shift_pressed":false,"ctrl_pressed":true,"meta_pressed":false,"pressed":false,"keycode":83,"physical_keycode":0,"key_label":0,"unicode":0,"location":0,"echo":false,"script":null)
, Object(InputEventJoypadButton,"resource_local_to_scene":false,"resource_name":"","device":-1,"button_index":6,"pressure":0.0,"pressed":true,"script":null)
, Object(InputEventMouseButton,"resource_local_to_scene":false,"resource_name":"","device":-1,"window_id":0,"alt_pressed":false,"shift_pressed":false,"ctrl_pressed":false,"meta_pressed":false,"button_mask":0,"position":Vector2(0, 0),"global_position":Vector2(0, 0),"factor":1.0,"button_index":2,"canceled":false,"pressed":true,"double_click":false,"script":null)
, Object(InputEventKey,"resource_local_to_scene":false,"resource_name":"","device":-1,"window_id":0,"alt_pressed":false,"shift_pressed":false,"ctrl_pressed":false,"meta_pressed":false,"pressed":false,"keycode":4194332,"physical_keycode":0,"key_label":0,"unicode":0,"location":0,"echo":false,"script":null)
]
}
"open menu"={
"deadzone": 0.5,
"events": []
}
"#;

    #[test]
    fn reads_autoloads_in_order() {
        let settings = ProjectSettings::parse(PROJECT);
        let loads = autoloads(&settings);
        assert_eq!(loads.len(), 2);
        assert_eq!(loads[0], Autoload { name: "Game".into(), path: "res://autoload/game.gd".into(), global: true, line: 4 });
        assert_eq!((loads[1].name.as_str(), loads[1].global), ("Music", false));
    }

    #[test]
    fn describes_input_events() {
        let settings = ProjectSettings::parse(PROJECT);
        let actions = input_actions(&settings);
        let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["move_left", "save", "open menu"]);
        assert_eq!(actions[0].deadzone, Some(0.2));
        assert_eq!(actions[0].line, 9);
        assert_eq!(actions[0].events, ["A (Physical)", "Left", "Left Stick Left"]);
        assert_eq!(actions[1].events, ["Ctrl+S", "Joypad Start", "Right Mouse Button", "F1"]);
        assert!(actions[2].events.is_empty());
    }
}
