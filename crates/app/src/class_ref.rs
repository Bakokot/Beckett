//! The class reference (Shift+F1): Godot's engine classes with their
//! documentation, from the language server (the class list comes with the
//! handshake, a class's docs from `textDocument/nativeSymbol`). Shown in
//! place of the editor, like the shortcuts page.

use editor_core::fuzzy::fuzzy_match;
use iced::font::{Style as FontStyle, Weight};
use iced::widget::{button, column, container, rich_text, row, scrollable, space, span, text, text_input, Column};
use iced::{Alignment, Background, Border, Element, Font, Length};
use lsp_client::{NativeClass, NativeSymbol, SymbolKind};

use crate::style;

pub const FILTER: &str = "class-ref-filter";
pub const PAGE: &str = "class-ref-page";
/// Classes listed at once; typing narrows the list.
const LISTED: usize = 200;

/// Where a link in the docs goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Class(String),
    /// A member of a class (`[method Node.add_child]`).
    Member(String, String),
}

#[derive(Debug, Clone)]
pub enum ClassRefMessage {
    Filter(String),
    MemberFilter(String),
    Open(Link),
    Back,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStyle {
    Plain,
    Bold,
    Italic,
    Code,
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    text: String,
    style: RunStyle,
    link: Option<Link>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Paragraph(Vec<Run>),
    Code(String),
}

/// A member, its docs parsed once.
struct Member {
    name: String,
    kind: SymbolKind,
    /// The declaration without the class prefix (`func look_at(...)`).
    signature: String,
    deprecated: bool,
    docs: Vec<Block>,
}

pub struct Page {
    pub name: String,
    description: Vec<Block>,
    members: Vec<Member>,
}

impl Page {
    pub fn new(symbol: NativeSymbol) -> Self {
        let class = symbol.name.clone();
        let members = symbol
            .children
            .into_iter()
            .map(|m| Member {
                signature: m.detail.replacen(&format!("{class}."), "", 1),
                docs: parse(&m.documentation, &class),
                name: m.name,
                kind: m.kind,
                deprecated: m.deprecated,
            })
            .collect();
        Self { description: parse(&symbol.documentation, &class), members, name: class }
    }
}

#[derive(Default)]
pub struct ClassRef {
    pub filter: String,
    pub member_filter: String,
    pub page: Option<Page>,
    /// The class whose docs were asked for and haven't arrived yet.
    pub loading: Option<String>,
    /// Classes shown before this one, for Back.
    pub history: Vec<String>,
}

impl ClassRef {
    /// Classes matching the filter, best first (all, by name, when empty).
    fn matching<'a>(&self, classes: &'a [NativeClass]) -> Vec<&'a NativeClass> {
        let query = self.filter.trim();
        if query.is_empty() {
            return classes.iter().collect();
        }
        let mut scored: Vec<(i64, &NativeClass)> =
            classes.iter().filter_map(|c| fuzzy_match(query, &c.name).map(|m| (m.score, c))).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.len().cmp(&b.1.name.len())));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    pub fn view<'a>(&'a self, classes: &'a [NativeClass], theme: &theme::Theme) -> Element<'a, ClassRefMessage> {
        let ui = &theme.ui;
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let current = self.page.as_ref().map(|p| p.name.as_str());

        // Left: the class list.
        let matching = self.matching(classes);
        let highlight = accent.scale_alpha(0.25);
        let mut list = Column::with_children(matching.iter().take(LISTED).map(|class| {
            let selected = Some(class.name.as_str()) == current;
            button(text(class.name.as_str()).size(13))
                .on_press(ClassRefMessage::Open(Link::Class(class.name.clone())))
                .width(Length::Fill)
                .padding([2, 8])
                .style(move |theme: &iced::Theme, status| {
                    let base = button::text(theme, status);
                    if selected {
                        button::Style { background: Some(Background::Color(highlight)), border: Border { radius: 3.0.into(), ..Border::default() }, ..base }
                    } else {
                        base
                    }
                })
                .into()
        }));
        if classes.is_empty() {
            list = list.push(container(text("The class list comes from the language server: waiting for it to connect").size(12).color(muted)).padding([4, 8]));
        } else if matching.len() > LISTED {
            list = list.push(container(text(format!("{} more: type to narrow", matching.len() - LISTED)).size(11).color(muted)).padding([4, 8]));
        } else if matching.is_empty() {
            list = list.push(container(text("No matching class").size(12).color(muted)).padding([4, 8]));
        }
        let filter = text_input("Search classes", &self.filter).id(FILTER).on_input(ClassRefMessage::Filter).size(13).padding([5, 8]);
        let sidebar = column![filter, scrollable(list.width(Length::Fill)).height(Length::Fill)].spacing(6).width(240).padding(8);

        // Right: the class page.
        let page: Element<'a, ClassRefMessage> = match (&self.page, &self.loading) {
            (_, Some(class)) => container(text(format!("Loading {class}…")).color(muted)).padding(16).into(),
            (Some(page), None) => self.page_view(page, classes, theme),
            (None, None) => container(text("Pick a class on the left, or press F12 on an engine class or method in a script").color(muted)).padding(16).into(),
        };
        let border = style::to_color(ui.border);
        let divider = container(space()).width(1).height(Length::Fill).style(move |_| container::Style {
            background: Some(border.into()),
            ..container::Style::default()
        });
        row![sidebar, divider, page].into()
    }

    fn page_view<'a>(&'a self, page: &'a Page, classes: &'a [NativeClass], theme: &theme::Theme) -> Element<'a, ClassRefMessage> {
        let ui = &theme.ui;
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let link = |name: &str| {
            button(text(name.to_string()).size(13).color(accent))
                .on_press(ClassRefMessage::Open(Link::Class(name.to_string())))
                .padding(0)
                .style(button::text)
        };

        let mut header = row![].spacing(12).align_y(Alignment::Center);
        if !self.history.is_empty() {
            header = header.push(button(text("< Back").size(12)).on_press(ClassRefMessage::Back).style(button::secondary).padding([2, 8]));
        }
        header = header.push(text(page.name.as_str()).size(24).font(Font { weight: Weight::Bold, ..Font::default() }));
        let mut content = column![header].spacing(8);

        let ancestors = ancestors(&page.name, classes);
        if !ancestors.is_empty() {
            let mut line = row![text("Inherits:").size(13).color(muted)].spacing(6);
            for (i, name) in ancestors.iter().enumerate() {
                if i > 0 {
                    line = line.push(text("<").size(13).color(muted));
                }
                line = line.push(link(name));
            }
            content = content.push(line);
        }
        let heirs: Vec<&str> = classes.iter().filter(|c| c.inherits == page.name).map(|c| c.name.as_str()).collect();
        if !heirs.is_empty() {
            // Wrapped as text: a row of buttons can't wrap.
            let mut spans = vec![span("Inherited by: ").color(muted)];
            for (i, name) in heirs.iter().enumerate() {
                if i > 0 {
                    spans.push(span(", ").color(muted));
                }
                spans.push(span(*name).color(accent).link(Link::Class(name.to_string())));
            }
            content = content.push(rich_text(spans).size(13).on_link_click(ClassRefMessage::Open));
        }
        content = content.push(blocks_view(&page.description, theme));

        content = content.push(
            text_input("Filter members", &self.member_filter).on_input(ClassRefMessage::MemberFilter).size(13).padding([5, 8]).width(320),
        );
        let wanted = self.member_filter.trim().to_lowercase();
        let sections: [(&str, &[SymbolKind]); 4] = [
            ("Properties", &[SymbolKind::Variable]),
            ("Methods", &[SymbolKind::Function]),
            ("Signals", &[SymbolKind::Signal]),
            ("Constants and enums", &[SymbolKind::Constant, SymbolKind::Enum, SymbolKind::EnumMember, SymbolKind::Class, SymbolKind::Other]),
        ];
        let surface = style::to_color(ui.surface);
        let mut shown = 0;
        for (title, kinds) in sections {
            let members: Vec<&Member> = page
                .members
                .iter()
                .filter(|m| kinds.contains(&m.kind) && (wanted.is_empty() || m.name.to_lowercase().contains(&wanted)))
                .collect();
            if members.is_empty() {
                continue;
            }
            shown += members.len();
            content = content.push(container(text(title).size(17).color(accent)).padding(iced::Padding { top: 10.0, ..iced::Padding::ZERO }));
            for member in members {
                let mut signature = row![text(member.signature.as_str()).size(13).font(Font::MONOSPACE)].spacing(8);
                if member.deprecated {
                    signature = signature.push(text("deprecated").size(11).color(muted));
                }
                let mut entry = column![signature].spacing(4);
                if !member.docs.is_empty() {
                    entry = entry.push(container(blocks_view(&member.docs, theme)).padding(iced::Padding { left: 16.0, ..iced::Padding::ZERO }));
                }
                content = content.push(container(entry).width(Length::Fill).padding([6, 8]).style(move |_| container::Style {
                    background: Some(Background::Color(surface)),
                    border: Border { radius: 4.0.into(), ..Border::default() },
                    ..container::Style::default()
                }));
            }
        }
        if shown == 0 && !wanted.is_empty() {
            content = content.push(text(format!("No member of {} matches \"{}\"", page.name, self.member_filter.trim())).size(13).color(muted));
        }
        scrollable(content.padding(iced::Padding { top: 12.0, right: 24.0, bottom: 24.0, left: 16.0 }).width(Length::Fill))
            .id(PAGE)
            .height(Length::Fill)
            .into()
    }
}

