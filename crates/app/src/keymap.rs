//! Customizable shortcuts: the commands that can be bound, their default
//! keys, and the user's changes kept in `<config>/keybindings.json` (only
//! what differs from the defaults, so new defaults still reach them).

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use iced::keyboard::{key::Named, Key, Modifiers};

/// A key with modifiers, e.g. Ctrl+Shift+P. Character keys are kept
/// lowercase; Shift is a separate flag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl Chord {
    /// Parses "Ctrl+Shift+P", "F5", "Alt+Up", "Ctrl+\\"…
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Chord { ctrl: false, shift: false, alt: false, key: String::new() };
        let text = text.trim();
        // The key itself may be "+" ("Ctrl++").
        let (mods, key) = match text.strip_suffix("++") {
            Some(mods) => (mods, "+"),
            None => text.rsplit_once('+').unwrap_or(("", text)),
        };
        for part in mods.split('+').filter(|p| !p.is_empty()) {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" | "option" => chord.alt = true,
                _ => return None,
            }
        }
        chord.key = normalize(key.trim())?;
        Some(chord)
    }

    /// The chord of a key press, if it is one shortcuts can use: with Ctrl
    /// or Alt, or a function key. Plain typing (and Ctrl+Alt+character,
    /// which is AltGr on many layouts) is not a chord.
    pub fn from_key(key: &Key, modifiers: Modifiers) -> Option<Self> {
        let (ctrl, alt, shift) = (modifiers.control(), modifiers.alt(), modifiers.shift());
        let name = match key.as_ref() {
            Key::Character(c) => {
                if ctrl && alt {
                    return None;
                }
                c.to_lowercase()
            }
            Key::Named(named) => named_key(named)?.to_string(),
            _ => return None,
        };
        let function_key = name.len() > 1 && name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit());
        (ctrl || alt || function_key).then_some(Chord { ctrl, shift, alt, key: name })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        match self.key.chars().next() {
            Some(c) if self.key.chars().count() == 1 => write!(f, "{}", c.to_uppercase()),
            _ => f.write_str(&self.key),
        }
    }
}

/// Key names as written in chords; characters lowercase.
fn normalize(key: &str) -> Option<String> {
    if key.chars().count() == 1 {
        return Some(key.to_lowercase());
    }
    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "escape" | "esc" => "Escape",
        _ => {
            let number = lower.strip_prefix('f')?.parse::<u8>().ok().filter(|n| (1..=24).contains(n))?;
            return Some(format!("F{number}"));
        }
    };
    Some(named.to_string())
}

fn named_key(named: Named) -> Option<&'static str> {
    use Named::*;
    Some(match named {
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Enter => "Enter",
        Tab => "Tab",
        Space => "Space",
        Backspace => "Backspace",
        Delete => "Delete",
        Insert => "Insert",
        Escape => "Escape",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        _ => return None,
    })
}

/// A command that can be bound to keys.
pub struct CommandDef {
    pub id: &'static str,
    pub title: &'static str,
    pub defaults: &'static [&'static str],
}

const fn def(id: &'static str, title: &'static str, defaults: &'static [&'static str]) -> CommandDef {
    CommandDef { id, title, defaults }
}

