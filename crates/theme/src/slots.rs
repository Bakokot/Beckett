//! Every color of a theme as a named slot, for editing them generically.

use crate::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Ui,
    Syntax,
    Terminal,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Ui => "Interface",
            Section::Syntax => "Syntax",
            Section::Terminal => "Terminal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub section: Section,
    /// The key in the theme file.
    pub key: &'static str,
    pub label: &'static str,
}

macro_rules! slots {
    ($($section:ident $group:ident . $field:ident => $key:literal, $label:literal;)*) => {
        /// All slots, in display order.
        pub const SLOTS: &[Slot] = &[
            $(Slot { section: Section::$section, key: $key, label: $label },)*
        ];

        impl Theme {
            pub fn color(&self, slot: &Slot) -> Color {
                match (slot.section, slot.key) {
                    $((Section::$section, $key) => self.$group.$field,)*
                    _ => unreachable!("unknown slot {}", slot.key),
                }
            }

            pub fn set_color(&mut self, slot: &Slot, color: Color) {
                match (slot.section, slot.key) {
                    $((Section::$section, $key) => self.$group.$field = color,)*
                    _ => unreachable!("unknown slot {}", slot.key),
                }
            }
        }
    };
}

slots! {
    Ui ui.background => "background", "Editor background";
    Ui ui.surface => "surface", "Panels and bars";
    Ui ui.text => "text", "Text";
    Ui ui.text_muted => "text_muted", "Secondary text";
    Ui ui.accent => "accent", "Accent (selection, buttons)";
    Ui ui.border => "border", "Borders";
    Ui ui.success => "success", "Success";
    Ui ui.warning => "warning", "Warnings";
    Ui ui.danger => "danger", "Errors";
    Syntax syntax.text => "text", "Plain code";
    Syntax syntax.keyword => "keyword", "Keywords (var, func)";
    Syntax syntax.control_flow => "control_flow", "Control flow (if, return)";
    Syntax syntax.r#type => "type", "Types (Vector2, int)";
    Syntax syntax.function => "function", "Function calls";
    Syntax syntax.annotation => "annotation", "Annotations (@export)";
    Syntax syntax.string => "string", "Strings";
    Syntax syntax.string_name => "string_name", "StringNames (&\"name\")";
    Syntax syntax.node_path => "node_path", "Node paths ($Node)";
    Syntax syntax.number => "number", "Numbers";
    Syntax syntax.comment => "comment", "Comments";
    Syntax syntax.constant => "constant", "Constants (true, MAX_HP)";
    Syntax syntax.symbol => "symbol", "Operators and brackets";
    Terminal terminal.black => "black", "Black";
    Terminal terminal.red => "red", "Red";
    Terminal terminal.green => "green", "Green";
    Terminal terminal.yellow => "yellow", "Yellow";
    Terminal terminal.blue => "blue", "Blue";
    Terminal terminal.magenta => "magenta", "Magenta";
    Terminal terminal.cyan => "cyan", "Cyan";
    Terminal terminal.white => "white", "White";
    Terminal terminal.bright_black => "bright_black", "Bright black";
    Terminal terminal.bright_red => "bright_red", "Bright red";
    Terminal terminal.bright_green => "bright_green", "Bright green";
    Terminal terminal.bright_yellow => "bright_yellow", "Bright yellow";
    Terminal terminal.bright_blue => "bright_blue", "Bright blue";
    Terminal terminal.bright_magenta => "bright_magenta", "Bright magenta";
    Terminal terminal.bright_cyan => "bright_cyan", "Bright cyan";
    Terminal terminal.bright_white => "bright_white", "Bright white";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_reads_and_writes_its_own_field() {
        let mut theme = crate::builtin().remove(0);
        for (i, slot) in SLOTS.iter().enumerate() {
            let color = Color { r: i as u8, g: 1, b: 2, a: 255 };
            theme.set_color(slot, color);
            assert_eq!(theme.color(slot), color, "{}", slot.key);
        }
        // Writing one slot never changed another.
        for (i, slot) in SLOTS.iter().enumerate() {
            assert_eq!(theme.color(slot).r, i as u8, "{}", slot.key);
        }
        assert_eq!(SLOTS.len(), 9 + 13 + 16);
    }
}