fn blocks_view<'a>(blocks: &'a [Block], theme: &theme::Theme) -> Element<'a, ClassRefMessage> {
    let ui = &theme.ui;
    let (accent, text_color) = (style::to_color(ui.accent), style::to_color(ui.text));
    let code_background = style::to_color(ui.border).scale_alpha(0.5);
    let syntax_string = style::to_color(theme.syntax.string);
    Column::with_children(blocks.iter().map(|block| match block {
        Block::Paragraph(runs) => {
            let spans: Vec<_> = runs
                .iter()
                .map(|run| {
                    let piece = span(run.text.as_str());
                    let piece = match run.style {
                        RunStyle::Plain => piece,
                        RunStyle::Bold => piece.font(Font { weight: Weight::Bold, ..Font::default() }),
                        RunStyle::Italic => piece.font(Font { style: FontStyle::Italic, ..Font::default() }),
                        RunStyle::Code => piece.font(Font::MONOSPACE).color(syntax_string),
                    };
                    match &run.link {
                        Some(link) => piece.color(accent).link(link.clone()),
                        None => piece,
                    }
                })
                .collect();
            rich_text(spans).size(13).color(text_color).on_link_click(ClassRefMessage::Open).into()
        }
        Block::Code(code) => container(text(code.as_str()).size(12).font(Font::MONOSPACE))
            .width(Length::Fill)
            .padding([6, 10])
            .style(move |_| container::Style {
                background: Some(Background::Color(code_background)),
                border: Border { radius: 4.0.into(), ..Border::default() },
                ..container::Style::default()
            })
            .into(),
    }))
    .spacing(6)
    .into()
}

