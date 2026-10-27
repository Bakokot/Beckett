//! The CHANGES panel: every file changed since the staged text, with its
//! hunks (removed lines red, added lines green, a little context), a
//! Revert button per hunk, and lines that jump to the code when clicked.

use std::borrow::Cow;
use std::path::Path;

use editor_core::diff::Hunk;
use iced::widget::tooltip::Position;
use iced::widget::{button, container, row, scrollable, space, text, Column};
use iced::{Alignment, Background, Color, Element, Font, Length};

use crate::git::FileStatus;
use crate::{style, Message};

/// Lines of unchanged code shown around each hunk.
const CONTEXT: usize = 3;
/// Rows drawn at most, so a huge diff can't stall the panel.
const MAX_ROWS: usize = 3000;

/// One changed file, ready to show.
pub struct FileView<'a> {
    pub path: &'a Path,
    pub label: String,
    pub status: FileStatus,
    /// The staged text (`None`: a new file).
    pub old: Option<&'a str>,
    /// The current text (the tab's, unsaved edits included), if read.
    pub new: Option<Cow<'a, str>>,
    pub hunks: Cow<'a, [Hunk]>,
    /// Open in a tab with unsaved edits.
    pub unsaved: bool,
}

pub struct Look {
    pub text: Color,
    pub muted: Color,
    pub added: Color,
    pub removed: Color,
    pub modified: Color,
    pub conflicted: Color,
}

pub fn view<'a>(files: Vec<FileView<'a>>, look: &Look, ui: &theme::UiColors) -> Element<'a, Message> {
    if files.is_empty() {
        let note = "No changes since the staged version (or the project isn't in a git repository)";
        return container(text(note).size(12).color(look.muted)).padding([6, 12]).into();
    }
    let mut items: Vec<Element<'a, Message>> = Vec::new();
    let mut rows = 0;
    for file in files {
        items.push(file_header(&file, look, ui));
        if rows >= MAX_ROWS {
            continue;
        }
        let (Some(old), Some(new)) = (file.old, file.new.as_deref()) else {
            let note = match file.status {
                FileStatus::Added => "New file",
                _ => "Too large to show, or unreadable",
            };
            items.push(container(text(note).size(12).color(look.muted)).padding([0, 28]).into());
            continue;
        };
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();
        for hunk in file.hunks.iter() {
            items.push(hunk_header(file.path, hunk, look, ui));
            let before = hunk.new.start.saturating_sub(CONTEXT)..hunk.new.start;
            let after = hunk.new.end..(hunk.new.end + CONTEXT).min(new_lines.len());
            for line in before {
                items.push(code_row(file.path, line, Some(line), ' ', new_lines[line], None, look));
            }
            for line in hunk.old.clone() {
                let content = old_lines.get(line).copied().unwrap_or_default();
                items.push(code_row(file.path, hunk.new.start, None, '-', content, Some(look.removed), look));
            }
            for line in hunk.new.clone() {
                let content = new_lines.get(line).copied().unwrap_or_default();
                items.push(code_row(file.path, line, Some(line), '+', content, Some(look.added), look));
            }
            for line in after {
                items.push(code_row(file.path, line, Some(line), ' ', new_lines[line], None, look));
            }
            rows += hunk.old.len() + hunk.new.len() + 2 * CONTEXT + 1;
            if rows >= MAX_ROWS {
                items.push(container(text("More changes not shown").size(12).color(look.muted)).padding([2, 28]).into());
                break;
            }
        }
    }
    scrollable(Column::with_children(items).width(Length::Fill).padding(iced::Padding { bottom: 8.0, ..Default::default() }))
        .height(Length::Fill)
        .into()
}

fn file_header<'a>(file: &FileView<'a>, look: &Look, ui: &theme::UiColors) -> Element<'a, Message> {
    let (tag, color) = match file.status {
        FileStatus::Modified => ("M", look.modified),
        FileStatus::Added => ("A", look.added),
        FileStatus::Conflicted => ("!", look.conflicted),
    };
    let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut content = row![
        text(tag).size(12).font(Font::MONOSPACE).color(color).width(14),
        text(name).size(13).color(color),
        text(file.label.clone()).size(11).color(look.muted),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if file.unsaved {
        content = content.push(text("(unsaved edits)").size(11).color(look.muted));
    }
    let open = button(content).on_press(Message::OpenFile(file.path.to_path_buf())).style(button::text).padding([3, 8]);
    style::tip(open, "Open the file", Position::Top, ui)
}

fn hunk_header<'a>(path: &Path, hunk: &Hunk, look: &Look, ui: &theme::UiColors) -> Element<'a, Message> {
    let lines = match (hunk.old.len(), hunk.new.len()) {
        (_, 0) => format!("{} removed before line {}", plural(hunk.old.len()), hunk.new.start + 1),
        (0, n) => format!("{} added at line {}", plural(n), hunk.new.start + 1),
        (_, _) => format!("Lines {}-{} changed", hunk.new.start + 1, hunk.new.end),
    };
    let revert = button(text("revert").size(11))
        .on_press(Message::RevertHunk(path.to_path_buf(), hunk.clone()))
        .style(button::text)
        .padding([0, 6]);
    row![
        text(lines).size(11).color(look.muted),
        space::horizontal(),
        style::tip(revert, "Put back the staged lines (in the editor, unsaved; Ctrl+Z undoes it)", Position::Left, ui),
    ]
    .align_y(Alignment::Center)
    .padding(iced::Padding { top: 4.0, bottom: 1.0, left: 28.0, right: 8.0 })
    .into()
}

fn plural(n: usize) -> String {
    if n == 1 { "1 line".to_string() } else { format!("{n} lines") }
}

/// A line of code: its number in the current text (none for removed
/// lines), a `+` / `-` mark and a tint. Clicking jumps to `target`.
fn code_row<'a>(
    path: &Path,
    target: usize,
    number: Option<usize>,
    mark: char,
    content: &str,
    tint: Option<Color>,
    look: &Look,
) -> Element<'a, Message> {
    let number = number.map(|n| (n + 1).to_string()).unwrap_or_default();
    let color = tint.unwrap_or(look.text);
    let content = row![
        text(number).size(12).font(Font::MONOSPACE).color(look.muted).width(48).align_x(iced::alignment::Horizontal::Right),
        text(mark.to_string()).size(12).font(Font::MONOSPACE).color(color).width(14),
        text(content.replace('\t', "    ")).size(12).font(Font::MONOSPACE).color(if tint.is_some() { color } else { look.muted }),
    ]
    .spacing(6);
    let background = tint.map(|c| c.scale_alpha(0.12));
    button(content)
        .on_press(Message::JumpTo(path.to_path_buf(), target as u32, 0))
        .width(Length::Fill)
        .padding([0, 8])
        .style(move |theme: &iced::Theme, status| {
            let base = button::text(theme, status);
            match (background, status) {
                (_, button::Status::Hovered) => base,
                (Some(tint), _) => button::Style { background: Some(Background::Color(tint)), ..base },
                (None, _) => base,
            }
        })
        .into()
}