/// Every bindable command, in the order the shortcuts page lists them.
pub const COMMANDS: &[CommandDef] = &[
    def("file.save", "Save", &["Ctrl+S"]),
    def("file.openFolder", "Open Folder", &["Ctrl+O"]),
    def("file.closeTab", "Close Tab", &["Ctrl+W"]),
    def("file.quickOpen", "Go to File", &["Ctrl+P"]),
    def("palette.commands", "Command Palette", &["Ctrl+Shift+P"]),
    def("go.line", "Go to Line", &["Ctrl+G"]),
    def("go.symbol", "Go to Symbol in File", &["Ctrl+Shift+O"]),
    def("go.definition", "Go to Definition", &["F12"]),
    def("go.references", "Find All References", &["Shift+F12"]),
    def("refactor.rename", "Rename Symbol", &["F2"]),
    def("find.find", "Find", &["Ctrl+F"]),
    def("find.replace", "Replace", &["Ctrl+H"]),
    def("find.next", "Find Next", &["F3"]),
    def("find.previous", "Find Previous", &["Shift+F3"]),
    def("search.project", "Search in Project", &["Ctrl+Shift+F"]),
    def("run.project", "Run Project", &["F5"]),
    def("run.scene", "Run Current Scene", &["F6"]),
    def("run.stop", "Stop", &["F8", "Shift+F5"]),
    def("view.terminal", "Toggle Terminal", &["Ctrl+J", "Ctrl+`", "Ctrl+²"]),
    def("view.split", "Split Editor (Move Tab to the Other Side)", &["Ctrl+\\"]),
    def("view.shortcuts", "Keyboard Shortcuts", &["F1"]),
    def("view.zoomIn", "Zoom In", &["Ctrl+=", "Ctrl++"]),
    def("view.zoomOut", "Zoom Out", &["Ctrl+-"]),
    def("view.zoomReset", "Reset Zoom", &["Ctrl+0"]),
    def("edit.undo", "Undo", &["Ctrl+Z"]),
    def("edit.redo", "Redo", &["Ctrl+Y", "Ctrl+Shift+Z"]),
    def("edit.selectAll", "Select All", &["Ctrl+A"]),
    def("edit.toggleComment", "Toggle Comment", &["Ctrl+K", "Ctrl+/"]),
    def("edit.deleteLine", "Delete Line", &["Ctrl+Shift+K"]),
    def("edit.moveLineUp", "Move Line Up", &["Alt+Up"]),
    def("edit.moveLineDown", "Move Line Down", &["Alt+Down"]),
    def("edit.duplicateLine", "Duplicate Line", &["Shift+Alt+Up", "Shift+Alt+Down"]),
    def("edit.cursorAbove", "Add Cursor Above", &["Ctrl+Alt+Up"]),
    def("edit.cursorBelow", "Add Cursor Below", &["Ctrl+Alt+Down"]),
    def("edit.selectNextOccurrence", "Select Next Occurrence", &["Ctrl+D"]),
    def("edit.completion", "Trigger Completion", &["Ctrl+Space"]),
    def("edit.fold", "Fold / Unfold", &["Alt+F"]),
    def("edit.format", "Format Document", &["Shift+Alt+F"]),
];

/// The commands that work even while the terminal has focus (everything
/// else goes to the shell, which needs Ctrl+letters).
pub const TERMINAL_PASSTHROUGH: &[&str] = &[
    "view.terminal",
    "palette.commands",
    "view.shortcuts",
    "view.split",
    "run.project",
    "run.scene",
    "run.stop",
    "view.zoomIn",
    "view.zoomOut",
    "view.zoomReset",
];

/// The keys in effect: defaults with the user's changes applied.
pub struct Keymap {
    bindings: BTreeMap<&'static str, Vec<Chord>>,
}

impl Keymap {
    pub fn defaults() -> Self {
        let bindings = COMMANDS.iter().map(|c| (c.id, c.defaults.iter().filter_map(|k| Chord::parse(k)).collect())).collect();
        Self { bindings }
    }

    /// Defaults with `overrides` (command id → key list) applied; unknown
    /// ids and unparsable keys are skipped.
    pub fn with_overrides(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let mut keymap = Self::defaults();
        for (id, keys) in overrides {
            if let Some(command) = COMMANDS.iter().find(|c| c.id == id) {
                keymap.bindings.insert(command.id, keys.iter().filter_map(|k| Chord::parse(k)).collect());
            }
        }
        keymap
    }

    /// The command bound to a key press.
    pub fn lookup(&self, chord: &Chord) -> Option<&'static str> {
        self.bindings.iter().find(|(_, chords)| chords.contains(chord)).map(|(id, _)| *id)
    }

    pub fn chords(&self, id: &str) -> &[Chord] {
        self.bindings.get(id).map_or(&[], Vec::as_slice)
    }

    /// The first key of a command, for menus and the palette ("" if none).
    pub fn label(&self, id: &str) -> String {
        self.chords(id).first().map(Chord::to_string).unwrap_or_default()
    }

    /// Binds `chord` to `id` alone (replacing its keys) and takes it from
    /// any other command; returns those commands.
    pub fn rebind(&mut self, id: &str, chord: Chord) -> Vec<&'static str> {
        let mut taken_from = Vec::new();
        for (other, chords) in &mut self.bindings {
            if *other != id && chords.contains(&chord) {
                chords.retain(|c| *c != chord);
                taken_from.push(*other);
            }
        }
        if let Some(chords) = self.bindings.get_mut(id) {
            *chords = vec![chord];
        }
        taken_from
    }

    pub fn unbind(&mut self, id: &str) {
        if let Some(chords) = self.bindings.get_mut(id) {
            chords.clear();
        }
    }

    pub fn reset(&mut self, id: &str) {
        if let Some(command) = COMMANDS.iter().find(|c| c.id == id) {
            self.bindings.insert(command.id, command.defaults.iter().filter_map(|k| Chord::parse(k)).collect());
        }
    }

    pub fn is_default(&self, id: &str) -> bool {
        let defaults = Self::defaults();
        defaults.chords(id) == self.chords(id)
    }

    /// The commands whose keys differ from the defaults, as saved.
    pub fn overrides(&self) -> BTreeMap<String, Vec<String>> {
        COMMANDS
            .iter()
            .filter(|c| !self.is_default(c.id))
            .map(|c| (c.id.to_string(), self.chords(c.id).iter().map(Chord::to_string).collect()))
            .collect()
    }

    pub fn load() -> Self {
        let overrides = file()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        Self::with_overrides(&overrides)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = file().ok_or("No user config folder")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.overrides()).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

