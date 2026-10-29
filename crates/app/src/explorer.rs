//! Project file tree with lazily loaded directories, filterable by file type.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use iced::widget::{button, container, mouse_area, row, svg, text, tooltip::Position, Column, Row, Space};
use iced::{Background, Border, Color, Element, Length};

use crate::context_menu::context_area;
use crate::drag::drag_source;
use crate::file_icons::Icon;
use crate::git::{self, FileStatus};
use crate::{style, ContextTarget, Message};

pub struct Explorer {
    root: Node,
    /// Extensions (lowercase, no dot) of the files shown; empty: every file.
    filter: BTreeSet<String>,
    /// The row of type chips is open.
    pub chips_open: bool,
    /// Built when first needed, dropped when files or the filter change.
    index: RefCell<Option<Index>>,
}

#[derive(Debug, Clone)]
pub enum FilterMessage {
    /// Open or close the row of type chips.
    ToggleChips,
    /// Show or hide files of this extension.
    Toggle(String),
    /// Show only files of this extension (or every file, when it is the only one shown).
    Only(String),
    Clear,
}

/// What the whole project holds, beyond the folders loaded so far.
struct Index {
    /// File count by extension.
    extensions: BTreeMap<String, usize>,
    /// Folders holding a file the filter shows, at any depth.
    folders: HashSet<PathBuf>,
}