/// The parents of `class`, nearest first.
pub fn ancestors(class: &str, classes: &[NativeClass]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = class;
    while let Some(parent) = classes.iter().find(|c| c.name == current).map(|c| c.inherits.as_str()).filter(|p| !p.is_empty()) {
        if out.iter().any(|seen| seen == parent) {
            break;
        }
        out.push(parent.to_string());
        current = parent;
    }
    out
}

/// The engine class (and member) a hover describes, from its signature
/// line: `<Native> class Node2D …`, `var CharacterBody2D.velocity: …`,
/// `func @GlobalScope.print(...) -> void`. Only known engine classes count,
/// so a script's own symbols give `None`.
pub fn hover_target(hover: &str, classes: &[NativeClass]) -> Option<(String, Option<String>)> {
    let first = hover.lines().map(str::trim).find(|l| !l.is_empty())?;
    let known = |name: &str| classes.iter().any(|c| c.name == name);
    if let Some(rest) = first.strip_prefix("<Native> class ") {
        let name = rest.split_whitespace().next()?;
        return known(name).then(|| (name.to_string(), None));
    }
    let rest = first.strip_prefix("static ").unwrap_or(first);
    let (_, rest) = rest.split_once(' ').filter(|(keyword, _)| ["func", "var", "const", "signal", "enum"].contains(keyword))?;
    let path = rest.split(|c: char| c == '(' || c == ':' || c == ' ' || c == '=').next()?;
    let (class, member) = path.rsplit_once('.')?;
    known(class).then(|| (class.to_string(), Some(member.to_string())))
}

