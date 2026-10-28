//! The INSPECTOR view: the properties of the node under the cursor in the
//! active scene tab, a section per class of its inheritance (as the Godot
//! inspector shows them), then what the scene sets that no class declares
//! (script variables, metadata, theme overrides). Values come from the
//! scene, else the class's default (faded); an edit sets the property in
//! the scene's text as one undo step, the arrow puts the default back.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use godot_project::{ClassDb, SceneNode};
use iced::widget::tooltip::Position;
use iced::widget::{button, checkbox, container, pick_list, row, scrollable, space, svg, text, text_input, Column};
use iced::{Alignment, Element, Font, Length};

use crate::scene_tree::{SceneEdit, SceneMessage, Tree};
use crate::{style, Message};

pub const FILTER_INPUT: &str = "inspector-filter";

#[derive(Debug, Clone)]
pub enum InspectorMessage {
    Filter(String),
    /// Typing in a property's field (kept until Enter).
    Input(String, String),
    /// Enter in a property's field: set it, written as its type is.
    Commit { scene: PathBuf, node: String, key: String, type_name: String },
}

/// What the view keeps between frames: its filter, and the field being typed in.
#[derive(Debug, Default)]
pub struct InspectorState {
    pub filter: String,
    /// Property name and the text typed so far.
    pub editing: Option<(String, String)>,
}

impl InspectorState {
    pub fn update(&mut self, message: &InspectorMessage) {
        match message {
            InspectorMessage::Filter(filter) => self.filter = filter.clone(),
            InspectorMessage::Input(key, value) => self.editing = Some((key.clone(), value.clone())),
            InspectorMessage::Commit { .. } => {}
        }
    }

    /// The edit Enter in `key`'s field makes, from what was typed.
    pub fn commit(&mut self, node: &str, key: &str, type_name: &str) -> Option<SceneEdit> {
        let (editing, typed) = self.editing.take()?;
        if editing != key {
            return None;
        }
        Some(SceneEdit::SetProperty { node: node.to_string(), key: key.to_string(), value: Some(encode(type_name, &typed)) })
    }
}

/// Where the class reference comes from, as the view shows it.
pub enum Classes<'a> {
    Ready(&'a ClassDb),
    Loading,
    Failed(&'a str),
}

/// One row: a property, its type, and its value in the scene (`None`:
/// the default).
struct Row<'a> {
    key: String,
    type_name: String,
    set: Option<&'a str>,
    default: Option<String>,
    enumeration: Option<String>,
    bitfield: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    scene: Option<(&Path, &Tree)>,
    cursor_line: Option<usize>,
    class_of: &dyn Fn(&Tree, &SceneNode) -> Option<String>,
    icon: &dyn Fn(&str) -> Option<svg::Handle>,
    classes: Classes<'_>,
    state: &'a InspectorState,
    theme: &theme::Theme,
) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let note = |label: String| -> Element<'a, Message> { container(text(label).size(12).color(muted)).padding([6, 12]).into() };
    let Some((path, tree)) = scene else {
        return note("Open a scene and put the cursor on a node (or click it in SCENE) to see its properties".to_string());
    };
    let Some(index) = cursor_line.and_then(|line| tree.at_line(line)) else {
        return note("Put the cursor on a node to see its properties".to_string());
    };
    let node = &tree.nodes[index];
    let set = &tree.props[index];
    let class = class_of(tree, node);

    // Who it is: icon, name, class, script.
    let class_label = class.clone().unwrap_or_else(|| "Unknown class".to_string());
    let mut header = row![].spacing(6).align_y(Alignment::Center);
    if let Some(handle) = icon(&class_label) {
        header = header.push(svg(handle).width(16).height(16));
    }
    header = header
        .push(text(node.name.clone()).size(14))
        .push(text(class_label.clone()).size(11).font(Font::MONOSPACE).color(muted));
    let mut top: Vec<Element<'a, Message>> = vec![container(header).padding([4, 8]).into()];
    if let Some(script) = &node.script {
        let link = button(text(script.clone()).size(11).color(muted)).on_press(Message::OpenOutputLink(script.clone(), 1)).style(button::text).padding([0, 8]);
        top.push(link.into());
    }
    let filter = text_input("Filter properties", &state.filter)
        .id(FILTER_INPUT)
        .on_input(|t| Message::Inspector(InspectorMessage::Filter(t)))
        .size(12)
        .padding([3, 6]);
    top.push(container(filter).padding([0, 8]).into());

    let db = match classes {
        Classes::Ready(db) => Some(db),
        Classes::Loading => {
            top.push(note("Reading Godot's class reference...".to_string()));
            None
        }
        Classes::Failed(error) => {
            top.push(note(format!("No class reference ({error}): only the values the scene sets are shown")));
            None
        }
    };

    let words: Vec<String> = state.filter.split_whitespace().map(str::to_lowercase).collect();
    let shown = |key: &str| {
        let key = key.to_lowercase();
        words.iter().all(|w| key.contains(w.as_str()) || key.replace('_', " ").contains(w.as_str()))
    };
    let value_of = |key: &str| set.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let mut declared: HashSet<&str> = HashSet::new();
    let mut sections: Vec<(String, Vec<Row<'_>>)> = Vec::new();
    if let (Some(db), Some(class)) = (db, class.as_deref()) {
        for info in db.chain(class).into_iter().filter(|c| c.name != "Object") {
            let rows: Vec<Row<'_>> = info
                .properties
                .iter()
                // Only what scenes store; the script has its own row above.
                .filter(|p| p.default.is_some() && p.name != "script")
                .inspect(|p| {
                    declared.insert(p.name.as_str());
                })
                .filter(|p| shown(&p.name))
                .map(|p| Row {
                    key: p.name.clone(),
                    type_name: p.type_name.clone(),
                    set: value_of(&p.name),
                    default: db.default_of(class, &p.name),
                    enumeration: p.enumeration.clone(),
                    bitfield: p.bitfield,
                })
                .collect();
            if !rows.is_empty() {
                sections.push((info.name.clone(), rows));
            }
        }
    }
    // What the scene sets that no class declares.
    let others: Vec<Row<'_>> = set
        .iter()
        .filter(|(k, _)| k != "script" && !declared.contains(k.as_str()) && shown(k))
        .map(|(k, v)| Row { key: k.clone(), type_name: String::new(), set: Some(v.as_str()), default: None, enumeration: None, bitfield: false })
        .collect();
    if !others.is_empty() {
        let title = if db.is_some() { "Script, metadata and overrides" } else { "Set in the scene" };
        sections.push((title.to_string(), others));
    }

    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    if sections.is_empty() && db.is_some() {
        rows.push(note(if words.is_empty() { "No property".to_string() } else { "No property matches the filter".to_string() }));
    }
    for (title, section) in sections {
        rows.push(container(text(title.to_uppercase()).size(11).color(muted)).padding(iced::Padding { top: 8.0, ..[0, 8].into() }).into());
        for property in section {
            rows.push(property_row(path, &node.path, property, db, state, theme));
        }
    }
    rows.push(space().height(8).into());
    let list = scrollable(Column::with_children(rows).width(Length::Fill)).height(Length::Fill);
    Column::with_children(top).push(list).spacing(4).into()
}

