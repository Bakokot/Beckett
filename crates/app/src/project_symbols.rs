//! Go to symbol in project (Ctrl+T, `#` in the palette): the declarations
//! of every script, found by `editor_core::symbols` since Godot's language
//! server has no `workspace/symbol`. Open tabs are read from their text, so
//! unsaved edits count.

use std::path::{Path, PathBuf};

use editor_core::symbols::{declarations, DeclKind};
use lsp_client::{uri, SymbolKind};

use crate::{outline, palette::Choice, Message};

/// One choice per declaration, file by file in `scripts` order.
pub fn choices(root: &Path, scripts: &[PathBuf], open: &[(PathBuf, String)], syntax: &theme::SyntaxColors) -> Vec<Choice<Message>> {
    let mut out = Vec::new();
    for script in scripts {
        let text = match open.iter().find(|(path, _)| uri::same_path(path, script)) {
            Some((_, text)) => text.clone(),
            None => match std::fs::read_to_string(script) {
                Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
                Err(_) => continue,
            },
        };
        let relative = script.strip_prefix(root).unwrap_or(script).to_string_lossy().replace('\\', "/");
        for d in declarations(&text) {
            let (tag, _) = outline::tag(kind(d.kind), syntax);
            let place = if d.container.is_empty() { relative.clone() } else { format!("{} · {relative}", d.container) };
            out.push(Choice {
                label: d.name,
                detail: format!("{tag}  {place}:{}", d.line + 1),
                message: Message::JumpTo(script.clone(), d.line, d.column),
            });
        }
    }
    out
}

fn kind(kind: DeclKind) -> SymbolKind {
    match kind {
        DeclKind::Class => SymbolKind::Class,
        DeclKind::Function => SymbolKind::Function,
        DeclKind::Signal => SymbolKind::Signal,
        DeclKind::Constant => SymbolKind::Constant,
        DeclKind::Variable => SymbolKind::Variable,
        DeclKind::Enum => SymbolKind::Enum,
    }
}