/// Godot's documentation BBCode as paragraphs and code blocks. References
/// (`[Node2D]`, `[method add_child]`, `[member Node.name]`) become links;
/// members without a class belong to `class`.
fn parse(bbcode: &str, class: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut style = RunStyle::Plain;
    let mut buffer = String::new();
    let mut rest = bbcode;

    fn flush(buffer: &mut String, runs: &mut Vec<Run>, style: RunStyle) {
        if !buffer.is_empty() {
            runs.push(Run { text: std::mem::take(buffer), style, link: None });
        }
    }
    fn end_paragraph(runs: &mut Vec<Run>, blocks: &mut Vec<Block>) {
        if runs.iter().any(|r| !r.text.trim().is_empty()) {
            blocks.push(Block::Paragraph(std::mem::take(runs)));
        }
        runs.clear();
    }

    while !rest.is_empty() {
        let Some(open) = rest.find(['[', '\n']) else {
            buffer.push_str(rest);
            break;
        };
        buffer.push_str(&rest[..open]);
        rest = &rest[open..];
        if rest.starts_with('\n') {
            flush(&mut buffer, &mut runs, style);
            end_paragraph(&mut runs, &mut blocks);
            rest = &rest[1..];
            continue;
        }
        let Some(close) = rest.find(']') else {
            buffer.push_str(rest);
            break;
        };
        let tag = &rest[1..close];
        let after = &rest[close + 1..];
        // Tags whose content is taken literally, up to the closing tag.
        let literal = |name: &str| -> Option<(String, &str)> {
            let end = format!("[/{name}]");
            let at = after.find(&end)?;
            Some((after[..at].to_string(), &after[at + end.len()..]))
        };
        let tag_name = tag.split([' ', '=']).next().unwrap_or("");
        match tag_name {
            "b" | "i" | "u" | "/b" | "/i" | "/u" => {
                flush(&mut buffer, &mut runs, style);
                style = match tag {
                    "b" => RunStyle::Bold,
                    "i" | "u" => RunStyle::Italic,
                    _ => RunStyle::Plain,
                };
                rest = after;
            }
            "code" | "kbd" => match literal(tag_name) {
                Some((code, next)) => {
                    flush(&mut buffer, &mut runs, style);
                    runs.push(Run { text: code, style: RunStyle::Code, link: None });
                    rest = next;
                }
                None => {
                    buffer.push('[');
                    rest = &rest[1..];
                }
            },
            "codeblock" | "codeblocks" => match literal(tag_name) {
                Some((code, next)) => {
                    flush(&mut buffer, &mut runs, style);
                    end_paragraph(&mut runs, &mut blocks);
                    // Several languages: GDScript only.
                    let code = match code.find("[gdscript") {
                        Some(start) => {
                            let body = &code[start..];
                            let body = &body[body.find(']').map_or(0, |i| i + 1)..];
                            body[..body.find("[/gdscript]").unwrap_or(body.len())].to_string()
                        }
                        None if code.contains("[csharp") => String::new(),
                        None => code,
                    };
                    let code = dedent(&code);
                    if !code.is_empty() {
                        blocks.push(Block::Code(code));
                    }
                    rest = next;
                }
                None => {
                    buffer.push('[');
                    rest = &rest[1..];
                }
            },
            "br" => {
                flush(&mut buffer, &mut runs, style);
                end_paragraph(&mut runs, &mut blocks);
                rest = after;
            }
            "param" => {
                flush(&mut buffer, &mut runs, style);
                runs.push(Run { text: tag[6..].trim().to_string(), style: RunStyle::Code, link: None });
                rest = after;
            }
            "method" | "member" | "signal" | "constant" | "enum" | "annotation" | "theme_item" | "constructor" | "operator" => {
                flush(&mut buffer, &mut runs, style);
                let target = tag[tag_name.len()..].trim();
                let (owner, name) = target.rsplit_once('.').unwrap_or((class, target));
                let shown = if tag_name == "method" { format!("{target}()") } else { target.to_string() };
                runs.push(Run { text: shown, style: RunStyle::Code, link: Some(Link::Member(owner.to_string(), name.to_string())) });
                rest = after;
            }
            // Formatting without an equivalent here: drop the tag, keep the text.
            "url" | "/url" | "color" | "/color" | "center" | "/center" | "font" | "/font" | "lb" | "rb" => {
                if tag == "lb" || tag == "rb" {
                    buffer.push(if tag == "lb" { '[' } else { ']' });
                }
                rest = after;
            }
            _ if is_class_name(tag) => {
                flush(&mut buffer, &mut runs, style);
                runs.push(Run { text: tag.to_string(), style: RunStyle::Plain, link: Some(Link::Class(tag.to_string())) });
                rest = after;
            }
            _ => {
                buffer.push('[');
                rest = &rest[1..];
            }
        }
    }
    flush(&mut buffer, &mut runs, style);
    end_paragraph(&mut runs, &mut blocks);
    blocks
}

