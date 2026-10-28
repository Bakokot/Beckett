//! The SCENE view: the project's scenes by folder (a click opens one), and
//! the node tree of the scene (`.tscn`) in the active tab, edited from each
//! node's right-click menu: add a child, rename, move up / down or under
//! another node, delete, attach or detach a script. An edit changes the
//! tab's text as one undo step; Ctrl+S saves it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use godot_project::{Scene, SceneNode};
use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, svg, text, text_input, Column};
use iced::{Alignment, Background, Border, Element, Font, Length};

use crate::context_menu::context_area;
use crate::file_icons::Icon;
use crate::{style, ContextTarget, Message};

pub const RENAME_INPUT: &str = "scene-rename-input";
pub const FILTER_INPUT: &str = "scene-filter-input";

/// An edit of a scene's tree; nodes by path from the root (`.`).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneEdit {
    Add { parent: String, type_name: String },
    Rename { node: String, name: String },
    Remove(String),
    /// Up (-1) or down (1) among its siblings.
    Shift(String, isize),
    Reparent { node: String, parent: String },
    /// A `res://` script, with its `uid://` when it has one.
    AttachScript { node: String, script: String, uid: Option<String> },
    DetachScript(String),
    /// An instance of a `res://` scene (with its `uid://`) under `parent`.
    Instance { parent: String, name: String, scene: String, uid: Option<String> },
}

#[derive(Debug, Clone)]
pub enum SceneMessage {
    /// Apply an edit to the scene open in this tab.
    Edit(PathBuf, SceneEdit),
    /// Ask which type of node to add under this one.
    ChooseType(PathBuf, String),
    /// Ask where to move this node.
    ChooseParent(PathBuf, String),
    /// Ask which script to attach to this node.
    ChooseScript(PathBuf, String),
    StartRename(PathBuf, String),
    RenameInput(String),
    RenameSubmit,
    RenameClose,
    /// The SCENES list's filter changed.
    Filter(String),
    /// Fold or unfold the SCENES list.
    ToggleList,
    /// Open or close a folder of the SCENES list.
    ToggleFolder(PathBuf),
    /// Ask which scene to instance under this node.
    ChooseInstance(PathBuf, String),
    /// Fold or unfold a node's children in the tree.
    ToggleNode(PathBuf, String),
    /// Ask the root type of a new scene in this folder.
    NewScene(PathBuf),
    /// Ask the name of a new scene in this folder, with a root of this type.
    StartNewScene(PathBuf, String),
    NewSceneInput(String),
    NewSceneSubmit,
    NewSceneClose,
}

/// The name field of a new scene, shown at the top of the view.
pub struct NewSceneBar {
    pub folder: PathBuf,
    /// The folder as `res://menus/`.
    pub label: String,
    pub root_type: String,
    pub name: String,
    pub error: Option<String>,
}

impl NewSceneBar {
    /// The scene's file: its name in snake_case, as Godot names them.
    pub fn file(&self) -> PathBuf {
        self.folder.join(format!("{}.tscn", file_stem(self.name.trim())))
    }
}

/// `MainMenu` → `main_menu`, `Level 2` → `level_2`, `HUD` → `hud`.
pub fn file_stem(name: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<char> = None;
    for c in name.chars() {
        if c.is_whitespace() || c == '-' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
        } else {
            if c.is_uppercase() && previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        }
        previous = Some(c);
    }
    out
}

/// The name field shown above the tree while a node is renamed.
pub struct RenameBar {
    pub scene: PathBuf,
    pub node: String,
    pub name: String,
    pub error: Option<String>,
}

/// A scene's nodes and the line of each section, as the panel shows them.
pub struct Tree {
    pub nodes: Vec<SceneNode>,
    /// Zero-based line of each section's header, by section index.
    pub lines: Vec<usize>,
}

impl Tree {
    pub fn parse(text: &str) -> Result<Self, String> {
        let scene = Scene::parse(text).map_err(|e| e.to_string())?;
        Ok(Self { nodes: scene.nodes(), lines: scene.section_lines() })
    }

