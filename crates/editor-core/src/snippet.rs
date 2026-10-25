//! GDScript snippets and their expansion.
//!
//! Bodies use a small subset of the VS Code / TextMate syntax: `$1` or
//! `${1:default}` tab stops visited in order, `$0` for the final cursor
//! (the end when absent), `\$` for a literal dollar sign and `\t` for one
//! indent level. Continuation lines get the indentation of the line the
//! snippet is inserted on.

pub struct Snippet {
    /// What is typed to find it, e.g. `_ready`.
    pub prefix: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

/// Inserted text and its tab stops, as char ranges relative to the start
/// of the text, in visiting order; the last one is the final cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub text: String,
    pub stops: Vec<(usize, usize)>,
}

const fn snippet(prefix: &'static str, description: &'static str, body: &'static str) -> Snippet {
    Snippet { prefix, description, body }
}

pub const GDSCRIPT: &[Snippet] = &[
    snippet("_ready", "Called when the node enters the scene tree for the first time", "func _ready() -> void:\n\t${0:pass}"),
    snippet("_process", "Called every frame", "func _process(delta: float) -> void:\n\t${0:pass}"),
    snippet("_physics_process", "Called every physics tick", "func _physics_process(delta: float) -> void:\n\t${0:pass}"),
    snippet("_input", "Called for every input event", "func _input(event: InputEvent) -> void:\n\t${0:pass}"),
    snippet(
        "_unhandled_input",
        "Called for input events nothing else consumed",
        "func _unhandled_input(event: InputEvent) -> void:\n\t${0:pass}",
    ),
    snippet("_init", "Constructor", "func _init() -> void:\n\t${0:pass}"),
    snippet("_enter_tree", "Called when the node enters the scene tree", "func _enter_tree() -> void:\n\t${0:pass}"),
    snippet("_exit_tree", "Called when the node leaves the scene tree", "func _exit_tree() -> void:\n\t${0:pass}"),
    snippet("func", "Function", "func ${1:name}(${2}) -> ${3:void}:\n\t${0:pass}"),
    snippet("static func", "Static function", "static func ${1:name}(${2}) -> ${3:void}:\n\t${0:pass}"),
    snippet("export", "Exported property, editable in the Inspector", "@export var ${1:name}: ${2:float} = ${0:0.0}"),
    snippet("onready", "Node reference resolved when ready", "@onready var ${1:name}: ${2:Node} = \\$${0:Path}"),
    snippet("signal", "Signal declaration", "signal ${1:name}(${0})"),
    snippet("connect", "Connect a signal to a method", "${1:signal_name}.connect(${0:_on_signal})"),
    snippet("emit", "Emit a signal", "${1:signal_name}.emit(${0})"),
    snippet("const", "Constant", "const ${1:NAME} = ${0:value}"),
    snippet("enum", "Enumeration", "enum ${1:Name} { ${0:A, B} }"),
    snippet("class_name", "Global class name and base class", "class_name ${1:Name}\nextends ${0:Node}"),
    snippet("class", "Inner class", "class ${1:Name}:\n\t${0:pass}"),
    snippet("if", "If statement", "if ${1:condition}:\n\t${0:pass}"),
    snippet("ifelse", "If / else statement", "if ${1:condition}:\n\t${2:pass}\nelse:\n\t${0:pass}"),
    snippet("elif", "Else-if branch", "elif ${1:condition}:\n\t${0:pass}"),
    snippet("for", "Loop over a range or collection", "for ${1:i} in ${2:range(10)}:\n\t${0:pass}"),
    snippet("while", "While loop", "while ${1:condition}:\n\t${0:pass}"),
    snippet("match", "Match statement", "match ${1:value}:\n\t${2:pattern}:\n\t\t${0:pass}"),
    snippet("await timer", "Wait a number of seconds", "await get_tree().create_timer(${1:1.0}).timeout$0"),
    snippet("preload", "Load a resource at compile time", "preload(\"res://${1}\")$0"),
    snippet("tween", "Animate a property", "var tween := create_tween()\ntween.tween_property(${1:self}, \"${2:position}\", ${3:Vector2.ZERO}, ${4:0.5})$0"),
    snippet("region", "Foldable region", "#region ${1:Name}\n$0\n#endregion"),
    snippet("tool", "Run this script in the editor", "@tool"),
];