struct Node {
    path: PathBuf,
    name: String,
    is_dir: bool,
    expanded: bool,
    /// `None` until the directory is first expanded.
    children: Option<Vec<Node>>,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let mut root = Node::new(root, true);
        root.expanded = true;
        root.load_children();
        Self { root, filter: BTreeSet::new(), chips_open: false, index: RefCell::new(None) }
    }

    /// The extensions shown, empty when every file is.
    pub fn filter(&self) -> &BTreeSet<String> {
        &self.filter
    }

    pub fn set_filter(&mut self, extensions: impl IntoIterator<Item = String>) {
        self.filter = extensions.into_iter().map(|e| e.to_lowercase()).collect();
        self.index.take();
    }

    pub fn update(&mut self, message: FilterMessage) {
        match message {
            FilterMessage::ToggleChips => self.chips_open = !self.chips_open,
            FilterMessage::Toggle(extension) => {
                if !self.filter.remove(&extension) {
                    self.filter.insert(extension);
                }
            }
            FilterMessage::Only(extension) => {
                let only_this = self.filter.len() == 1 && self.filter.contains(&extension);
                self.filter = if only_this { BTreeSet::new() } else { BTreeSet::from([extension]) };
            }
            FilterMessage::Clear => self.filter.clear(),
        }
        self.index.take();
    }

    /// Expands or collapses the directory at `path`.
    pub fn toggle(&mut self, path: &Path) {
        if let Some(node) = find_mut(&mut self.root, path) {
            if node.is_dir {
                node.expanded = !node.expanded;
                if node.expanded && node.children.is_none() {
                    node.load_children();
                }
            }
        }
    }

    /// Re-reads `dir` if it is loaded, keeping which subfolders are expanded.
    pub fn refresh(&mut self, dir: &Path) {
        self.index.take();
        let Some(node) = find_mut(&mut self.root, dir) else {
            return;
        };
        let Some(mut old) = node.children.take() else {
            return;
        };
        node.load_children();
        for child in node.children.iter_mut().flatten() {
            if let Some(i) = old.iter().position(|o| o.path == child.path && o.is_dir == child.is_dir) {
                let previous = old.swap_remove(i);
                child.expanded = previous.expanded;
                child.children = previous.children;
            }
        }
    }

    pub fn view<'a>(&'a self, active: Option<&Path>, git: &GitLook<'_>, muted: Color) -> Element<'a, Message> {
        let index = self.index();
        let index = index.as_ref().expect("built by index()");
        let shown = |node: &Node| {
            self.filter.is_empty()
                || if node.is_dir { index.folders.contains(&node.path) } else { self.filter.contains(&extension(&node.path)) }
        };
        let mut rows = Vec::new();
        for child in self.root.children.iter().flatten().filter(|c| shown(c)) {
            push_rows(child, 0, active, git, &shown, &mut rows);
        }
        if rows.is_empty() && !self.filter.is_empty() {
            let types: Vec<String> = self.filter.iter().map(|e| format!(".{e}")).collect();
            let note = format!("No {} files in the project", types.join(" or "));
            rows.push(container(text(note).size(12).color(muted)).padding([6, 12]).into());
        }
        Column::with_children(rows).width(Length::Fill).into()
    }

    /// One chip per extension in the project (and per one filtered on),
    /// highlighted when shown.
    pub fn type_chips<'a>(&self, ui: &theme::UiColors) -> Element<'a, Message> {
        let index = self.index();
        let index = index.as_ref().expect("built by index()");
        let mut counts = index.extensions.clone();
        for extension in &self.filter {
            counts.entry(extension.clone()).or_insert(0);
        }
        let accent = style::to_color(ui.accent);
        let muted = style::to_color(ui.text_muted);
        let message = |m: FilterMessage| Message::FileTypes(m);
        let mut chips: Vec<Element<'a, Message>> = counts
            .into_iter()
            .map(|(extension, count)| {
                let selected = self.filter.contains(&extension);
                let icon = Icon::for_file(Path::new(&format!("file.{extension}"))).handle();
                let label = text(format!(".{extension}")).size(11).color_maybe((!selected).then_some(muted));
                let chip = button(row![svg(icon).width(12).height(12), label].spacing(3).align_y(iced::Alignment::Center))
                    .on_press(message(FilterMessage::Toggle(extension.clone())))
                    .style(move |theme: &iced::Theme, status| {
                        let base = button::text(theme, status);
                        if !selected {
                            return base;
                        }
                        // Shown: tinted and outlined in the accent color.
                        let tint = if status == button::Status::Hovered { 0.25 } else { 0.12 };
                        button::Style {
                            background: Some(Background::Color(accent.scale_alpha(tint))),
                            border: Border { color: accent.scale_alpha(0.6), width: 1.0, radius: 4.0.into() },
                            ..base
                        }
                    })
                    .padding([1, 5]);
                let chip = mouse_area(chip).on_right_press(message(FilterMessage::Only(extension)));
                let files = if count == 1 { "1 file".to_string() } else { format!("{count} files") };
                let description = format!("{files}. Click: show or hide this type. Right-click: only this type");
                style::tip(chip, description, Position::Bottom, ui)
            })
            .collect();
        if !self.filter.is_empty() {
            let clear = button(text("all").size(11)).on_press(message(FilterMessage::Clear)).style(button::text).padding([1, 5]);
            chips.push(style::tip(clear, "Show every file", Position::Bottom, ui));
        }
        container(Row::with_children(chips).spacing(4).wrap().vertical_spacing(4)).padding([2, 8]).into()
    }

    /// The project's index, built now if files or the filter changed since.
    fn index(&self) -> std::cell::Ref<'_, Option<Index>> {
        if self.index.borrow().is_none() {
            let mut index = Index { extensions: BTreeMap::new(), folders: HashSet::new() };
            walk(&self.root.path, &mut |path| {
                let extension = extension(path);
                if extension.is_empty() {
                    return;
                }
                if self.filter.contains(&extension) {
                    for folder in path.ancestors().skip(1).take_while(|f| *f != self.root.path) {
                        if !index.folders.insert(folder.to_path_buf()) {
                            break;
                        }
                    }
                }
                *index.extensions.entry(extension).or_insert(0) += 1;
            });
            *self.index.borrow_mut() = Some(index);
        }
        self.index.borrow()
    }
}

/// The lowercase extension of a file, empty when it has none.
fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Visits every file the tree would show under `dir`.
fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else { continue };
        if godot_project::is_hidden(&entry.file_name().to_string_lossy(), kind.is_dir()) {
            continue;
        }
        if kind.is_dir() {
            walk(&entry.path(), visit);
        } else {
            visit(&entry.path());
        }
    }
}

/// How names show their git status: changed files and the folders holding
/// them, new files, conflicts.
pub struct GitLook<'a> {
    pub status: &'a git::Status,
    pub modified: Color,
    pub added: Color,
    pub conflicted: Color,
}

