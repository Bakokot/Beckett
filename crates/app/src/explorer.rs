//! Project file tree with lazily loaded directories.

use std::path::{Path, PathBuf};

use iced::widget::{button, row, svg, text, Column, Space};
use iced::{Element, Length};

use crate::context_menu::context_area;
use crate::drag::drag_source;
use crate::file_icons::Icon;
use crate::{ContextTarget, Message};

pub struct Explorer {
    root: Node,
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
        Self { root }
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

    pub fn view(&self, active: Option<&Path>) -> Element<'_, Message> {
        let mut rows = Vec::new();
        for child in self.root.children.iter().flatten() {
            push_rows(child, 0, active, &mut rows);
        }
        Column::with_children(rows).width(Length::Fill).into()
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

    let row_button = button(
        row![
            Space::new().width(f32::from(depth) * 14.0),
            arrow,
            svg(icon).width(16).height(16),
            text(&node.name).size(13),
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
        for child in node.children.iter().flatten() {
            push_rows(child, depth + 1, active, rows);
        }
    }
}