fn property_row<'a>(scene: &Path, node: &str, property: Row<'_>, db: Option<&ClassDb>, state: &'a InspectorState, theme: &theme::Theme) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let is_set = property.set.is_some();
    let value = property.set.map(str::to_string).or(property.default.clone()).unwrap_or_default();
    let edit = {
        let (scene, node, key) = (scene.to_path_buf(), node.to_string(), property.key.clone());
        move |raw: Option<String>| {
            let edit = SceneEdit::SetProperty { node: node.clone(), key: key.clone(), value: raw };
            Message::Scene(SceneMessage::Edit(scene.clone(), edit))
        }
    };

    let label = text(humanize(&property.key)).size(12).color_maybe((!is_set).then_some(muted));
    let label = style::tip(
        container(label).width(Length::FillPortion(2)),
        if property.type_name.is_empty() { property.key.clone() } else { format!("{}: {}", property.key, property.type_name) },
        Position::Left,
        ui,
    );

    let choices = property
        .enumeration
        .as_deref()
        .filter(|_| !property.bitfield)
        .and_then(|e| db.and_then(|db| db.enum_values(e)))
        .map(enum_choices);
    let editor: Element<'a, Message> = if property.type_name == "bool" {
        let checked = value == "true";
        let edit = edit.clone();
        checkbox(checked).on_toggle(move |on| edit(Some(on.to_string()))).size(14).into()
    } else if let Some(choices) = choices {
        let selected = value.parse::<i64>().ok().and_then(|v| choices.iter().find(|c| c.value == v).cloned());
        let edit = edit.clone();
        pick_list(choices, selected, move |choice: EnumChoice| edit(Some(choice.value.to_string())))
            .text_size(12)
            .padding([2, 6])
            .width(Length::Fill)
            .into()
    } else {
        let typed = state.editing.as_ref().filter(|(k, _)| *k == property.key).map(|(_, t)| t.clone());
        let shown = typed.unwrap_or_else(|| decode(&property.type_name, &value));
        let key = property.key.clone();
        let commit = Message::Inspector(InspectorMessage::Commit {
            scene: scene.to_path_buf(),
            node: node.to_string(),
            key: property.key.clone(),
            type_name: property.type_name.clone(),
        });
        let mut input = text_input("", &shown)
            .on_input(move |t| Message::Inspector(InspectorMessage::Input(key.clone(), t)))
            .on_submit(commit)
            .size(12)
            .padding([2, 6]);
        if property.type_name == "String" || property.type_name.is_empty() || property.set.is_none() {
            input = input.font(Font::MONOSPACE);
        }
        input.into()
    };

    let revert: Element<'a, Message> = if is_set && property.default.is_some() {
        let back = button(text("↺").size(12)).on_press(edit(None)).style(button::text).padding([0, 4]);
        style::tip(back, format!("Back to the default: {}", property.default.clone().unwrap_or_default()), Position::Left, ui)
    } else if is_set {
        let remove = button(text("x").size(11)).on_press(edit(None)).style(button::text).padding([0, 5]);
        style::tip(remove, "Remove it from the scene", Position::Left, ui)
    } else {
        space().width(18).into()
    };
    row![label, container(editor).width(Length::FillPortion(3)), revert]
        .spacing(6)
        .align_y(Alignment::Center)
        .padding([1, 8])
        .into()
}

