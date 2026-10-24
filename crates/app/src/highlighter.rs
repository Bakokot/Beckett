//! Bridges the GDScript lexer to iced's `text_editor` highlighting.

use std::ops::Range;

use editor_core::gdscript::{self, LineState, TokenKind};
use iced::advanced::text::highlighter::{self, Format};
use iced::{Color, Font, Theme};

use crate::style::to_color;

/// Highlighter settings: the active theme's syntax palette. Changing it
/// re-highlights the whole document.
pub type Settings = theme::SyntaxColors;

pub struct GdHighlighter {
    colors: Settings,
    /// Lexer state at the start of each line up to `current_line`.
    states: Vec<LineState>,
    current_line: usize,
}

impl highlighter::Highlighter for GdHighlighter {
    type Settings = Settings;
    type Highlight = Color;
    type Iterator<'a>
        = std::vec::IntoIter<(Range<usize>, Color)>
    where
        Self: 'a;

    fn new(settings: &Self::Settings) -> Self {
        Self {
            colors: *settings,
            states: vec![LineState::Normal],
            current_line: 0,
        }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.colors = *new_settings;
        self.change_line(0);
    }

    fn change_line(&mut self, line: usize) {
        let line = line.min(self.states.len() - 1);
        self.states.truncate(line + 1);
        self.current_line = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let state = self.states[self.current_line];
        let (tokens, next) = gdscript::lex_line(line, state);
        self.states.push(next);
        self.current_line += 1;

        tokens
            .into_iter()
            .map(|token| (token.range, to_color(color_for(&self.colors, token.kind))))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

pub fn to_format(color: &Color, _theme: &Theme) -> Format<Font> {
    Format {
        color: Some(*color),
        font: None,
    }
}

fn color_for(colors: &Settings, kind: TokenKind) -> theme::Color {
    match kind {
        TokenKind::Keyword => colors.keyword,
        TokenKind::ControlFlow => colors.control_flow,
        TokenKind::Type => colors.r#type,
        TokenKind::Function => colors.function,
        TokenKind::Annotation => colors.annotation,
        TokenKind::String => colors.string,
        TokenKind::StringName => colors.string_name,
        TokenKind::NodePath => colors.node_path,
        TokenKind::Number => colors.number,
        TokenKind::Comment => colors.comment,
        TokenKind::Constant => colors.constant,
        TokenKind::Symbol => colors.symbol,
    }
}