    pub fn node(&self, path: &str) -> Option<&SceneNode> {
        self.nodes.iter().find(|n| n.path == path)
    }

    fn line(&self, node: &SceneNode) -> usize {
        self.lines.get(node.section).copied().unwrap_or(0)
    }

    /// The node whose section holds `line`: the last one starting at or
    /// before it.
    fn at_line(&self, line: usize) -> Option<usize> {
        self.nodes.iter().rposition(|n| self.line(n) <= line)
    }

    /// Paths of the node's siblings, itself included, in order.
    pub fn siblings(&self, node: &SceneNode) -> Vec<&str> {
        self.nodes.iter().filter(|n| n.parent == node.parent).map(|n| n.path.as_str()).collect()
    }

    /// A node's class: its type, else the class it has in the scene it
    /// comes from (the root of an instanced scene, or a node inside one),
    /// read with `scene_of` (a `res://` path's tree).
    pub fn class_of(&self, node: &SceneNode, scene_of: &dyn Fn(&str) -> Option<Parsed>) -> Option<String> {
        self.class_within(node, scene_of, 0)
    }

    fn class_within(&self, node: &SceneNode, scene_of: &dyn Fn(&str) -> Option<Parsed>, depth: usize) -> Option<String> {
        if let Some(type_name) = &node.type_name {
            return Some(type_name.clone());
        }
        // Scenes instancing each other in a loop don't load in Godot either.
        if depth > 8 {
            return None;
        }
        // The node itself is instanced, or it sits inside an instanced node.
        let (instance, inner) = match &node.instance {
            Some(instance) => (instance.clone(), ".".to_string()),
            None => {
                let owner = self
                    .nodes
                    .iter()
                    .filter(|n| n.instance.is_some() && node.path.starts_with(&format!("{}/", n.path)))
                    .max_by_key(|n| n.path.len())?;
                let inner = node.path[owner.path.len() + 1..].to_string();
                (owner.instance.clone()?, inner)
            }
        };
        let parsed = scene_of(&instance)?;
        let scene = parsed.as_ref().as_ref().ok()?;
        let found = scene.node(&inner)?;
        scene.class_within(found, scene_of, depth + 1)
    }
}

/// A parsed tree, or why the scene can't be read.
pub type Parsed = Rc<Result<Tree, String>>;

/// The Godot icon for a class: its own, else its nearest parent class's
/// (`classes` from the language server), else `Node`'s.
pub fn icon_name(class: Option<&str>, classes: &[lsp_client::NativeClass]) -> String {
    let Some(class) = class else {
        return "Node".to_string();
    };
    std::iter::once(class.to_string())
        .chain(crate::class_ref::ancestors(class, classes))
        .find(|name| crate::godot_icons::has(name))
        .unwrap_or_else(|| "Node".to_string())
}

/// The parsed tree of the scene shown, kept until its tab's text changes.
#[derive(Default)]
pub struct Cache(RefCell<Option<(PathBuf, u64, Parsed)>>);

impl Cache {
    pub fn get(&self, path: &Path, version: u64, text: impl FnOnce() -> String) -> Parsed {
        let mut cached = self.0.borrow_mut();
        if let Some((p, v, tree)) = cached.as_ref() {
            if p == path && *v == version {
                return tree.clone();
            }
        }
        let tree = Rc::new(Tree::parse(&text()));
        *cached = Some((path.to_path_buf(), version, tree.clone()));
        tree
    }
}

