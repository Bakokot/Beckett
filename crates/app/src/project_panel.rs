//! The sidebar's PROJECT tab: main scene, autoloads and the Input Map read
//! from `project.godot`, and completion of action names in scripts.

use std::path::{Path, PathBuf};

use editor_core::Buffer;
use godot_project::{Autoload, GodotProject, InputAction, BUILTIN_ACTIONS, PROJECT_FILE};
use iced::widget::{button, column, container, row, scrollable, space, text, tooltip, Column};
use iced::{Alignment, Element, Font, Length};

use crate::paths::string_at;
use crate::{style, Message};

/// What the panel shows, read once per change of `project.godot`.
#[derive(Default)]
pub struct ProjectInfo {
    pub main_scene: Option<Target>,
    pub autoloads: Vec<(Autoload, Target)>,
    pub actions: Vec<InputAction>,
}

/// A `res://` or `uid://` path of `project.godot`, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The `res://` path when the file was found, else the path as written.
    pub label: String,
    pub file: Option<PathBuf>,
}

impl ProjectInfo {
    pub fn read(godot: Option<&GodotProject>) -> Self {
        let Some(godot) = godot else {
            return Self::default();
        };
        let main_scene = godot.main_scene();
        let autoloads = godot.autoloads();
        let raw: Vec<&str> = main_scene.iter().map(String::as_str).chain(autoloads.iter().map(|a| a.path.as_str())).collect();
        let mut targets = resolve(godot, &raw).into_iter();
        Self {
            main_scene: main_scene.and_then(|_| targets.next()),
            autoloads: autoloads.into_iter().zip(targets).collect(),
            actions: godot.input_actions(),
        }
    }

    /// Every action name scripts can use: the project's, then the built-in
    /// `ui_*` ones it doesn't redefine. With a short description each.
    pub fn action_names(&self) -> Vec<(String, String)> {
        let project = self.actions.iter().map(|a| {
            let events = if a.events.is_empty() { "No events".to_string() } else { a.events.join(", ") };
            (a.name.clone(), events)
        });
        let builtin = BUILTIN_ACTIONS
            .iter()
            .filter(|name| !self.actions.iter().any(|a| a.name == **name))
            .map(|name| (name.to_string(), "Built-in action".to_string()));
        project.chain(builtin).collect()
    }
}

/// Finds the files of `res://` and `uid://` paths; the uids in one pass.
fn resolve(godot: &GodotProject, raw: &[&str]) -> Vec<Target> {
    let uids: Vec<&str> = raw.iter().copied().filter(|p| p.starts_with("uid://")).collect();
    let mut by_uid = godot.find_uids(&uids).into_iter();
    raw.iter()
        .map(|path| {
            let file = if path.starts_with("uid://") {
                by_uid.next().flatten()
            } else {
                godot.resolve(path).filter(|f| f.exists())
            };
            let label = file.as_deref().and_then(|f| godot.to_res_path(f)).unwrap_or_else(|| path.to_string());
            Target { label, file }
        })
        .collect()
}

/// Calls whose string arguments are action names.
const ACTION_CALLS: &[&str] = &[
    "is_action_pressed",
    "is_action_just_pressed",
    "is_action_just_released",
    "is_action_released",
    "is_action",
    "get_action_strength",
    "get_action_raw_strength",
    "get_axis",
    "get_vector",
    "action_press",
    "action_release",
    "has_action",
    "erase_action",
    "action_add_event",
    "action_erase_event",
    "action_erase_events",
    "action_get_events",
    "action_get_deadzone",
    "action_set_deadzone",
    "action_has_event",
    "event_is_action",
];

/// Where action-name completion starts for a cursor at `head`: inside a
/// string passed to `Input.is_action_pressed(` and the like.
pub fn action_completion_anchor(buffer: &Buffer, head: usize) -> Option<usize> {
    let string = string_at(buffer, head)?;
    let call = open_call(&string.before)?;
    ACTION_CALLS.contains(&call).then_some(string.content_start)
}

/// The name of the innermost call left open in `code`, e.g. `get_axis` in
/// `x = Input.get_axis("left", `.
fn open_call(code: &str) -> Option<&str> {
    // Byte offsets of the `(` still open; brackets in strings don't count.
    let mut open = Vec::new();
    let mut in_string: Option<char> = None;
    for (i, c) in code.char_indices() {
        match in_string {
            Some(quote) if c == quote => in_string = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => in_string = Some(c),
                '(' => open.push(i),
                ')' => {
                    open.pop();
                }
                _ => {}
            },
        }
    }
    let before = code[..*open.last()?].trim_end();
    let start = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    let name = &before[start..];
    (!name.is_empty()).then_some(name)
}