impl GitLook<'_> {
    fn color(&self, path: &Path, is_dir: bool) -> Option<Color> {
        if is_dir {
            return self.status.folder_changed(path).then(|| self.modified.scale_alpha(0.75));
        }
        self.status.file(path).map(|status| match status {
            FileStatus::Modified => self.modified,
            FileStatus::Added => self.added,
            FileStatus::Conflicted => self.conflicted,
        })
    }
}

impl Node {
    fn new(path: PathBuf, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self {
            path,
            name,
            is_dir,
            expanded: false,
            children: None,
        }
    }

    fn load_children(&mut self) {
        let mut children: Vec<Node> = std::fs::read_dir(&self.path)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let is_dir = entry.file_type().ok()?.is_dir();
                let node = Node::new(entry.path(), is_dir);
                (!godot_project::is_hidden(&node.name, is_dir)).then_some(node)
            })
            .collect();
        children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.children = Some(children);
    }
}

fn find_mut<'a>(node: &'a mut Node, path: &Path) -> Option<&'a mut Node> {
    if node.path == path {
        return Some(node);
    }
    if !path.starts_with(&node.path) {
        return None;
    }
    node.children
        .as_mut()?
        .iter_mut()
        .find_map(|child| find_mut(child, path))
}

fn push_rows<'a>(
    node: &'a Node,
    depth: u16,
    active: Option<&Path>,
    git: &GitLook<'_>,
    shown: &dyn Fn(&Node) -> bool,
    rows: &mut Vec<Element<'a, Message>>,
) {
    // Folders get an expand arrow; files leave its space empty so names line up.
    let arrow: Element<'a, Message> = if node.is_dir {
        let arrow = if node.expanded { Icon::ArrowDown } else { Icon::ArrowRight };
        svg(arrow.handle()).width(12).height(12).into()
    } else {
        Space::new().width(12).into()
    };
    let icon = if node.is_dir { Icon::for_folder(node.expanded) } else { Icon::for_file(&node.path) }.handle();
    let message = if node.is_dir {
        Message::ToggleDir(node.path.clone())
    } else {
        Message::OpenFile(node.path.clone())
    };
    let style: fn(&iced::Theme, button::Status) -> button::Style =
        if active == Some(node.path.as_path()) {
            button::secondary
        } else {
            button::text
        };

    let mut name = text(&node.name).size(13);
    if let Some(color) = git.color(&node.path, node.is_dir) {
        name = name.color(color);
    }
    let row_button = button(
        row![
            Space::new().width(f32::from(depth) * 14.0),
            arrow,
            svg(icon).width(16).height(16),
            name,
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    )
    .on_press(message)
    .style(style)
    .width(Length::Fill)
    .padding([2, 8]);
    // Files can be dragged into code.
    let row_button: Element<'a, Message> = if node.is_dir {
        row_button.into()
    } else {
        drag_source(row_button, Message::FileDrag(Some(node.path.clone())), Message::FileDrag(None)).into()
    };
    // Right-click: the file or folder menu.
    let target = if node.is_dir { ContextTarget::Folder(node.path.clone()) } else { ContextTarget::File(node.path.clone()) };
    rows.push(context_area(row_button, move |point| Message::ContextMenu(target.clone(), point)).into());

    if node.expanded {
        for child in node.children.iter().flatten().filter(|c| shown(c)) {
            push_rows(child, depth + 1, active, git, shown, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_keeps_matching_files_and_the_folders_holding_them() {
        let root = std::env::temp_dir().join(format!("beckett-explorer-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["player/sprites", "ui", ".godot"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["player/player.gd", "player/player.tscn", "player/sprites/idle.png", "ui/menu.tscn", "main.GD", "README", ".godot/cache.gd"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let mut explorer = Explorer::new(root.clone());
        let counts = explorer.index().as_ref().unwrap().extensions.clone();
        assert_eq!(counts, BTreeMap::from([("gd".into(), 2), ("png".into(), 1), ("tscn".into(), 2)]));

        explorer.update(FilterMessage::Only("gd".into()));
        let folders = explorer.index().as_ref().unwrap().folders.clone();
        assert_eq!(folders, HashSet::from([root.join("player")]));

        explorer.update(FilterMessage::Toggle("png".into()));
        assert!(explorer.index().as_ref().unwrap().folders.contains(&root.join("player/sprites")));
        explorer.update(FilterMessage::Clear);
        assert!(explorer.filter().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