/// Applies an edit to scene text; the new text, or why it can't be done.
pub fn apply(text: &str, edit: &SceneEdit) -> Result<String, String> {
    let mut scene = Scene::parse(text).map_err(|e| e.to_string())?;
    let result = match edit {
        SceneEdit::Add { parent, type_name } => scene.add_node(parent, type_name, type_name).map(drop),
        SceneEdit::Rename { node, name } => scene.rename_node(node, name.trim()).map(drop),
        SceneEdit::Remove(node) => scene.remove_node(node),
        SceneEdit::Shift(node, delta) => {
            let tree = Tree { nodes: scene.nodes(), lines: Vec::new() };
            let found = tree.node(node).ok_or_else(|| format!("no node at \"{node}\""))?;
            let parent = found.parent.clone().ok_or("the root node can't be moved")?;
            let siblings = tree.siblings(found);
            let index = siblings.iter().position(|p| p == node).unwrap_or(0);
            let Some(target) = index.checked_add_signed(*delta).filter(|&i| i < siblings.len()) else {
                return Ok(text.to_string());
            };
            // Among the other children: the one now at `target` goes before it.
            scene.reparent_node(node, &parent, Some(target)).map(drop)
        }
        SceneEdit::Reparent { node, parent } => scene.reparent_node(node, parent, None).map(drop),
        SceneEdit::AttachScript { node, script, uid } => scene.attach_script(node, script, uid.as_deref()),
        SceneEdit::DetachScript(node) => scene.detach_script(node),
        SceneEdit::Instance { parent, name, scene: res, uid } => {
            scene.instance_scene(parent, name, res, uid.as_deref()).map(drop)
        }
    };
    result.map_err(|e| e.to_string())?;
    Ok(scene.to_text())
}

/// Classes that can be added as nodes, the common ones first: every class
/// deriving from `Node` the language server listed, or a short list
/// without it.
pub fn node_types(classes: &[lsp_client::NativeClass]) -> Vec<String> {
    let mut types: Vec<String> = classes
        .iter()
        .filter(|c| c.name == "Node" || crate::class_ref::ancestors(&c.name, classes).iter().any(|a| a == "Node"))
        .map(|c| c.name.clone())
        .collect();
    if types.is_empty() {
        return COMMON_TYPES.iter().map(|t| t.to_string()).collect();
    }
    types.sort_by_key(|t| (COMMON_TYPES.iter().position(|c| c == t).unwrap_or(usize::MAX), t.clone()));
    types
}

const COMMON_TYPES: &[&str] = &[
    "Node",
    "Node2D",
    "Node3D",
    "Control",
    "Sprite2D",
    "CharacterBody2D",
    "Area2D",
    "CollisionShape2D",
    "AnimatedSprite2D",
    "Camera2D",
    "Timer",
    "Label",
    "Button",
    "Panel",
    "ColorRect",
    "TextureRect",
    "VBoxContainer",
    "HBoxContainer",
    "MarginContainer",
    "CanvasLayer",
    "AudioStreamPlayer",
    "AnimationPlayer",
    "RigidBody2D",
    "StaticBody2D",
    "TileMapLayer",
    "Marker2D",
    "Sprite3D",
    "MeshInstance3D",
    "CharacterBody3D",
    "Area3D",
    "CollisionShape3D",
    "Camera3D",
    "DirectionalLight3D",
];

/// A folder of the SCENES tree: only the subfolders that hold scenes, and
/// its scenes.
#[derive(Debug, Default, PartialEq)]
pub struct Folder<'a> {
    pub path: PathBuf,
    pub name: String,
    pub folders: Vec<Folder<'a>>,
    pub scenes: Vec<&'a PathBuf>,
}

impl Folder<'_> {
    fn sort(&mut self) {
        self.folders.sort_by_key(|f| f.name.to_lowercase());
        self.scenes.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        self.folders.iter_mut().for_each(Folder::sort);
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.scenes.is_empty()
    }
}

/// The project's scenes as a folder tree from `root`, those whose `res://`
/// path contains every word of `filter` (any case); folders first, sorted.
pub fn folder_tree<'a>(scenes: &'a [PathBuf], root: &Path, filter: &str) -> Folder<'a> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut top = Folder { path: root.to_path_buf(), ..Folder::default() };
    for path in scenes {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let res = relative.to_string_lossy().replace('\\', "/").to_lowercase();
        if !words.iter().all(|w| res.contains(w.as_str())) {
            continue;
        }
        let mut folder = &mut top;
        for part in relative.parent().into_iter().flat_map(Path::components) {
            let name = part.as_os_str().to_string_lossy().into_owned();
            let index = match folder.folders.iter().position(|f| f.name == name) {
                Some(index) => index,
                None => {
                    let path = folder.path.join(&name);
                    folder.folders.push(Folder { path, name, ..Folder::default() });
                    folder.folders.len() - 1
                }
            };
            folder = &mut folder.folders[index];
        }
        folder.scenes.push(path);
    }
    top.sort();
    top
}

