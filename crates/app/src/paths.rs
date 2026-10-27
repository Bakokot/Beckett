//! `res://` / `uid://` paths inside GDScript strings: finding the string
//! under the cursor, when to complete paths, and ranking them.

use editor_core::fuzzy::fuzzy_match;
use editor_core::gdscript::TokenKind;
use editor_core::{Buffer, Language, Pos};

/// A string literal around a char index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringAt {
    /// Char index of the first char inside the quotes.
    pub content_start: usize,
    /// Char index of the closing quote (or the line end when unclosed).
    pub content_end: usize,
    /// The code on the line before the string.
    pub before: String,
}

impl StringAt {
    pub fn content(&self, buffer: &Buffer) -> String {
        buffer.slice(self.content_start, self.content_end)
    }
}

/// The string literal `index` is inside (between its quotes), if any.
pub fn string_at(buffer: &Buffer, index: usize) -> Option<StringAt> {
    let pos = buffer.char_to_pos(index);
    let line = buffer.line_text(pos.line);
    let line = line.trim_end_matches(['\n', '\r']);
    let byte = line.char_indices().nth(pos.column).map_or(line.len(), |(b, _)| b);
    let column = |b: usize| line[..b].chars().count();
    let line_start = buffer.pos_to_char(Pos { line: pos.line, column: 0 });
    if buffer.language() != Language::GDScript {
        // Scenes, resources, project.godot: plain `"…"` pairs, no escapes.
        let quotes: Vec<usize> = line.match_indices('"').map(|(b, _)| b).collect();
        let (open, close) = quotes.as_chunks::<2>().0.iter().map(|&[o, c]| (o, c)).find(|&(o, c)| o < byte && byte <= c)?;
        return Some(StringAt {
            content_start: line_start + column(open + 1),
            content_end: line_start + column(close),
            before: line[..open].to_string(),
        });
    }
    let token = buffer
        .tokens(pos.line)
        .into_iter()
        .find(|t| t.kind == TokenKind::String && t.range.start < byte && byte <= t.range.end)?;
    let text = &line[token.range.clone()];
    // Skip a raw-string `r` prefix, then the opening quote(s).
    let prefix = text.find(['"', '\''])?;
    let quote = &text[prefix..prefix + 1];
    let quotes = if text[prefix..].starts_with(&quote.repeat(3)) { 3 } else { 1 };
    let open_end = token.range.start + prefix + quotes;
    let closing = quote.repeat(quotes);
    let closed = text.len() >= prefix + 2 * quotes && text.ends_with(&closing);
    let close_start = if closed { token.range.end - quotes } else { token.range.end };
    if byte < open_end || byte > close_start {
        return None;
    }
    Some(StringAt {
        content_start: line_start + column(open_end),
        content_end: line_start + column(close_start),
        before: line[..token.range.start].to_string(),
    })
}

/// Whether the char at `index` is in a comment or a plain string: prose,
/// where a word like `a` isn't a symbol (the language server would still
/// match it to some engine member). StringNames and NodePaths count as code.
pub fn in_prose(buffer: &Buffer, index: usize) -> bool {
    let pos = buffer.char_to_pos(index);
    let line = buffer.line_text(pos.line);
    let byte = line.char_indices().nth(pos.column).map_or(line.len(), |(b, _)| b);
    buffer
        .tokens(pos.line)
        .iter()
        .any(|t| matches!(t.kind, TokenKind::Comment | TokenKind::String) && t.range.start <= byte && byte < t.range.end)
}

/// The `res://` or `uid://` path in the string under `index`, without a
/// `::sub_resource` suffix.
pub fn link_at(buffer: &Buffer, index: usize) -> Option<String> {
    let content = string_at(buffer, index)?.content(buffer);
    let path = content.split("::").next().unwrap_or_default().trim();
    (path.starts_with("res://") || path.starts_with("uid://")).then(|| path.to_string())
}

/// Where `res://` path completion should start for a cursor at `head`:
/// inside a string already starting with `res://`, or one being typed as
/// the argument of `load(` / `preload(`.
pub fn path_completion_anchor(buffer: &Buffer, head: usize) -> Option<usize> {
    let string = string_at(buffer, head)?;
    let typed = buffer.slice(string.content_start, head);
    let in_load = string.before.trim_end().ends_with("load(");
    let wanted = typed.starts_with("res://") || (in_load && "res://".starts_with(typed.as_str()));
    wanted.then_some(string.content_start)
}

/// What dropping the file at `res_path` at char `index` inserts: the bare
/// path inside a string or outside GDScript, else `preload("…")`, and with
/// Ctrl a constant holding it (as in the Godot editor).
pub fn drop_text(buffer: &Buffer, index: usize, res_path: &str, ctrl: bool) -> String {
    if buffer.language() != Language::GDScript || string_at(buffer, index).is_some() {
        return res_path.to_string();
    }
    let preload = format!("preload(\"{res_path}\")");
    if !ctrl {
        return preload;
    }
    let file = res_path.rsplit('/').next().unwrap_or(res_path);
    let stem = file.split('.').next().unwrap_or(file);
    format!("const {} = {preload}", constant_case(stem))
}

