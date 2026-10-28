//! The settings page (Ctrl+,): every setting with its control, filterable,
//! for you (`settings.json` in the config folder) or for the open project
//! (`.beckett/settings.json`, overriding yours where a setting allows it).

use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input, toggler, Column};
use iced::{Alignment, Element, Length};
use serde_json::{Map, Value};

use crate::settings::{Item, Kind, Settings, ITEMS};
use crate::style;

pub const INPUT: &str = "settings-filter";

/// Whose settings the page changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    User,
    Project,
}

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    Filter(String),
    Scope(Scope),
    Set(&'static str, Value),
    /// Back to the default (user scope) or to your setting (project scope).
    Reset(&'static str),
    /// Open the JSON file of the current scope in a tab.
    OpenFile,
    Close,
}

#[derive(Default)]
pub struct SettingsPage {
    pub filter: String,
    pub scope: Scope,
}

impl SettingsPage {
    /// Settings matching the filter (title, description, section or key).
    pub fn matching(&self) -> Vec<&'static Item> {
        let words: Vec<String> = self.filter.split_whitespace().map(str::to_lowercase).collect();
        ITEMS
            .iter()
            .filter(|i| {
                let haystack = format!("{} {} {} {}", i.section, i.title, i.description, i.key).to_lowercase();
                words.iter().all(|w| haystack.contains(w.as_str()))
            })
            .collect()
    }

    /// `project` holds the open project's overrides (none without a project).
    pub fn view<'a>(
        &'a self,
        ui: &theme::UiColors,
        user: &Settings,
        project: Option<&Map<String, Value>>,
    ) -> Element<'a, SettingsMessage> {
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let scope = if project.is_some() { self.scope } else { Scope::User };
        let defaults = Settings::default();
        let effective = project.map(|p| user.with_project(p));
        let shown = effective.as_ref().filter(|_| scope == Scope::Project).unwrap_or(user);

        let close = button(text("x").size(12)).on_press(SettingsMessage::Close).style(button::text).padding([2, 6]);
        let mut scopes = row![].spacing(4).align_y(Alignment::Center);
        for (choice, label, tip) in [
            (Scope::User, "User", "Your settings, for every project"),
            (Scope::Project, "Project", "This project's settings: they override yours (where a setting allows it)"),
        ] {
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if scope == choice { button::secondary } else { button::text };
            let enabled = choice == Scope::User || project.is_some();
            let pick = button(text(label).size(12))
                .on_press_maybe(enabled.then_some(SettingsMessage::Scope(choice)))
                .style(style)
                .padding([2, 8]);
            let tip = if enabled { tip } else { "Open a project to give it its own settings" };
            scopes = scopes.push(style::tip(pick, tip, Position::Bottom, ui));
        }
        let open = button(text("Open JSON").size(12)).on_press(SettingsMessage::OpenFile).style(button::text).padding([2, 8]);
        let open_tip = match scope {
            Scope::User => "Edit settings.json (your settings) in a tab",
            Scope::Project => "Edit .beckett/settings.json (this project's settings) in a tab",
        };
        let header = row![
            text("Settings").size(20),
            space::horizontal(),
            scopes,
            style::tip(open, open_tip, Position::Bottom, ui),
            text_input("Filter (e.g. \"cursor\")", &self.filter)
                .id(INPUT)
                .on_input(SettingsMessage::Filter)
                .size(13)
                .padding([3, 6])
                .width(240),
            style::tip(close, "Close (Escape)", Position::Bottom, ui),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let mut list: Vec<Element<'a, SettingsMessage>> = Vec::new();
        let mut section = "";
        for item in self.matching() {
            if item.section != section {
                section = item.section;
                list.push(container(text(section.to_uppercase()).size(12).color(accent)).padding([12, 0]).into());
            }
            let value = shown.value(item.key).unwrap_or(Value::Null);
            // In project scope, user-only settings show yours, unchangeable.
            let editable = scope == Scope::User || item.project;
            let control = control(item, &value, editable, muted);

            let mut title = row![text(item.title).size(13)].spacing(8).align_y(Alignment::Center);
            let reset_tip = match scope {
                Scope::User if value != defaults.value(item.key).unwrap_or(Value::Null) => Some("Back to the default"),
                Scope::Project if project.is_some_and(|p| p.contains_key(item.key)) => {
                    title = title.push(text("set for this project").size(11).color(accent));
                    Some("Use your own setting in this project")
                }
                Scope::Project if !item.project => {
                    title = title.push(text("yours only").size(11).color(muted));
                    None
                }
                _ => None,
            };
            if let Some(tip) = reset_tip {
                let reset = button(text("reset").size(11)).on_press(SettingsMessage::Reset(item.key)).style(button::text).padding([0, 6]);
                title = title.push(style::tip(reset, tip, Position::Top, ui));
            }
            let words = column![title, text(item.description).size(12).color(muted)].spacing(2);
            list.push(
                row![container(control).width(140), words]
                    .spacing(16)
                    .align_y(Alignment::Center)
                    .padding([6, 0])
                    .into(),
            );
        }
        if list.is_empty() {
            list.push(text("No setting matches the filter").size(13).color(muted).into());
        }
        // Right padding keeps the scrollbar off the descriptions.
        let list = Column::with_children(list).padding(iced::Padding { left: 4.0, right: 24.0, ..iced::Padding::ZERO });
        let body = scrollable(list).height(Length::Fill);

        container(column![header, body].spacing(12).max_width(1000))
            .padding([16, 24])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// A toggle, or a number with − / + buttons.
fn control<'a>(item: &'static Item, value: &Value, editable: bool, muted: iced::Color) -> Element<'a, SettingsMessage> {
    match item.kind {
        Kind::Toggle => {
            let on = value.as_bool().unwrap_or(false);
            let key = item.key;
            toggler(on).on_toggle_maybe(editable.then_some(move |on| SettingsMessage::Set(key, Value::Bool(on)))).size(18).into()
        }
        Kind::Number { min, max, step, scale, unit } => {
            let n = value.as_f64().unwrap_or(min);
            let step_to = |target: f64| {
                let target = target.clamp(min, max);
                (editable && target != n).then(|| SettingsMessage::Set(item.key, number(value, target, step)))
            };
            let less = button(text("−").size(13)).on_press_maybe(step_to(n - step)).style(button::text).padding([0, 8]);
            let more = button(text("+").size(13)).on_press_maybe(step_to(n + step)).style(button::text).padding([0, 8]);
            let shown = text(format!("{}{unit}", (n * scale).round())).size(13);
            let shown = if editable { shown } else { shown.color(muted) };
            row![less, shown, more].spacing(4).align_y(Alignment::Center).into()
        }
    }
}