/// A constant of an enum, as the dropdown lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumChoice {
    pub label: String,
    pub value: i64,
}

impl fmt::Display for EnumChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// An enum's constants, named without their shared prefix:
/// `TEXTURE_FILTER_NEAREST` → `Nearest`.
pub fn enum_choices(values: &[(String, i64)]) -> Vec<EnumChoice> {
    let names: Vec<&str> = values.iter().map(|(n, _)| n.as_str()).collect();
    let prefix = common_word_prefix(&names);
    values
        .iter()
        .map(|(name, value)| {
            let short = &name[prefix.min(name.len())..];
            EnumChoice { label: humanize(if short.is_empty() { name } else { short }), value: *value }
        })
        .collect()
}

/// Length of the `WORD_` prefix every name shares (whole words only).
fn common_word_prefix(names: &[&str]) -> usize {
    let Some(first) = names.first() else {
        return 0;
    };
    if names.len() < 2 {
        return 0;
    }
    let mut len = 0;
    for (i, c) in first.char_indices() {
        if c == '_' && names.iter().all(|n| n.len() > i + 1 && n[..=i] == first[..=i]) {
            len = i + 1;
        }
    }
    len
}

/// `texture_filter` → `Texture Filter`, `NEAREST_WITH_MIPMAPS` → `Nearest With Mipmaps`.
pub fn humanize(name: &str) -> String {
    name.split(['_', '/'])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let lower = word.to_lowercase();
            let mut chars = lower.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A raw value as the field shows it: strings without their quotes.
pub fn decode(type_name: &str, raw: &str) -> String {
    let inner = match type_name {
        "String" => raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')),
        "StringName" => raw.strip_prefix("&\"").and_then(|r| r.strip_suffix('"')),
        "NodePath" => raw.strip_prefix("NodePath(\"").and_then(|r| r.strip_suffix("\")")),
        _ => None,
    };
    match inner {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\"),
        None => raw.to_string(),
    }
}

/// What a field's text is written as in the scene: strings quoted.
pub fn encode(type_name: &str, typed: &str) -> String {
    let quoted = || format!("\"{}\"", typed.replace('\\', "\\\\").replace('"', "\\\""));
    match type_name {
        "String" => quoted(),
        "StringName" => format!("&{}", quoted()),
        "NodePath" => format!("NodePath({})", quoted()),
        _ => typed.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_godots() {
        assert_eq!(humanize("texture_filter"), "Texture Filter");
        assert_eq!(humanize("metadata/_edit_lock_"), "Metadata Edit Lock");
        let values = [("TEXTURE_FILTER_PARENT_NODE".to_string(), 0), ("TEXTURE_FILTER_NEAREST".to_string(), 1)];
        let choices = enum_choices(&values);
        assert_eq!(choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Parent Node", "Nearest"]);
        assert_eq!(choices[1].value, 1);
        // Names sharing part of a word keep it.
        let values = [("SIZE_FILL".to_string(), 1), ("SIZE_FILLER".to_string(), 2)];
        assert_eq!(enum_choices(&values)[1].label, "Filler");
        assert_eq!(enum_choices(&[("ONLY".to_string(), 0)])[0].label, "Only");
    }

    #[test]
    fn strings_are_edited_without_quotes() {
        assert_eq!(decode("String", "\"say \\\"hi\\\"\""), "say \"hi\"");
        assert_eq!(encode("String", "say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(decode("StringName", "&\"jump\""), "jump");
        assert_eq!(encode("StringName", "jump"), "&\"jump\"");
        assert_eq!(decode("NodePath", "NodePath(\"../A\")"), "../A");
        assert_eq!(encode("NodePath", "../A"), "NodePath(\"../A\")");
        assert_eq!(decode("Vector2", "Vector2(1, 2)"), "Vector2(1, 2)");
        assert_eq!(encode("float", " 1.5 "), "1.5");
    }

    #[test]
    fn commits_only_the_field_typed_in() {
        let mut state = InspectorState::default();
        state.update(&InspectorMessage::Input("text".into(), "Hello".into()));
        assert_eq!(state.commit("Label", "other", "String"), None);
        state.update(&InspectorMessage::Input("text".into(), "Hello".into()));
        assert_eq!(
            state.commit("Label", "text", "String"),
            Some(SceneEdit::SetProperty { node: "Label".into(), key: "text".into(), value: Some("\"Hello\"".into()) })
        );
        assert_eq!(state.editing, None);
    }
}