/// The SCENES list above the tree: what it is filtered by, its open
/// folders, and whether it is folded away.
#[derive(Default)]
pub struct ListState {
    pub filter: String,
    pub expanded: HashSet<PathBuf>,
    pub collapsed: bool,
    /// Nodes whose children are hidden: scene file and node path.
    pub folded: HashSet<(PathBuf, String)>,
}

/// Rows of a folder's contents, like the FILES tree: folders (with their
/// arrow) then scenes. While filtering, every folder is open.
fn push_folder_rows<'a>(
    folder: &Folder<'_>,
    depth: u16,
    list: &ListState,
    active: Option<&Path>,
    highlight: iced::Color,
    rows: &mut Vec<Element<'a, Message>>,
) {
    let filtering = !list.filter.trim().is_empty();
    let indent = || space().width(f32::from(depth) * 14.0);
    for sub in &folder.folders {
        let open = filtering || list.expanded.contains(&sub.path);
        let arrow = if open { Icon::ArrowDown } else { Icon::ArrowRight };
        let content = row![
            indent(),
            svg(arrow.handle()).width(12).height(12),
            svg(Icon::for_folder(open).handle()).width(16).height(16),
            text(sub.name.clone()).size(13),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let entry = button(content)
            .on_press(Message::Scene(SceneMessage::ToggleFolder(sub.path.clone())))
            .width(Length::Fill)
            .padding([2, 8])
            .style(button::text);
        let target = ContextTarget::Folder(sub.path.clone());
        rows.push(context_area(entry, move |point| Message::ContextMenu(target.clone(), point)).into());
        if open {
            push_folder_rows(sub, depth + 1, list, active, highlight, rows);
        }
    }
    for path in &folder.scenes {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let selected = active.is_some_and(|a| a == path.as_path());
        let content = row![
            indent(),
            space().width(12),
            svg(Icon::for_file(path).handle()).width(16).height(16),
            text(name).size(13),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let entry = button(content)
            .on_press(Message::OpenFile(path.to_path_buf()))
            .width(Length::Fill)
            .padding([2, 8])
            .style(move |theme: &iced::Theme, status| selected_style(theme, status, selected, highlight));
        let target = ContextTarget::File(path.to_path_buf());
        rows.push(context_area(entry, move |point| Message::ContextMenu(target.clone(), point)).into());
    }
}

/// The project's scenes (to open one) above the tree of the active one.
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    scenes: &'a [PathBuf],
    root: &Path,
    list: &'a ListState,
    scene: Option<(&Path, &Result<Tree, String>)>,
    cursor_line: Option<usize>,
    rename: Option<&'a RenameBar>,
    new_scene: Option<&'a NewSceneBar>,
    icon: &dyn Fn(&Tree, &SceneNode) -> Option<svg::Handle>,
    theme: &theme::Theme,
) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let message = |m: SceneMessage| Message::Scene(m);
    let heading = |label: String| text(label).size(11).color(muted);
    let new_scene_bar = new_scene.map(|bar| new_scene_row(bar, ui));

    let arrow = if list.collapsed { ">" } else { "v" };
    let toggle = button(row![text(arrow).size(11).font(Font::MONOSPACE).color(muted), heading(format!("SCENES ({})", scenes.len()))].spacing(6))
        .on_press(message(SceneMessage::ToggleList))
        .style(button::text)
        .width(Length::Fill)
        .padding([2, 8]);
    let toggle = style::tip(toggle, if list.collapsed { "Show the project's scenes" } else { "Hide the scene list" }, Position::Bottom, ui);
    let mut parts: Vec<Element<'a, Message>> = vec![toggle];
    parts.extend(new_scene_bar);

    if !list.collapsed {
        let filter = text_input("Filter scenes", &list.filter)
            .id(FILTER_INPUT)
            .on_input(move |t| message(SceneMessage::Filter(t)))
            .size(12)
            .padding([3, 6]);
        parts.push(container(filter).padding([0, 8]).into());
        let active = scene.map(|(path, _)| path);
        let highlight = style::to_color(ui.accent).scale_alpha(0.2);
        let mut rows: Vec<Element<'a, Message>> = Vec::new();
        let tree = folder_tree(scenes, root, &list.filter);
        if tree.is_empty() {
            let note = if scenes.is_empty() { "No scene (.tscn) in the project" } else { "No scene matches the filter" };
            rows.push(container(text(note).size(12).color(muted)).padding([2, 12]).into());
        }
        push_folder_rows(&tree, 0, list, active, highlight, &mut rows);
        let list = scrollable(Column::with_children(rows).width(Length::Fill));
        // Sharing the height with the tree when a scene is open.
        parts.push(list.height(if scene.is_some() { Length::FillPortion(2) } else { Length::Fill }).into());
    }

    if let Some((path, _)) = scene {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        parts.push(container(heading(format!("NODES OF {}", name.to_uppercase()))).padding(iced::Padding { top: 6.0, ..[0, 8].into() }).into());
        parts.push(container(tree_view(scene, cursor_line, rename, &list.folded, icon, theme)).height(Length::FillPortion(3)).into());
    } else if list.collapsed {
        let note = "Open a scene to see and edit its node tree";
        parts.push(container(text(note).size(12).color(muted)).padding([6, 12]).into());
    }
    Column::with_children(parts).spacing(4).height(Length::Fill).into()
}