/// Expands `body` for insertion on a line indented with `indent`.
pub fn expand(body: &str, indent: &str) -> Expansion {
    let mut text = String::new();
    let mut len = 0; // chars in `text`
    let mut stops: Vec<(u32, usize, usize)> = Vec::new();
    let push = |text: &mut String, len: &mut usize, c: char| {
        text.push(c);
        *len += 1;
        if c == '\n' {
            for i in indent.chars() {
                text.push(i);
                *len += 1;
            }
        }
    };

    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1) == Some(&'$') {
            push(&mut text, &mut len, '$');
            i += 2;
            continue;
        }
        if c == '$' {
            if let Some((number, default, next)) = tab_stop(&chars, i + 1) {
                let start = len;
                for d in default.chars() {
                    push(&mut text, &mut len, d);
                }
                stops.push((number, start, len));
                i = next;
                continue;
            }
        }
        push(&mut text, &mut len, c);
        i += 1;
    }

    // Visit 1, 2, … then 0 (the final cursor, at the end when absent).
    stops.sort_by_key(|&(number, start, _)| (number == 0, number, start));
    if stops.last().is_none_or(|&(number, _, _)| number != 0) {
        stops.push((0, len, len));
    }
    Expansion { text, stops: stops.into_iter().map(|(_, start, end)| (start, end)).collect() }
}

/// Parses `1` or `{1:default}` at `chars[at..]`: (number, default, index after).
fn tab_stop(chars: &[char], at: usize) -> Option<(u32, String, usize)> {
    let digits = |from: usize| chars[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    if chars.get(at) == Some(&'{') {
        let n = digits(at + 1);
        let number = chars[at + 1..at + 1 + n].iter().collect::<String>().parse().ok()?;
        let mut i = at + 1 + n;
        let mut default = String::new();
        if chars.get(i) == Some(&':') {
            i += 1;
            while *chars.get(i)? != '}' {
                default.push(chars[i]);
                i += 1;
            }
        }
        (chars.get(i) == Some(&'}')).then_some((number, default, i + 1))
    } else {
        let n = digits(at);
        let number = chars[at..at + n].iter().collect::<String>().parse().ok()?;
        Some((number, String::new(), at + n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slices(e: &Expansion) -> Vec<String> {
        let chars: Vec<char> = e.text.chars().collect();
        e.stops.iter().map(|&(s, end)| chars[s..end].iter().collect()).collect()
    }

    #[test]
    fn expands_stops_in_order_with_final_cursor_last() {
        let e = expand("func ${1:name}(${2}) -> ${3:void}:\n\t${0:pass}", "\t");
        assert_eq!(e.text, "func name() -> void:\n\t\tpass");
        assert_eq!(slices(&e), ["name", "", "void", "pass"]);
        assert_eq!(e.stops[1], (10, 10));
    }

    #[test]
    fn adds_final_cursor_and_handles_escapes() {
        let e = expand("@onready var ${1:x} = \\$${0:Path}", "");
        assert_eq!(e.text, "@onready var x = $Path");
        assert_eq!(slices(&e), ["x", "Path"]);

        let e = expand("signal $1", "");
        assert_eq!(e.text, "signal ");
        assert_eq!(e.stops, [(7, 7), (7, 7)]);

        let e = expand("costs $ 5", "");
        assert_eq!((e.text.as_str(), e.stops.as_slice()), ("costs $ 5", &[(9, 9)][..]));
    }

    #[test]
    fn every_builtin_snippet_expands() {
        for s in GDSCRIPT {
            let e = expand(s.body, "    ");
            assert!(!e.text.contains("${"), "{}: {}", s.prefix, e.text);
            let len = e.text.chars().count();
            assert!(e.stops.iter().all(|&(a, b)| a <= b && b <= len), "{}", s.prefix);
        }
    }
}