/// `target` as a JSON number of the same kind as `current` (whole numbers
/// stay whole), rounded to the step so repeated steps don't drift.
fn number(current: &Value, target: f64, step: f64) -> Value {
    let rounded = (target / step).round() * step;
    if current.is_u64() {
        Value::from(rounded.round() as u64)
    } else {
        Value::from((rounded * 100.0).round() / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_title_description_or_section() {
        let mut page = SettingsPage::default();
        assert_eq!(page.matching().len(), ITEMS.len());
        page.filter = "BLINK".to_string();
        assert_eq!(page.matching().iter().map(|i| i.key).collect::<Vec<_>>(), ["cursor_blink"]);
        page.filter = "interface".to_string();
        assert_eq!(page.matching().iter().map(|i| i.key).collect::<Vec<_>>(), ["ui_scale"]);
        page.filter = "nothing like this".to_string();
        assert!(page.matching().is_empty());
    }

    #[test]
    fn steps_keep_the_number_kind_and_do_not_drift() {
        assert_eq!(number(&Value::from(500u64), 600.0, 100.0), Value::from(600u64));
        assert_eq!(number(&Value::from(1.0), 1.1 + 0.1 + 0.1, 0.1), Value::from(1.3));
    }

    #[test]
    fn sections_are_contiguous() {
        // The page starts a section header whenever the section changes.
        let mut seen: Vec<&str> = Vec::new();
        for item in ITEMS {
            if seen.last() != Some(&item.section) {
                assert!(!seen.contains(&item.section), "{} is split", item.section);
                seen.push(item.section);
            }
        }
    }
}