fn selected_style(theme: &iced::Theme, status: button::Status, selected: bool, highlight: iced::Color) -> button::Style {
    let base = button::text(theme, status);
    if selected {
        button::Style {
            background: Some(Background::Color(highlight)),
            border: Border { radius: 3.0.into(), ..Border::default() },
            ..base
        }
    } else {
        base
    }
}

/// The node tree of the scene open in the active tab.
fn tree_view<'a>(
    scene: Option<(&Path, &Result<Tree, String>)>,
    cursor_line: Option<usize>,
    rename: Option<&'a RenameBar>,
    folded: &HashSet<(PathBuf, String)>,
    icon: &dyn Fn(&Tree, &SceneNode) -> Option<svg::Handle>,
    theme: &theme::Theme,
) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let note = |label: String| container(text(label).size(12).color(muted)).padding([6, 12]);
    let Some((path, tree)) = scene else {
        return note("Open a scene (.tscn) to see and edit its node tree".to_string()).into();
    };
    let tree = match tree {
        Ok(tree) => tree,
        Err(error) => return note(format!("Can't read this scene: {error}")).into(),
    };
    if tree.nodes.is_empty() {
        return note("This scene has no nodes".to_string()).into();
    }

    let here = cursor_line.and_then(|line| tree.at_line(line));
    let highlight = style::to_color(ui.accent).scale_alpha(0.2);
    let type_color = style::to_color(theme.syntax.r#type);
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    let rename = rename.filter(|bar| bar.scene == path);
    let parents: HashSet<&str> = tree.nodes.iter().filter_map(|n| n.parent.as_deref()).collect();
    // The folded node whose descendants are being skipped.
    let mut hidden_under: Option<&str> = None;
    for (index, node) in tree.nodes.iter().enumerate() {
        if hidden_under.is_some_and(|folded| is_within(&node.path, folded) && node.path != folded) {
            continue;
        }
        let is_folded = folded.contains(&(path.to_path_buf(), node.path.clone()));
        hidden_under = is_folded.then_some(node.path.as_str());
        let scene = path.to_path_buf();
        // Nodes with children get an arrow to fold them; others its room.
        let arrow: Element<'a, Message> = if parents.contains(node.path.as_str()) {
            let arrow = if is_folded { Icon::ArrowRight } else { Icon::ArrowDown };
            button(svg(arrow.handle()).width(12).height(12))
                .on_press(Message::Scene(SceneMessage::ToggleNode(scene.clone(), node.path.clone())))
                .style(button::text)
                .padding(0)
                .into()
        } else {
            space().width(12).into()
        };
        if let Some(bar) = rename.filter(|bar| bar.node == node.path) {
            rows.push(rename_row(bar, node.depth, ui));
            continue;
        }
        // Nodes of an instanced scene, only overridden here: faded.
        let name_color = (!node.is_editable()).then_some(muted);
        let kind = match (&node.type_name, &node.instance) {
            (Some(type_name), _) => type_name.clone(),
            (None, Some(_)) => "scene".to_string(),
            (None, None) => String::new(),
        };
        let icon: Element<'a, Message> = match icon(tree, node) {
            Some(handle) => svg(handle).width(16).height(16).opacity(if node.is_editable() { 1.0_f32 } else { 0.6 }).into(),
            None => space().width(16).into(),
        };
        let mut content = row![
            space().width(14.0 * node.depth as f32),
            arrow,
            icon,
            text(node.name.clone()).size(13).color_maybe(name_color),
            text(kind).size(11).font(Font::MONOSPACE).color(type_color.scale_alpha(0.8)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        content = content.push(space::horizontal());
        for (res, label, tip) in [(&node.instance, "tscn", "Open the instanced scene"), (&node.script, "gd", "Open the script")] {
            if let Some(res) = res {
                let link = button(text(label).size(11).font(Font::MONOSPACE))
                    .on_press(Message::OpenOutputLink(res.clone(), 1))
                    .style(button::text)
                    .padding([0, 4]);
                content = content.push(style::tip(link, format!("{tip}: {res}"), Position::Left, ui));
            }
        }
        let selected = here == Some(index);
        let line = tree.line(node) as u32;
        let node_button = button(content)
            .on_press(Message::JumpTo(scene.clone(), line, 0))
            .width(Length::Fill)
            .padding([2, 8])
            .style(move |theme: &iced::Theme, status| selected_style(theme, status, selected, highlight));
        let target = ContextTarget::SceneNode(scene, node.path.clone());
        rows.push(context_area(node_button, move |point| Message::ContextMenu(target.clone(), point)).into());
    }
    let hint = text("Right-click a node to edit the tree").size(11).color(muted);
    let list = scrollable(Column::with_children(rows).width(Length::Fill)).height(Length::Fill);
    column![container(hint).padding([0, 12]), list].spacing(4).into()
}

