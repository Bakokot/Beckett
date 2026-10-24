//! Live theme editor: every color slot with a picker. The draft theme is
//! applied to the whole IDE while editing; saving writes it to a file.

use editor_core::{Buffer, Language};
use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, slider, space, text, text_input, Column};
use iced::{Alignment, Background, Border, Element, Font, Length};
use theme::{Color, Theme, SLOTS};

use crate::code_editor::{self, CodeEditor};
use crate::style;

pub const NAME_INPUT: &str = "theme-name";

const SAMPLE: &str = r#"class_name Player
extends CharacterBody2D

## Movement speed in pixels per second.
@export var speed: float = 300.0
@onready var sprite := $Sprite2D
const MAX_HP = 100
signal died

func _physics_process(delta: float) -> void:
	var direction := Input.get_vector(&"left", &"right", &"up", &"down")
	if direction.length() > 0.0:
		velocity = direction * speed
	move_and_slide()  # 2 + 2 == 4
"#;

#[derive(Debug, Clone)]
pub enum ThemeMessage {
    Select(usize),
    Hex(String),
    Hue(f32),
    Saturation(f32),
    Brightness(f32),
    Opacity(f32),
    Name(String),
    Save,
    Revert,
    Close,
    Import,
    Export,
    CopyCode,
    PasteCode,
    /// Input on the preview editor, which is not editable.
    Ignore,
}

/// HSV plus alpha, kept alongside the color so the hue survives while
/// saturation or brightness is at zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    /// 0..360
    pub h: f32,
    /// 0..100
    pub s: f32,
    /// 0..100
    pub v: f32,
    /// 0..255
    pub a: f32,
}

impl Hsv {
    pub fn from_color(c: Color) -> Self {
        let (r, g, b) = (f32::from(c.r) / 255.0, f32::from(c.g) / 255.0, f32::from(c.b) / 255.0);
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let s = if max == 0.0 { 0.0 } else { delta / max };
        Self { h, s: s * 100.0, v: max * 100.0, a: f32::from(c.a) }
    }

    pub fn to_color(self) -> Color {
        let (s, v) = (self.s / 100.0, self.v / 100.0);
        let c = v * s;
        let h = (self.h.rem_euclid(360.0)) / 60.0;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let byte = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color { r: byte(r), g: byte(g), b: byte(b), a: self.a.round().clamp(0.0, 255.0) as u8 }
    }
}

pub struct ThemeEditor {
    pub draft: Theme,
    /// Index in the app's theme list the draft started from.
    pub base: usize,
    /// The base is built in (saving creates a copy).
    pub base_is_builtin: bool,
    pub selected: usize,
    pub hex: String,
    hsv: Hsv,
    pub dirty: bool,
    sample: Buffer,
}

impl ThemeEditor {
    pub fn new(theme: Theme, base: usize, base_is_builtin: bool) -> Self {
        let mut editor = Self {
            draft: theme,
            base,
            base_is_builtin,
            selected: 0,
            hex: String::new(),
            hsv: Hsv { h: 0.0, s: 0.0, v: 0.0, a: 255.0 },
            dirty: false,
            sample: Buffer::new(SAMPLE, Language::GDScript),
        };
        editor.select(0);
        editor
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(SLOTS.len() - 1);
        let color = self.draft.color(&SLOTS[self.selected]);
        self.hex = color.to_string();
        self.hsv = Hsv::from_color(color);
    }

    /// Replaces the draft (revert, or another theme picked while editing).
    pub fn reset(&mut self, theme: Theme, base: usize, base_is_builtin: bool) {
        self.draft = theme;
        self.base = base;
        self.base_is_builtin = base_is_builtin;
        self.dirty = false;
        self.select(self.selected);
    }

