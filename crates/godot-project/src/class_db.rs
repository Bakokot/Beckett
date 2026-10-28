//! The engine's classes as the Inspector needs them: inheritance, the
//! properties scenes store (with type, default and enum), and enum values.
//! Read from the class reference Godot writes with `--doctool <dir>`.

use std::collections::HashMap;
use std::path::Path;

/// A property of a class, as the class reference lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    /// Variant type or class: `Vector2`, `int`, `Texture2D`…
    pub type_name: String,
    /// Raw variant text of the default; `None` for properties that aren't
    /// stored (computed ones like `global_position`).
    pub default: Option<String>,
    /// `Class.Enum` (or `Enum` of the global scope) for enum-valued ints.
    pub enumeration: Option<String>,
    /// The int is a set of flags of `enumeration`.
    pub bitfield: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassInfo {
    pub name: String,
    /// Empty for `Object` and the built-in types.
    pub inherits: String,
    /// The properties it declares, in the reference's (alphabetical) order.
    pub properties: Vec<Property>,
    /// Defaults it changes of properties declared by its ancestors.
    pub overridden: HashMap<String, String>,
    /// Its enums: name → (constant, value) in order.
    pub enums: HashMap<String, Vec<(String, i64)>>,
}

/// Every class of the reference, by name.
#[derive(Debug, Clone, Default)]
pub struct ClassDb {
    classes: HashMap<String, ClassInfo>,
}

impl ClassDb {
    /// Reads every `*.xml` under `dir` (the folder given to `--doctool`).
    pub fn load(dir: &Path) -> Self {
        let mut files = Vec::new();
        collect_xml(dir, &mut files);
        let classes = files
            .iter()
            .filter_map(|file| std::fs::read_to_string(file).ok())
            .filter_map(|text| parse_class(&text))
            .map(|class| (class.name.clone(), class))
            .collect();
        Self { classes }
    }

    pub fn from_classes(classes: impl IntoIterator<Item = ClassInfo>) -> Self {
        Self { classes: classes.into_iter().map(|c| (c.name.clone(), c)).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    pub fn class(&self, name: &str) -> Option<&ClassInfo> {
        self.classes.get(name)
    }

    /// `name` and its ancestors, nearest first.
    pub fn chain(&self, name: &str) -> Vec<&ClassInfo> {
        let mut chain: Vec<&ClassInfo> = Vec::new();
        let mut current = self.classes.get(name);
        while let Some(class) = current {
            if chain.iter().any(|c| c.name == class.name) {
                break;
            }
            chain.push(class);
            current = self.classes.get(&class.inherits);
        }
        chain
    }

    /// The default of `property` on an object of class `class`: the nearest
    /// override, else the declaring class's.
    pub fn default_of(&self, class: &str, property: &str) -> Option<String> {
        for info in self.chain(class) {
            if let Some(value) = info.overridden.get(property) {
                return Some(value.clone());
            }
            if let Some(found) = info.properties.iter().find(|p| p.name == property) {
                return found.default.clone();
            }
        }
        None
    }

    /// The constants of an enum named `Class.Enum` (or a global `Enum`).
    pub fn enum_values(&self, enumeration: &str) -> Option<&[(String, i64)]> {
        let (class, name) = enumeration.rsplit_once('.').unwrap_or(("@GlobalScope", enumeration));
        self.classes.get(class)?.enums.get(name).map(Vec::as_slice)
    }
}

fn collect_xml(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_xml(&path, out);
        } else if path.extension().is_some_and(|e| e == "xml") {
            out.push(path);
        }
    }
}

/// One class reference file.
pub fn parse_class(xml: &str) -> Option<ClassInfo> {
    let (_, class_attrs) = tags(xml, "class").next()?;
    let mut class = ClassInfo {
        name: attr(class_attrs, "name")?,
        inherits: attr(class_attrs, "inherits").unwrap_or_default(),
        ..ClassInfo::default()
    };
    for (_, attrs) in tags(xml, "member") {
        let Some(name) = attr(attrs, "name") else {
            continue;
        };
        let default = attr(attrs, "default");
        // Listed again by a class changing its parent's default.
        if attr(attrs, "overrides").is_some() {
            if let Some(default) = default {
                class.overridden.insert(name, default);
            }
            continue;
        }
        class.properties.push(Property {
            name,
            type_name: attr(attrs, "type").unwrap_or_default(),
            default,
            enumeration: attr(attrs, "enum"),
            bitfield: attr(attrs, "is_bitfield").is_some_and(|v| v == "true"),
        });
    }
    for (_, attrs) in tags(xml, "constant") {
        let (Some(name), Some(enumeration)) = (attr(attrs, "name"), attr(attrs, "enum")) else {
            continue;
        };
        let value = attr(attrs, "value").and_then(|v| v.parse().ok()).unwrap_or(0);
        class.enums.entry(enumeration).or_default().push((name, value));
    }
    Some(class)
}