/// Whether node `path` is `ancestor` or below it (every node is below `.`).
fn is_within(path: &str, ancestor: &str) -> bool {
    ancestor == "." || path == ancestor || path.starts_with(&format!("{ancestor}/"))
}

pub const NEW_SCENE_INPUT: &str = "scene-new-input";

fn new_scene_row<'a>(bar: &'a NewSceneBar, ui: &theme::UiColors) -> Element<'a, Message> {
    let message = |m: SceneMessage| Message::Scene(m);
    let muted = style::to_color(ui.text_muted);
    let input = text_input("Scene name, e.g. MainMenu", &bar.name)
        .id(NEW_SCENE_INPUT)
        .on_input(move |name| message(SceneMessage::NewSceneInput(name)))
        .on_submit(message(SceneMessage::NewSceneSubmit))
        .size(13)
        .padding([2, 6]);
    let title = text(format!("New {} scene in {}", bar.root_type, bar.label)).size(11).color(muted);
    let hint = match &bar.error {
        Some(error) => text(error.as_str()).size(11).color(style::to_color(ui.danger)),
        None if bar.name.trim().is_empty() => text("Enter to create, Escape to cancel").size(11).color(muted),
        None => {
            let file = bar.file();
            let file = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            text(format!("Creates {}{file}", bar.label)).size(11).color(muted)
        }
    };
    let content = column![title, input, hint].spacing(3).padding([4, 8]);
    crate::on_escape::on_escape(container(content).style(style::surface(ui)), message(SceneMessage::NewSceneClose)).into()
}