/// `Node2D`, `@GlobalScope`, `int`: a single identifier.
fn is_class_name(tag: &str) -> bool {
    let name = tag.strip_prefix('@').unwrap_or(tag);
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// Drops the indentation shared by a code block's lines, and blank edges.
fn dedent(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(start, |i| i + 1);
    let lines = &lines[start..end];
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines.iter().map(|l| l.get(common..).unwrap_or("").replace('\t', "    ")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, inherits: &str) -> NativeClass {
        NativeClass { name: name.into(), inherits: inherits.into() }
    }

    #[test]
    fn parses_bbcode_into_runs_links_and_code() {
        let blocks = parse(
            "Moves [b]fast[/b]. See [member position] and [method Node.add_child], [Vector2], [param delta], [code]a[0][/code].\n[codeblocks]\n[gdscript]\n\tvar x = 1\n\tprint(x)\n[/gdscript]\n[csharp]\nint x;\n[/csharp]\n[/codeblocks]\nDone [unknown tag].",
            "Node2D",
        );
        assert_eq!(blocks.len(), 3);
        let Block::Paragraph(runs) = &blocks[0] else { panic!() };
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["Moves ", "fast", ". See ", "position", " and ", "Node.add_child()", ", ", "Vector2", ", ", "delta", ", ", "a[0]", "."]);
        assert_eq!(runs[1].style, RunStyle::Bold);
        assert_eq!(runs[3].link, Some(Link::Member("Node2D".into(), "position".into())));
        assert_eq!(runs[5].link, Some(Link::Member("Node".into(), "add_child".into())));
        assert_eq!(runs[7].link, Some(Link::Class("Vector2".into())));
        assert_eq!(blocks[1], Block::Code("var x = 1\nprint(x)".into()));
        let Block::Paragraph(last) = &blocks[2] else { panic!() };
        assert_eq!(last[0].text, "Done [unknown tag].");
    }

    #[test]
    fn finds_hover_targets_of_engine_symbols_only() {
        let classes = [class("CharacterBody2D", "PhysicsBody2D"), class("@GlobalScope", "")];
        let target = |hover: &str| hover_target(hover, &classes);
        assert_eq!(target("\t<Native> class CharacterBody2D extends PhysicsBody2D\n\nA body."), Some(("CharacterBody2D".into(), None)));
        assert_eq!(target("\tvar CharacterBody2D.velocity: Vector2\n\nDocs"), Some(("CharacterBody2D".into(), Some("velocity".into()))));
        assert_eq!(target("\tfunc @GlobalScope.print(...) -> void"), Some(("@GlobalScope".into(), Some("print".into()))));
        assert_eq!(target("\tvar Player.speed: float"), None);
        assert_eq!(target("\tfunc jump() -> void"), None);
    }

    #[test]
    fn walks_ancestors_and_ranks_classes() {
        let classes = [class("Node", "Object"), class("Node2D", "CanvasItem"), class("CanvasItem", "Node"), class("Object", "")];
        assert_eq!(ancestors("Node2D", &classes), ["CanvasItem", "Node", "Object"]);
        let page = ClassRef { filter: "node".into(), ..ClassRef::default() };
        let names: Vec<&str> = page.matching(&classes).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names[..2], ["Node", "Node2D"]);
    }
}