    /// Applies a color or name change. Returns whether the draft changed.
    pub fn apply(&mut self, message: &ThemeMessage) -> bool {
        let slot = &SLOTS[self.selected];
        match message {
            ThemeMessage::Select(index) => {
                self.select(*index);
                return false;
            }
            ThemeMessage::Hex(input) => {
                self.hex = input.clone();
                let hex = if input.starts_with('#') { input.clone() } else { format!("#{input}") };
                let Ok(color) = Color::from_hex(hex.trim()) else {
                    return false;
                };
                self.hsv = Hsv::from_color(color);
                self.draft.set_color(slot, color);
            }
            ThemeMessage::Hue(h) => self.set_hsv(Hsv { h: *h, ..self.hsv }),
            ThemeMessage::Saturation(s) => self.set_hsv(Hsv { s: *s, ..self.hsv }),
            ThemeMessage::Brightness(v) => self.set_hsv(Hsv { v: *v, ..self.hsv }),
            ThemeMessage::Opacity(a) => self.set_hsv(Hsv { a: *a, ..self.hsv }),
            ThemeMessage::Name(name) => self.draft.name = name.clone(),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    fn set_hsv(&mut self, hsv: Hsv) {
        self.hsv = hsv;
        let color = hsv.to_color();
        self.hex = color.to_string();
        self.draft.set_color(&SLOTS[self.selected], color);
    }

    pub fn view(&self) -> Element<'_, ThemeMessage> {
        let ui = &self.draft.ui;
        let muted = style::to_color(ui.text_muted);
        let action = |label: &'static str, message: ThemeMessage, description: &'static str| {
            let button = button(text(label).size(12)).on_press(message).style(button::secondary).padding([3, 10]);
            style::tip(button, description, Position::Bottom, ui)
        };

        let note = if self.base_is_builtin {
            "Built-in theme: saving creates a copy in your themes folder"
        } else if self.dirty {
            "Unsaved changes"
        } else {
            "Changes apply live"
        };
        let header = row![
            text("THEME").size(12),
            text_input("Theme name", &self.draft.name)
                .id(NAME_INPUT)
                .on_input(ThemeMessage::Name)
                .size(13)
                .padding([3, 6])
                .width(240),
            text(note).size(12).color(muted),
            space::horizontal(),
            action("Save", ThemeMessage::Save, "Save to your themes folder"),
            action("Revert", ThemeMessage::Revert, "Undo all changes since the last save"),
            action("Close", ThemeMessage::Close, "Close the editor; unsaved changes are discarded"),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let share = row![
            action("Import...", ThemeMessage::Import, "Add a theme from a .toml file"),
            action("Export...", ThemeMessage::Export, "Save this theme as a .toml file to share"),
            action("Copy share code", ThemeMessage::CopyCode, "Copy this theme as one line of text to paste in a chat"),
            action("Paste share code", ThemeMessage::PasteCode, "Add a theme from a code in the clipboard"),
            text("You can also drop a .toml theme file on the window.").size(12).color(muted),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let body = row![
            scrollable(self.slot_list()).width(Length::Fill).height(Length::Fill),
            container(self.picker()).width(380).height(Length::Fill),
        ]
        .spacing(12);

        container(column![header, share, body].spacing(10))
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::surface(ui))
            .into()
    }

    fn slot_list(&self) -> Element<'_, ThemeMessage> {
        let ui = &self.draft.ui;
        let muted = style::to_color(ui.text_muted);
        let (accent, text_color) = (style::to_color(ui.accent).scale_alpha(0.25), style::to_color(ui.text));
        let mut list = Column::new().spacing(2).padding([0, 8]);
        let mut section = None;
        for (index, slot) in SLOTS.iter().enumerate() {
            if section != Some(slot.section) {
                section = Some(slot.section);
                list = list.push(container(text(slot.section.title()).size(13)).padding([8, 0]));
            }
            let color = self.draft.color(slot);
            let selected = index == self.selected;
            let row = row![
                swatch(color, ui, 28.0, 16.0),
                text(slot.label).size(13),
                space::horizontal(),
                text(color.to_string()).size(12).font(Font::MONOSPACE).color(muted),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            list = list.push(
                button(row)
                    .on_press(ThemeMessage::Select(index))
                    .width(Length::Fill)
                    .padding([3, 6])
                    .style(move |theme: &iced::Theme, status| {
                        if selected {
                            button::Style {
                                background: Some(Background::Color(accent)),
                                text_color,
                                border: Border { radius: 4.0.into(), ..Border::default() },
                                ..button::Style::default()
                            }
                        } else {
                            button::text(theme, status)
                        }
                    }),
            );
        }
        list.into()
    }

    fn picker(&self) -> Element<'_, ThemeMessage> {
        let ui = &self.draft.ui;
        let muted = style::to_color(ui.text_muted);
        let slot = &SLOTS[self.selected];
        let color = self.draft.color(slot);
        let channel = |label: &'static str, value: f32, max: f32, on_change: fn(f32) -> ThemeMessage| {
            row![
                text(label).size(12).width(80),
                slider(0.0..=max, value, on_change).step(1.0_f32),
                text(format!("{value:.0}")).size(12).font(Font::MONOSPACE).width(34),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        };

        let terminal_row = |colors: &[Color]| {
            iced::widget::Row::with_children(colors.iter().map(|&c| swatch(c, ui, 22.0, 16.0)))
                .spacing(4)
        };
        let ansi = self.draft.terminal.ansi();

        column![
            text(format!("{} / {}", slot.section.title(), slot.label)).size(13),
            swatch(color, ui, 380.0, 44.0),
            row![
                text("Hex").size(12).width(80),
                text_input("#rrggbb or #rrggbbaa", &self.hex)
                    .on_input(ThemeMessage::Hex)
                    .size(13)
                    .padding([3, 6])
                    .font(Font::MONOSPACE),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            channel("Hue", self.hsv.h, 359.0, ThemeMessage::Hue),
            channel("Saturation", self.hsv.s, 100.0, ThemeMessage::Saturation),
            channel("Brightness", self.hsv.v, 100.0, ThemeMessage::Brightness),
            channel("Opacity", self.hsv.a, 255.0, ThemeMessage::Opacity),
            text("Preview").size(12).color(muted),
            container(
                CodeEditor::new(&self.sample, code_editor::Colors::from_theme(&self.draft), |_| ThemeMessage::Ignore)
                    .focused(false),
            )
            .height(250),
            text("Terminal colors").size(12).color(muted),
            terminal_row(&ansi[..8]),
            terminal_row(&ansi[8..]),
        ]
        .spacing(8)
        .into()
    }
}

/// A filled color sample with a thin border (visible on any background).
fn swatch<'a, Message: 'a>(color: Color, ui: &theme::UiColors, width: f32, height: f32) -> Element<'a, Message> {
    let fill = style::to_color(color);
    let border = style::to_color(ui.border);
    container(space())
        .width(width)
        .height(height)
        .style(move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border { color: border, width: 1.0, radius: 3.0.into() },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips_colors() {
        for hex in ["#ff7085", "#42ffc2", "#1d2229", "#000000", "#ffffff", "#478cbf80", "#808080"] {
            let color = Color::from_hex(hex).unwrap();
            assert_eq!(Hsv::from_color(color).to_color(), color, "{hex}");
        }
        let red = Hsv::from_color(Color::from_hex("#ff0000").unwrap());
        assert_eq!((red.h, red.s, red.v), (0.0, 100.0, 100.0));
    }

    #[test]
    fn hue_survives_desaturation() {
        let mut editor = ThemeEditor::new(theme::builtin().remove(0), 0, true);
        editor.select(4); // accent
        editor.apply(&ThemeMessage::Hue(120.0));
        editor.apply(&ThemeMessage::Saturation(0.0));
        editor.apply(&ThemeMessage::Saturation(100.0));
        assert_eq!(editor.hsv.h, 120.0);
        assert!(editor.dirty);
    }

    #[test]
    fn hex_input_accepts_with_or_without_hash_and_ignores_partial_input() {
        let mut editor = ThemeEditor::new(theme::builtin().remove(0), 0, true);
        let slot = SLOTS[0];
        assert!(editor.apply(&ThemeMessage::Hex("112233".into())));
        assert_eq!(editor.draft.color(&slot).to_string(), "#112233");
        assert!(!editor.apply(&ThemeMessage::Hex("#12".into())));
        assert_eq!(editor.hex, "#12");
        assert_eq!(editor.draft.color(&slot).to_string(), "#112233");
    }
}