fn rename_row<'a>(bar: &'a RenameBar, depth: usize, ui: &theme::UiColors) -> Element<'a, Message> {
    let message = |m: SceneMessage| Message::Scene(m);
    let input = text_input("Node name", &bar.name)
        .id(RENAME_INPUT)
        .on_input(move |name| message(SceneMessage::RenameInput(name)))
        .on_submit(message(SceneMessage::RenameSubmit))
        .size(13)
        .padding([2, 6]);
    let field = row![space().width(14.0 * depth as f32), input].padding([1, 8]);
    let field: Element<'a, Message> = crate::on_escape::on_escape(field, message(SceneMessage::RenameClose)).into();
    match &bar.error {
        Some(error) => {
            let error = text(error.as_str()).size(11).color(style::to_color(ui.danger));
            column![field, container(error).padding([0, 12])].into()
        }
        None => field,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = "[gd_scene format=3]\n\n[node name=\"Main\" type=\"Node2D\"]\n\n\
        [node name=\"A\" type=\"Node2D\" parent=\".\"]\n\n[node name=\"B\" type=\"Node2D\" parent=\".\"]\n\n\
        [node name=\"C\" type=\"Node2D\" parent=\".\"]\n";

    fn order(text: &str) -> Vec<String> {
        Tree::parse(text).unwrap().nodes.into_iter().map(|n| n.path).collect()
    }

    #[test]
    fn shifts_nodes_among_their_siblings() {
        let down = apply(SCENE, &SceneEdit::Shift("A".into(), 1)).unwrap();
        assert_eq!(order(&down), [".", "B", "A", "C"]);
        let up = apply(SCENE, &SceneEdit::Shift("C".into(), -1)).unwrap();
        assert_eq!(order(&up), [".", "A", "C", "B"]);
        // Already first or last: unchanged.
        assert_eq!(apply(SCENE, &SceneEdit::Shift("A".into(), -1)).unwrap(), SCENE);
        assert_eq!(apply(SCENE, &SceneEdit::Shift("C".into(), 1)).unwrap(), SCENE);
        assert!(apply(SCENE, &SceneEdit::Shift(".".into(), 1)).is_err());
    }

    #[test]
    fn instances_scenes_under_a_node() {
        let edit = SceneEdit::Instance { parent: "B".into(), name: "Gun".into(), scene: "res://gun.tscn".into(), uid: None };
        let text = apply(SCENE, &edit).unwrap();
        assert_eq!(order(&text), [".", "A", "B", "B/Gun", "C"]);
        assert!(text.contains("[ext_resource type=\"PackedScene\" path=\"res://gun.tscn\""));
    }

    #[test]
    fn new_scene_files_are_snake_case() {
        assert_eq!(file_stem("MainMenu"), "main_menu");
        assert_eq!(file_stem("HUD"), "hud");
        assert_eq!(file_stem("Level2Boss"), "level2_boss");
        assert_eq!(file_stem("pause menu"), "pause_menu");
        assert_eq!(file_stem("player"), "player");
        let bar = NewSceneBar {
            folder: PathBuf::from("/game/menus"),
            label: "res://menus/".into(),
            root_type: "Control".into(),
            name: " MainMenu ".into(),
            error: None,
        };
        assert_eq!(bar.file(), Path::new("/game/menus").join("main_menu.tscn"));
        assert!(is_within("A/B", "A") && is_within("A", ".") && !is_within("AB", "A"));
    }

    #[test]
    fn adds_nodes_named_after_their_type() {
        let text = apply(SCENE, &SceneEdit::Add { parent: "B".into(), type_name: "Sprite2D".into() }).unwrap();
        assert_eq!(order(&text), [".", "A", "B", "B/Sprite2D", "C"]);
        let error = apply(SCENE, &SceneEdit::Rename { node: "A".into(), name: "B".into() }).unwrap_err();
        assert_eq!(error, "a sibling is already named \"B\"");
    }

    #[test]
    fn classes_come_from_instanced_scenes() {
        let gun = "[gd_scene format=3]\n\n[node name=\"Gun\" type=\"Area2D\"]\n\n\
            [node name=\"Muzzle\" type=\"Marker2D\" parent=\".\"]\n";
        let main = "[gd_scene format=3]\n\n[ext_resource type=\"PackedScene\" path=\"res://gun.tscn\" id=\"1_a\"]\n\n\
            [node name=\"Main\" type=\"Node2D\"]\n\n[node name=\"Gun\" parent=\".\" instance=ExtResource(\"1_a\")]\n\n\
            [node name=\"Muzzle\" parent=\"Gun\" index=\"0\"]\n\n[node name=\"Lost\" parent=\".\" instance=ExtResource(\"1_a\")]\n";
        let gun = Rc::new(Tree::parse(gun));
        let scene_of = |res: &str| (res == "res://gun.tscn").then(|| gun.clone());
        let tree = Tree::parse(main).unwrap();
        let class = |path: &str| tree.class_of(tree.node(path).unwrap(), &scene_of);
        assert_eq!(class(".").as_deref(), Some("Node2D"));
        assert_eq!(class("Gun").as_deref(), Some("Area2D"));
        assert_eq!(class("Gun/Muzzle").as_deref(), Some("Marker2D"));
        assert_eq!(icon_name(class("Gun").as_deref(), &[]), "Area2D");
        assert_eq!(icon_name(None, &[]), "Node");
        let class = |name: &str, inherits: &str| lsp_client::NativeClass { name: name.into(), inherits: inherits.into() };
        let classes = [class("MyNode", "Node2D"), class("Node2D", "CanvasItem")];
        assert_eq!(icon_name(Some("MyNode"), &classes), "Node2D");
        assert_eq!(icon_name(Some("Unknown"), &[]), "Node");
    }

    #[test]
    fn finds_the_node_under_the_cursor() {
        let tree = Tree::parse(SCENE).unwrap();
        assert_eq!(tree.at_line(0), None);
        assert_eq!(tree.at_line(2), Some(0));
        assert_eq!(tree.at_line(5), Some(1));
        assert_eq!(tree.at_line(100), Some(3));
    }

    #[test]
    fn builds_a_folder_tree_of_the_scenes() {
        let root = Path::new("/game");
        let scenes: Vec<PathBuf> =
            ["menus/Pause.tscn", "main.tscn", "menus/sub/x.tscn", "menus/credits.tscn", "Levels/one.tscn", "a.tscn"]
                .iter()
                .map(|p| root.join(p))
                .collect();
        // Folder and scene names, depth first, folders before scenes.
        fn names(folder: &Folder<'_>, depth: usize, out: &mut Vec<String>) {
            for sub in &folder.folders {
                out.push(format!("{}{}/", "  ".repeat(depth), sub.name));
                names(sub, depth + 1, out);
            }
            for scene in &folder.scenes {
                out.push(format!("{}{}", "  ".repeat(depth), scene.file_name().unwrap().to_string_lossy()));
            }
        }
        let list = |filter: &str| {
            let mut out = Vec::new();
            names(&folder_tree(&scenes, root, filter), 0, &mut out);
            out
        };
        assert_eq!(
            list(""),
            ["Levels/", "  one.tscn", "menus/", "  sub/", "    x.tscn", "  credits.tscn", "  Pause.tscn", "a.tscn", "main.tscn"]
        );
        assert_eq!(folder_tree(&scenes, root, "").folders[1].path, root.join("menus"));
        assert_eq!(list("MENU pau"), ["menus/", "  Pause.tscn"]);
        assert!(folder_tree(&scenes, root, "nothing").is_empty());
    }

    #[test]
    fn node_types_put_common_ones_first() {
        let class = |name: &str, inherits: &str| lsp_client::NativeClass { name: name.into(), inherits: inherits.into() };
        let classes = [class("Object", ""), class("Node", "Object"), class("Zebra", "Node"), class("Node2D", "Node"), class("Resource", "Object")];
        assert_eq!(node_types(&classes), ["Node", "Node2D", "Zebra"]);
        assert_eq!(node_types(&[]).first().map(String::as_str), Some("Node"));
    }
}