/// Each opening `<tag …>` in `xml`: its offset and attribute text.
fn tags<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = (usize, &'a str)> + 'a {
    let open = format!("<{tag} ");
    let mut from = 0;
    std::iter::from_fn(move || {
        let start = from + xml[from..].find(&open)?;
        let attrs_start = start + open.len();
        let end = attrs_start + xml[attrs_start..].find('>')?;
        from = end;
        Some((start, xml[attrs_start..end].trim_end_matches('/')))
    })
}

/// The unescaped value of `key="…"` in a tag's attribute text.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let mut search = 0;
    // A whole attribute name: at the start or after a space.
    let start = loop {
        let found = search + attrs[search..].find(&needle)?;
        if found == 0 || attrs[..found].ends_with(char::is_whitespace) {
            break found + needle.len();
        }
        search = found + needle.len();
    };
    let len = attrs[start..].find('"')?;
    Some(unescape_xml(&attrs[start..start + len]))
}

fn unescape_xml(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS_ITEM: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<class name="CanvasItem" inherits="Node" api_type="core">
	<members>
		<member name="modulate" type="Color" setter="set_modulate" getter="get_modulate" default="Color(1, 1, 1, 1)">
		</member>
		<member name="texture_filter" type="int" setter="set_texture_filter" getter="get_texture_filter" enum="CanvasItem.TextureFilter" default="0">
		</member>
	</members>
	<constants>
		<constant name="NOTIFICATION_DRAW" value="30">
		</constant>
		<constant name="TEXTURE_FILTER_PARENT_NODE" value="0" enum="TextureFilter">
		</constant>
		<constant name="TEXTURE_FILTER_NEAREST" value="1" enum="TextureFilter">
		</constant>
	</constants>
</class>"#;

    const NODE2D: &str = r#"<class name="Node2D" inherits="CanvasItem">
	<members>
		<member name="global_position" type="Vector2" setter="set_global_position" getter="get_global_position">
		</member>
		<member name="position" type="Vector2" setter="set_position" getter="get_position" default="Vector2(0, 0)">
		</member>
		<member name="modulate" type="Color" setter="" getter="" overrides="CanvasItem" default="Color(1, 0, 0, 1)" />
		<member name="text" type="String" setter="" getter="" default="&quot;a &amp; b&quot;">
		</member>
	</members>
</class>"#;

    #[test]
    fn reads_members_enums_and_overrides() {
        let db = ClassDb::from_classes([parse_class(CANVAS_ITEM).unwrap(), parse_class(NODE2D).unwrap()]);
        let node2d = db.class("Node2D").unwrap();
        assert_eq!(node2d.inherits, "CanvasItem");
        let names: Vec<&str> = node2d.properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["global_position", "position", "text"]);
        assert_eq!(node2d.properties[0].default, None);
        assert_eq!(node2d.properties[2].default.as_deref(), Some("\"a & b\""));
        let chain: Vec<&str> = db.chain("Node2D").iter().map(|c| c.name.as_str()).collect();
        assert_eq!(chain, ["Node2D", "CanvasItem"]);
        assert_eq!(db.default_of("Node2D", "modulate").as_deref(), Some("Color(1, 0, 0, 1)"));
        assert_eq!(db.default_of("CanvasItem", "modulate").as_deref(), Some("Color(1, 1, 1, 1)"));
        let filter = &db.class("CanvasItem").unwrap().properties[1];
        assert_eq!(filter.enumeration.as_deref(), Some("CanvasItem.TextureFilter"));
        let values = db.enum_values("CanvasItem.TextureFilter").unwrap();
        assert_eq!(values, [("TEXTURE_FILTER_PARENT_NODE".to_string(), 0), ("TEXTURE_FILTER_NEAREST".to_string(), 1)]);
        assert_eq!(db.enum_values("CanvasItem.Nothing"), None);
    }

    /// Reads Godot's real reference: `BECKETT_DOCS=dir cargo test -p godot-project -- --ignored`.
    #[test]
    #[ignore]
    fn reads_a_real_reference() {
        let db = ClassDb::load(Path::new(&std::env::var("BECKETT_DOCS").expect("BECKETT_DOCS")));
        let chain: Vec<&str> = db.chain("Button").iter().map(|c| c.name.as_str()).collect();
        assert_eq!(chain, ["Button", "BaseButton", "Control", "CanvasItem", "Node", "Object"]);
        assert_eq!(db.default_of("Node2D", "position").as_deref(), Some("Vector2(0, 0)"));
        let flags = db.chain("Control")[0].properties.iter().find(|p| p.name == "size_flags_horizontal").unwrap();
        assert!(flags.bitfield);
        assert!(db.enum_values("Control.SizeFlags").is_some_and(|v| v.len() >= 4));
        eprintln!("{} classes", db.classes.len());
    }
}