pub fn view<'a>(info: &'a ProjectInfo, godot: Option<&'a GodotProject>, theme: &'a theme::Theme) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let Some(godot) = godot else {
        let note = "No project.godot in this folder";
        return container(text(note).size(12).color(muted)).padding([6, 12]).into();
    };
    let project_file = godot.root.join(PROJECT_FILE);
    let heading = |label: String| container(text(label).size(11).color(muted)).padding(iced::Padding { top: 8.0, ..[0, 8].into() });
    let note = |label: &'a str| container(text(label).size(12).color(muted)).padding([2, 12]);
    let link = |content: Element<'a, Message>, message: Message| {
        button(content).on_press(message).width(Length::Fill).padding([2, 12]).style(button::text)
    };
    let mut items: Vec<Element<'a, Message>> = Vec::new();

    items.push(heading("MAIN SCENE".into()).into());
    match &info.main_scene {
        Some(scene) => {
            let label = text(scene.label.as_str()).size(12);
            items.push(match &scene.file {
                Some(path) => link(label.into(), Message::OpenFile(path.clone())).into(),
                None => container(label).padding([2, 12]).into(),
            });
        }
        None => items.push(note("None set").into()),
    }

    items.push(heading(format!("AUTOLOADS ({})", info.autoloads.len())).into());
    if info.autoloads.is_empty() {
        items.push(note("None").into());
    }
    for (autoload, target) in &info.autoloads {
        let name = text(autoload.name.as_str()).size(13).font(Font::MONOSPACE);
        let path = text(target.label.as_str()).size(11).color(muted);
        let content = column![name, path];
        let message = match &target.file {
            Some(path) => Message::OpenFile(path.clone()),
            None => Message::JumpTo(project_file.clone(), autoload.line as u32, 0),
        };
        let description = if autoload.global {
            "Global: reachable by this name from every script"
        } else {
            "Loaded at startup, not a global name"
        };
        items.push(style::tip(link(content.into(), message), description, tooltip::Position::Right, ui));
    }

    items.push(heading(format!("INPUT MAP ({})", info.actions.len())).into());
    if info.actions.is_empty() {
        items.push(note("No actions of the project's own (the built-in ui_* ones are always there)").into());
    }
    for action in &info.actions {
        let events = if action.events.is_empty() { "No events".to_string() } else { action.events.join(" · ") };
        let content = column![
            text(action.name.as_str()).size(13).font(Font::MONOSPACE),
            text(events).size(11).color(muted),
        ];
        let name = link(content.into(), Message::JumpTo(project_file.clone(), action.line as u32, 0));
        let uses = button(text("uses").size(11)).on_press(Message::SearchProject(action.name.clone())).style(button::text).padding([2, 6]);
        let line = row![
            style::tip(name, "Show in project.godot", tooltip::Position::Right, ui),
            style::tip(uses, "Find where scripts use this action", tooltip::Position::Left, ui),
        ]
        .align_y(Alignment::Start)
        .padding(iced::Padding { right: 4.0, ..Default::default() });
        items.push(line.into());
    }
    items.push(space::vertical().height(8).into());

    scrollable(Column::with_children(items).width(Length::Fill)).height(Length::Fill).into()
}

/// The file the panel's data comes from, to reload it when it changes.
pub fn is_project_file(godot: &GodotProject, path: &Path) -> bool {
    lsp_client::uri::same_path(&godot.root.join(PROJECT_FILE), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::Language;

    fn anchor(text: &str) -> Option<usize> {
        let index = text.chars().position(|c| c == '|').unwrap();
        let buffer = Buffer::new(&text.replace('|', ""), Language::GDScript);
        action_completion_anchor(&buffer, index)
    }

    #[test]
    fn completes_actions_only_in_action_arguments() {
        assert_eq!(anchor("\tif Input.is_action_pressed(\"ju|\"):\n"), Some(29));
        assert!(anchor("\tvar d = Input.get_vector(\"left\", \"ri|\", \"up\", \"down\")\n").is_some());
        assert!(anchor("\tif event.is_action_just_pressed('|'):\n").is_some());
        assert!(anchor("\tInput.is_action_pressed(str(\"|\"))\n").is_none());
        assert!(anchor("\tprint(\"ju|\")\n").is_none());
        assert!(anchor("\tInputMap.add_action(\"|\")\n").is_none());
    }

    #[test]
    fn finds_the_innermost_open_call() {
        assert_eq!(open_call("x = Input.get_axis(\"a(\", "), Some("get_axis"));
        assert_eq!(open_call("f(g(1), "), Some("f"));
        assert_eq!(open_call("f(1)"), None);
        assert_eq!(open_call("("), None);
    }

    #[test]
    fn built_in_actions_follow_the_project_ones() {
        let info = ProjectInfo {
            actions: vec![InputAction { name: "ui_accept".into(), deadzone: None, events: vec!["Space".into()], line: 0 }],
            ..Default::default()
        };
        let names = info.action_names();
        assert_eq!(names[0], ("ui_accept".to_string(), "Space".to_string()));
        assert_eq!(names.iter().filter(|(n, _)| n == "ui_accept").count(), 1);
        assert!(names.iter().any(|(n, _)| n == "ui_cancel"));
    }
}
