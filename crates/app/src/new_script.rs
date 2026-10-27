//! The New Script bar: path, base class and template of a GDScript file to
//! create. Templates are Godot's built-in ones plus the project's own
//! (`res://script_templates/<Base>/*.gd`, with `# meta-…` header lines).

use std::path::{Path, PathBuf};

use godot_project::GodotProject;
use iced::widget::tooltip::Position;
use iced::widget::{button, container, pick_list, row, space, text, text_input};
use iced::{Alignment, Element, Length};

use crate::style;

pub const PATH_INPUT: &str = "new-script-path";

/// File name (without `.gd`) the path starts with, selected for typing over.
pub const DEFAULT_NAME: &str = "new_script";

#[derive(Debug, Clone)]
pub enum NewScriptMessage {
    Path(String),
    Extends(String),
    Template(String),
    Submit,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: String,
    /// The text with `_BASE_`, `_CLASS_`, `_CLASS_SNAKE_CASE_` and `_TS_`
    /// (one indent) placeholders.
    pub body: String,
    /// The indent `_TS_` stands for.
    pub indent: String,
    /// Picked first (`# meta-default: true`).
    pub default: bool,
}

pub struct NewScript {
    /// `res://` path of the file to create.
    pub path: String,
    pub extends: String,
    /// Templates for `extends`: built-ins, then the project's.
    pub templates: Vec<Template>,
    pub template: usize,
    /// Where the project's templates are, if it has any.
    templates_dir: Option<PathBuf>,
}

/// Godot's own templates (`Node/default.gd`, `Object/empty.gd`).
fn builtin_templates() -> Vec<Template> {
    let template = |name: &str, body: &str| Template {
        name: name.to_string(),
        body: body.to_string(),
        indent: "\t".to_string(),
        default: false,
    };
    vec![
        template(
            "Default",
            "extends _BASE_\n\n\n# Called when the node enters the scene tree for the first time.\nfunc _ready() -> void:\n_TS_pass # Replace with function body.\n\n\n# Called every frame. 'delta' is the elapsed time since the previous frame.\nfunc _process(delta: float) -> void:\n_TS_pass\n",
        ),
        template("Empty", "extends _BASE_\n"),
    ]
}

impl NewScript {
    /// A bar for a new script in `folder` (a `res://` path).
    pub fn new(godot: &GodotProject, folder: &str) -> Self {
        let separator = if folder.ends_with('/') { "" } else { "/" };
        let dir = godot
            .settings
            .get_string("editor/script/templates_search_path")
            .unwrap_or_else(|| "res://script_templates".to_string());
        let mut script = Self {
            path: format!("{folder}{separator}{DEFAULT_NAME}.gd"),
            extends: "Node".to_string(),
            templates: Vec::new(),
            template: 0,
            templates_dir: godot.resolve(&dir).filter(|d| d.is_dir()),
        };
        script.load_templates();
        script
    }

    /// Lists the templates for the current base class; the project's
    /// `Object` templates apply to every class, as in Godot.
    fn load_templates(&mut self) {
        let previous = self.templates.get(self.template).map(|t| t.name.clone());
        self.templates = builtin_templates();
        if let Some(dir) = &self.templates_dir {
            let base = self.extends.trim();
            let mut folders = vec![base];
            if base != "Object" {
                folders.push("Object");
            }
            for folder in folders {
                self.templates.extend(read_templates(&dir.join(folder)));
            }
        }
        self.template = previous
            .and_then(|name| self.templates.iter().position(|t| t.name == name))
            .or_else(|| self.templates.iter().position(|t| t.default))
            .unwrap_or(0);
    }

    pub fn update(&mut self, message: NewScriptMessage) {
        match message {
            NewScriptMessage::Path(path) => self.path = path,
            NewScriptMessage::Extends(extends) => {
                self.extends = extends;
                self.load_templates();
            }
            NewScriptMessage::Template(name) => {
                if let Some(index) = self.templates.iter().position(|t| t.name == name) {
                    self.template = index;
                }
            }
            NewScriptMessage::Submit | NewScriptMessage::Close => {}
        }
    }

    /// The file to create and its text, or why it can't be.
    pub fn validate(&self, godot: &GodotProject) -> Result<(PathBuf, String), String> {
        let path = self.path.trim();
        if !path.starts_with("res://") {
            return Err("The path must start with res://".into());
        }
        let file = path.rsplit('/').next().unwrap_or_default();
        let Some(stem) = file.strip_suffix(".gd").filter(|s| !s.is_empty()) else {
            return Err("The file name must end with .gd".into());
        };
        let Some(target) = godot.resolve(path) else {
            return Err("Not a path inside the project".into());
        };
        if target.exists() {
            return Err("A file with this name already exists".into());
        }
        let extends = self.extends.trim();
        if !valid_extends(extends) {
            return Err("Extends needs a class name or a \"res://…\" path".into());
        }
        let template = self.templates.get(self.template).ok_or("No template")?;
        Ok((target, expand(template, extends, stem)))
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors, error: Option<&'a str>) -> Element<'a, NewScriptMessage> {
        let muted = style::to_color(ui.text_muted);
        let hint = match error {
            Some(error) => text(error).size(12).color(style::to_color(ui.danger)),
            None => text("Enter to create, Escape to cancel").size(12).color(muted),
        };
        let names: Vec<String> = self.templates.iter().map(|t| t.name.clone()).collect();
        let selected = names.get(self.template).cloned();
        let close = button(text("x").size(12)).on_press(NewScriptMessage::Close).style(button::text).padding([2, 6]);
        let content = row![
            text("New script").size(12),
            text_input("res://path/to/script.gd", &self.path)
                .id(PATH_INPUT)
                .on_input(NewScriptMessage::Path)
                .on_submit(NewScriptMessage::Submit)
                .size(13)
                .padding([3, 6])
                .width(300),
            text("extends").size(12).color(muted),
            text_input("Node", &self.extends)
                .on_input(NewScriptMessage::Extends)
                .on_submit(NewScriptMessage::Submit)
                .size(13)
                .padding([3, 6])
                .width(150),
            text("template").size(12).color(muted),
            pick_list(names, selected, NewScriptMessage::Template).text_size(13).padding([3, 6]),
            button(text("Create").size(12)).on_press(NewScriptMessage::Submit).padding([3, 10]),
            hint,
            space::horizontal(),
            style::tip(close, "Cancel (Escape)", Position::Bottom, ui),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        container(content).padding([4, 8]).width(Length::Fill).style(style::surface(ui)).into()
    }
}

/// A class name (`Node2D`, `MyClass.Inner`) or a quoted script path.
fn valid_extends(extends: &str) -> bool {
    if let Some(path) = extends.strip_prefix('"').and_then(|e| e.strip_suffix('"')) {
        return path.starts_with("res://") || path.starts_with("uid://");
    }
    !extends.is_empty()
        && extends.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// The `.gd` templates in `dir`, sorted by name.
fn read_templates(dir: &Path) -> Vec<Template> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "gd")).collect();
    files.sort();
    files
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let stem = path.file_stem()?.to_string_lossy();
            Some(parse_template(&stem, &text))
        })
        .collect()
}