/// `BigEnemy`, `big-enemy`, `big enemy` → `BIG_ENEMY`.
fn constant_case(name: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<char> = None;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            previous = None;
            continue;
        }
        let boundary = match previous {
            None => !out.is_empty(),
            Some(p) => c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit()),
        };
        if boundary {
            out.push('_');
        }
        out.extend(c.to_uppercase());
        previous = Some(c);
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Ranks `res://` paths against what was typed in the string: fuzzy on the
/// part after `res://`, best first, ties in path order.
pub fn rank_paths(paths: &[String], typed: &str) -> Vec<usize> {
    let needle = typed.strip_prefix("res://").unwrap_or(if "res://".starts_with(typed) { "" } else { typed });
    let mut scored: Vec<(i64, usize)> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let haystack = path.strip_prefix("res://").unwrap_or(path);
            fuzzy_match(needle, haystack).map(|m| (m.score, index))
        })
        .collect();
    scored.sort_by_key(|&(score, index)| (-score, index));
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str) -> Buffer {
        Buffer::new(text, Language::GDScript)
    }

    /// Char index of the `|` marker, with the marker removed.
    fn at(text: &str) -> (Buffer, usize) {
        let index = text.chars().position(|c| c == '|').unwrap();
        (buffer(&text.replace('|', "")), index)
    }

    #[test]
    fn finds_the_string_around_the_cursor() {
        let (b, i) = at("var s = load(\"res://pl|ayer.gd\")\n");
        let s = string_at(&b, i).unwrap();
        assert_eq!(s.content(&b), "res://player.gd");
        assert_eq!(s.before, "var s = load(");

        // Just before the opening quote and just after the closing one: outside.
        let (b, i) = at("x = |\"abc\"\n");
        assert_eq!(string_at(&b, i), None);
        let (b, i) = at("x = \"abc\"|\n");
        assert_eq!(string_at(&b, i), None);
        // Right after the opening quote and at the closing one: inside.
        let (b, i) = at("x = \"|abc\"\n");
        assert!(string_at(&b, i).is_some());
        let (b, i) = at("x = \"abc|\"\n");
        assert!(string_at(&b, i).is_some());
        // Unclosed string while typing.
        let (b, i) = at("\tpreload(\"res://sc|\n");
        assert_eq!(string_at(&b, i).unwrap().content(&b), "res://sc");
        // Comments and node paths are not strings.
        let (b, i) = at("# \"res://|a.gd\"\n");
        assert_eq!(string_at(&b, i), None);
    }

    #[test]
    fn comments_and_strings_are_prose() {
        let (b, i) = at("var x = 1 # a lo|ng note\n");
        assert!(in_prose(&b, i));
        let (b, i) = at("print(\"a wo|rd\")\n");
        assert!(in_prose(&b, i));
        let (b, i) = at("var a: Vec|tor2\n");
        assert!(!in_prose(&b, i));
        let (b, i) = at("get_node(^\"Pla|yer\")\n");
        assert!(!in_prose(&b, i));
        // The code before a comment on the same line is still code.
        let (b, i) = at("var spe|ed = 2 # fast\n");
        assert!(!in_prose(&b, i));
    }

    #[test]
    fn links_strip_sub_resources() {
        let (b, i) = at("const S = preload(\"res://|ui/theme.tres::3\")\n");
        assert_eq!(link_at(&b, i).as_deref(), Some("res://ui/theme.tres"));
        let (b, i) = at("var u = \"uid://b|x7\"\n");
        assert_eq!(link_at(&b, i).as_deref(), Some("uid://bx7"));
        let (b, i) = at("print(\"hel|lo\")\n");
        assert_eq!(link_at(&b, i), None);

        let scene = "[ext_resource type=\"Script\" path=\"res://pl|ayer.gd\" id=\"1\"]\n";
        let index = scene.chars().position(|c| c == '|').unwrap();
        let b = Buffer::new(&scene.replace('|', ""), Language::Plain);
        assert_eq!(link_at(&b, index).as_deref(), Some("res://player.gd"));
    }

    #[test]
    fn path_completion_only_where_paths_go() {
        let (b, i) = at("var x = \"res://|\"\n");
        assert_eq!(path_completion_anchor(&b, i), Some(9));
        let (b, i) = at("var x = preload(\"re|\")\n");
        assert_eq!(path_completion_anchor(&b, i), Some(17));
        let (b, i) = at("var x = load(\"|\")\n");
        assert!(path_completion_anchor(&b, i).is_some());
        let (b, i) = at("print(\"re|\")\n");
        assert_eq!(path_completion_anchor(&b, i), None);
    }

    #[test]
    fn dropped_files_become_preloads() {
        let (b, i) = at("var s = |\n");
        assert_eq!(drop_text(&b, i, "res://enemies/BigEnemy.tscn", false), "preload(\"res://enemies/BigEnemy.tscn\")");
        assert_eq!(
            drop_text(&b, i, "res://enemies/BigEnemy.tscn", true),
            "const BIG_ENEMY = preload(\"res://enemies/BigEnemy.tscn\")"
        );
        let (b, i) = at("var s = load(\"|\")\n");
        assert_eq!(drop_text(&b, i, "res://a.png", true), "res://a.png");
        let scene = Buffer::new("[node]\n", Language::Plain);
        assert_eq!(drop_text(&scene, 0, "res://a.gd", false), "res://a.gd");

        assert_eq!(constant_case("ui-theme"), "UI_THEME");
        assert_eq!(constant_case("level2Boss"), "LEVEL2_BOSS");
        assert_eq!(constant_case("3d model"), "_3D_MODEL");
        assert_eq!(constant_case("player"), "PLAYER");
    }

    #[test]
    fn ranks_by_file_name() {
        let paths: Vec<String> = ["res://addons/x/plugin.gd", "res://player/player.gd", "res://scenes/main.tscn"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(rank_paths(&paths, "res://"), [0, 1, 2]);
        assert_eq!(rank_paths(&paths, "re"), [0, 1, 2]);
        assert_eq!(rank_paths(&paths, "res://main")[0], 2);
        assert_eq!(rank_paths(&paths, "res://player")[0], 1);
    }
}