fn file() -> Option<PathBuf> {
    Some(theme::config_dir()?.join("keybindings.json"))
}

pub fn title(id: &str) -> &'static str {
    COMMANDS.iter().find(|c| c.id == id).map_or("", |c| c.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse_and_print() {
        for text in ["Ctrl+Shift+P", "F5", "Shift+F12", "Alt+Up", "Ctrl+\\", "Ctrl++", "Ctrl+=", "Shift+Alt+F", "Ctrl+Space"] {
            let chord = Chord::parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(chord.to_string(), text);
        }
        assert_eq!(Chord::parse("ctrl+shift+p"), Chord::parse("Ctrl+Shift+P"));
        assert_eq!(Chord::parse("Hyper+X"), None);
        assert_eq!(Chord::parse("Ctrl+NotAKey"), None);
    }

    #[test]
    fn key_presses_become_chords_only_for_shortcuts() {
        let ctrl = Modifiers::CTRL;
        assert_eq!(Chord::from_key(&Key::Character("P".into()), ctrl | Modifiers::SHIFT), Chord::parse("Ctrl+Shift+P"));
        assert_eq!(Chord::from_key(&Key::Named(Named::F5), Modifiers::empty()), Chord::parse("F5"));
        // Plain typing, and AltGr (Ctrl+Alt) characters, are not shortcuts.
        assert_eq!(Chord::from_key(&Key::Character("a".into()), Modifiers::empty()), None);
        assert_eq!(Chord::from_key(&Key::Character("@".into()), ctrl | Modifiers::ALT), None);
        assert_eq!(Chord::from_key(&Key::Named(Named::ArrowUp), Modifiers::ALT), Chord::parse("Alt+Up"));
    }

    #[test]
    fn overrides_rebinding_and_saving_differences() {
        let mut keymap = Keymap::defaults();
        assert_eq!(keymap.lookup(&Chord::parse("Ctrl+S").unwrap()), Some("file.save"));
        assert_eq!(keymap.label("run.stop"), "F8");

        // Taking Ctrl+S for "Run Project" removes it from "Save".
        let taken = keymap.rebind("run.project", Chord::parse("Ctrl+S").unwrap());
        assert_eq!(taken, ["file.save"]);
        assert_eq!(keymap.lookup(&Chord::parse("Ctrl+S").unwrap()), Some("run.project"));
        assert_eq!(keymap.lookup(&Chord::parse("F5").unwrap()), None);
        let saved = keymap.overrides();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved["run.project"], ["Ctrl+S"]);
        assert!(saved["file.save"].is_empty());

        let reloaded = Keymap::with_overrides(&saved);
        assert_eq!(reloaded.lookup(&Chord::parse("Ctrl+S").unwrap()), Some("run.project"));
        keymap.reset("file.save");
        keymap.reset("run.project");
        assert!(keymap.overrides().is_empty());

        let mut wild = BTreeMap::new();
        wild.insert("no.such.command".to_string(), vec!["Ctrl+Q".to_string()]);
        wild.insert("file.save".to_string(), vec!["garbage".to_string(), "Ctrl+Shift+S".to_string()]);
        let keymap = Keymap::with_overrides(&wild);
        assert_eq!(keymap.chords("file.save"), [Chord::parse("Ctrl+Shift+S").unwrap()]);
    }

    #[test]
    fn defaults_parse_and_dont_clash() {
        let keymap = Keymap::defaults();
        let mut seen = std::collections::HashSet::new();
        for command in COMMANDS {
            assert_eq!(keymap.chords(command.id).len(), command.defaults.len(), "{}", command.id);
            for chord in keymap.chords(command.id) {
                assert!(seen.insert(chord.clone()), "{chord} bound twice");
            }
        }
    }
}