/// Reads the `# meta-name:`, `# meta-default:` and `# meta-space-indent:`
/// lines of a template and drops every `# meta-` line from its body.
fn parse_template(file_stem: &str, text: &str) -> Template {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut template = Template { name: file_stem.to_string(), body: String::new(), indent: "\t".to_string(), default: false };
    for line in text.split_inclusive('\n') {
        let Some(meta) = line.trim().strip_prefix("# meta-") else {
            template.body.push_str(line);
            continue;
        };
        let (key, value) = meta.split_once(':').unwrap_or((meta, ""));
        let value = value.trim();
        match key.trim() {
            "name" if !value.is_empty() => template.name = value.to_string(),
            "default" => template.default = value.eq_ignore_ascii_case("true"),
            "space-indent" => {
                if let Ok(width) = value.parse::<usize>() {
                    template.indent = " ".repeat(width.min(16));
                }
            }
            _ => {}
        }
    }
    template
}

fn expand(template: &Template, extends: &str, file_stem: &str) -> String {
    template
        .body
        .replace("_CLASS_SNAKE_CASE_", &snake_case(file_stem))
        .replace("_BASE_", extends)
        .replace("_CLASS_", &pascal_case(file_stem))
        .replace("_TS_", &template.indent)
}

/// `big_enemy` / `big-enemy` → `BigEnemy`.
fn pascal_case(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

/// `BigEnemy` / `big-enemy` → `big_enemy`.
fn snake_case(name: &str) -> String {
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
        out.extend(c.to_lowercase());
        previous = Some(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_the_default_template() {
        let text = expand(&builtin_templates()[0], "CharacterBody2D", "player");
        assert!(text.starts_with("extends CharacterBody2D\n\n\n# Called when"));
        assert!(text.contains("func _ready() -> void:\n\tpass # Replace with function body.\n"));
        assert!(!text.contains("_TS_") && !text.contains("_BASE_"));
    }

    #[test]
    fn reads_project_template_metadata() {
        let source = "# meta-name: State\n# meta-description: A state machine state\n# meta-default: true\n# meta-space-indent: 4\nclass_name _CLASS_\nextends _BASE_\n\nfunc enter() -> void:\n_TS_pass\n";
        let template = parse_template("state", source);
        assert_eq!(template.name, "State");
        assert!(template.default);
        assert_eq!(
            expand(&template, "Node", "big_enemy_state"),
            "class_name BigEnemyState\nextends Node\n\nfunc enter() -> void:\n    pass\n"
        );
        assert_eq!(parse_template("plain", "extends _BASE_\n").name, "plain");
    }

    #[test]
    fn validates_the_path() {
        let root = std::env::temp_dir().join(format!("beckett-new-script-{}", std::process::id()));
        std::fs::create_dir_all(root.join("player")).unwrap();
        std::fs::write(root.join("player/taken.gd"), "extends Node\n").unwrap();
        let godot = GodotProject { root: root.clone(), settings: Default::default() };
        let mut script = NewScript::new(&godot, "res://player");
        assert_eq!(script.path, "res://player/new_script.gd");
        assert_eq!(NewScript::new(&godot, "res://").path, "res://new_script.gd");

        let (target, text) = script.validate(&godot).unwrap();
        assert_eq!(target, root.join("player").join("new_script.gd"));
        assert!(text.starts_with("extends Node\n"));
        for (path, error) in [
            ("player/x.gd", "The path must start with res://"),
            ("res://player/x.txt", "The file name must end with .gd"),
            ("res://player/taken.gd", "A file with this name already exists"),
        ] {
            script.path = path.to_string();
            assert_eq!(script.validate(&godot).unwrap_err(), error);
        }
        script.path = "res://player/ok.gd".to_string();
        script.extends = "Not Valid".to_string();
        assert!(script.validate(&godot).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_extends() {
        for good in ["Node", "Node2D", "MyClass.Inner", "\"res://base.gd\"", "_Private"] {
            assert!(valid_extends(good), "{good}");
        }
        for bad in ["", "2D", "Node 2D", "res://base.gd", "\"user://x.gd\""] {
            assert!(!valid_extends(bad), "{bad}");
        }
        assert_eq!(snake_case("BigEnemy"), "big_enemy");
        assert_eq!(pascal_case("big-enemy"), "BigEnemy");
    }
}
