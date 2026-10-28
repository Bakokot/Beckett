//! Reader and writer for Godot's text scene format (`.tscn`), and the edits
//! of the scene tree editor: add, rename, remove and reparent nodes, attach
//! and detach scripts.
//!
//! Sections keep the text they were read with; only the ones an edit
//! touches are written back in Godot's layout, so saving an unchanged scene
//! gives the same bytes. Values stay raw Godot variant text.

use std::collections::HashSet;
use std::fmt;
use std::hash::BuildHasher;
use std::io;
use std::path::Path;

/// One `[tag attr=value …]` section and the `key = value` lines under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Blank lines (and anything else outside a section) before the header.
    leading: String,
    pub tag: String,
    /// Header attributes as raw variant text, in file order.
    attrs: Vec<(String, String)>,
    /// Properties as raw variant text, in file order.
    props: Vec<(String, String)>,
    /// The header and properties as read; dropped once the section is edited.
    original: Option<String>,
}

impl Section {
    fn new(tag: &str, attrs: Vec<(String, String)>, leading: &str) -> Self {
        Self { leading: leading.to_string(), tag: tag.to_string(), attrs, props: Vec::new(), original: None }
    }

    /// Raw text of a header attribute.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// A header attribute that holds a string, unquoted.
    pub fn attr_string(&self, key: &str) -> Option<String> {
        self.attr(key).and_then(unquote)
    }

    /// Raw text of a property.
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn props(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn set_attr(&mut self, key: &str, raw: String) {
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) if *v == raw => return,
            Some((_, v)) => *v = raw,
            None => self.attrs.push((key.to_string(), raw)),
        }
        self.original = None;
    }

    fn remove_attr(&mut self, key: &str) {
        let len = self.attrs.len();
        self.attrs.retain(|(k, _)| k != key);
        if self.attrs.len() != len {
            self.original = None;
        }
    }

    /// Sets a property; a new one goes first.
    fn set_prop(&mut self, key: &str, raw: String) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) if *v == raw => return,
            Some((_, v)) => *v = raw,
            None => self.props.insert(0, (key.to_string(), raw)),
        }
        self.original = None;
    }

    fn remove_prop(&mut self, key: &str) {
        let len = self.props.len();
        self.props.retain(|(k, _)| k != key);
        if self.props.len() != len {
            self.original = None;
        }
    }

    /// The header and properties, as read or in Godot's layout once edited.
    fn body(&self) -> String {
        if let Some(original) = &self.original {
            return original.clone();
        }
        let mut out = format!("[{}", self.tag);
        for (key, value) in &self.attrs {
            out.push_str(&format!(" {key}={value}"));
        }
        out.push_str("]\n");
        for (key, value) in &self.props {
            out.push_str(&format!("{key} = {value}\n"));
        }
        out
    }
}

/// A node of the scene tree, as listed by [`Scene::nodes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNode {
    /// Index of its `[node]` section.
    pub section: usize,
    pub name: String,
    /// Path from the root: `.` for the root, `Player/Sprite` below it.
    pub path: String,
    /// Path of the parent; `None` for the root.
    pub parent: Option<String>,
    /// Number of ancestors (0 for the root).
    pub depth: usize,
    /// Class, e.g. `CharacterBody2D`; `None` for instanced scenes and for
    /// nodes of an instanced scene whose properties this scene overrides.
    pub type_name: Option<String>,
    /// `res://` path of the instanced scene.
    pub instance: Option<String>,
    /// `res://` path of the attached script.
    pub script: Option<String>,
}

impl SceneNode {
    /// Nodes that come from an instanced scene (only overridden here) can't
    /// be renamed, moved or removed.
    pub fn is_editable(&self) -> bool {
        self.type_name.is_some() || self.instance.is_some() || self.parent.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// Text that isn't a section or a `key = value` line, at a zero-based line.
    Syntax(usize),
    NoSuchNode(String),
    /// Empty, or with one of `. : @ / " %`.
    InvalidName(String),
    /// A sibling already has the name.
    NameTaken(String),
    /// The root can't be removed or moved.
    Root,
    /// The node comes from an instanced scene.
    Inherited(String),
    /// A node can't be moved under itself or its descendants.
    IntoItself,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::Syntax(line) => write!(f, "unexpected text on line {}", line + 1),
            SceneError::NoSuchNode(path) => write!(f, "no node at \"{path}\""),
            SceneError::InvalidName(name) => write!(f, "\"{name}\" is not a valid node name"),
            SceneError::NameTaken(name) => write!(f, "a sibling is already named \"{name}\""),
            SceneError::Root => write!(f, "the root node can't be removed or moved"),
            SceneError::Inherited(path) => write!(f, "\"{path}\" comes from an instanced scene"),
            SceneError::IntoItself => write!(f, "a node can't be moved under itself"),
        }
    }
}

impl std::error::Error for SceneError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    /// The `[gd_scene]` header first, then resources, nodes, connections.
    sections: Vec<Section>,
    /// Text after the last section.
    trailing: String,
    crlf: bool,
}

impl Scene {
    pub fn parse(text: &str) -> Result<Self, SceneError> {
        let crlf = text.contains("\r\n");
        let text = if crlf { text.replace("\r\n", "\n") } else { text.to_string() };
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut sections: Vec<Section> = Vec::new();
        let mut pending = String::new();
        let mut i = 0;
        while i < lines.len() {
            let start = i;
            let trimmed = lines[i].trim();
            if trimmed.is_empty() || trimmed.starts_with(';') {
                pending.push_str(lines[i]);
                i += 1;
                continue;
            }
            // A section header or a property, possibly over several lines.
            let mut chunk = String::from(lines[i]);
            let mut nesting = Nesting::default();
            nesting.feed_str(lines[i]);
            i += 1;
            while i < lines.len() && nesting.is_open() {
                chunk.push_str(lines[i]);
                nesting.feed_str(lines[i]);
                i += 1;
            }
            if trimmed.starts_with('[') {
                let (tag, attrs) = parse_header(&chunk).ok_or(SceneError::Syntax(start))?;
                sections.push(Section {
                    leading: std::mem::take(&mut pending),
                    tag,
                    attrs,
                    props: Vec::new(),
                    original: Some(chunk),
                });
            } else {
                let section = sections.last_mut().ok_or(SceneError::Syntax(start))?;
                let (key, value) = chunk.split_once('=').ok_or(SceneError::Syntax(start))?;
                section.props.push((key.trim().to_string(), value.trim().to_string()));
                let original = section.original.get_or_insert_with(String::new);
                original.push_str(&std::mem::take(&mut pending));
                original.push_str(&chunk);
            }
        }
        Ok(Self { sections, trailing: pending, crlf })
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_text())
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            out.push_str(&section.leading);
            out.push_str(&section.body());
        }
        out.push_str(&self.trailing);
        if self.crlf {
            out = out.replace('\n', "\r\n");
        }
        out
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Every node, in file (tree) order.
    pub fn nodes(&self) -> Vec<SceneNode> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.tag == "node")
            .map(|(i, s)| self.node_at(i, s))
            .collect()
    }

    /// The zero-based line each section's header is on, by section index.
    pub fn section_lines(&self) -> Vec<usize> {
        let mut line = 0;
        self.sections
            .iter()
            .map(|section| {
                line += section.leading.matches('\n').count();
                let start = line;
                line += section.body().matches('\n').count();
                start
            })
            .collect()
    }

    pub fn node(&self, path: &str) -> Option<SceneNode> {
        let i = self.find(path)?;
        Some(self.node_at(i, &self.sections[i]))
    }

    fn node_at(&self, index: usize, section: &Section) -> SceneNode {
        let name = section.attr_string("name").unwrap_or_default();
        let parent = section.attr_string("parent");
        let path = match parent.as_deref() {
            None => ".".to_string(),
            Some(".") => name.clone(),
            Some(parent) => format!("{parent}/{name}"),
        };
        let depth = match parent.as_deref() {
            None => 0,
            Some(".") => 1,
            Some(parent) => parent.split('/').count() + 1,
        };
        let ext_path = |raw: Option<&str>| raw.and_then(ext_id).and_then(|id| self.ext_resource_path(&id));
        SceneNode {
            section: index,
            name,
            path,
            parent,
            depth,
            type_name: section.attr_string("type"),
            instance: ext_path(section.attr("instance")),
            script: ext_path(section.prop("script")),
        }
    }

    /// Paths of the direct children of the node at `path`, in order.
    pub fn children(&self, path: &str) -> Vec<String> {
        self.nodes().into_iter().filter(|n| n.parent.as_deref() == Some(path)).map(|n| n.path).collect()
    }

    /// `res://` path of the `[ext_resource]` with this id.
    pub fn ext_resource_path(&self, id: &str) -> Option<String> {
        self.sections
            .iter()
            .find(|s| s.tag == "ext_resource" && s.attr_string("id").as_deref() == Some(id))
            .and_then(|s| s.attr_string("path"))
    }

    fn find(&self, path: &str) -> Option<usize> {
        self.nodes().into_iter().find(|n| n.path == path).map(|n| n.section)
    }

    fn editable_node(&self, path: &str) -> Result<SceneNode, SceneError> {
        let node = self.node(path).ok_or_else(|| SceneError::NoSuchNode(path.to_string()))?;
        if !node.is_editable() {
            return Err(SceneError::Inherited(path.to_string()));
        }
        Ok(node)
    }

    /// Index just past the node at `index` and its descendants.
    fn subtree_end(&self, index: usize) -> usize {
        let path = self.node_at(index, &self.sections[index]).path;
        let mut end = index + 1;
        while end < self.sections.len()
            && self.sections[end].tag == "node"
            && is_within(&self.node_at(end, &self.sections[end]).path, &path)
        {
            end += 1;
        }
        end
    }
}

impl Scene {
    /// Adds a `type_name` node as the last child of `parent`; a taken name
    /// gets a number, as Godot does. Returns the new node's path.
    pub fn add_node(&mut self, parent: &str, name: &str, type_name: &str) -> Result<String, SceneError> {
        validate_name(name)?;
        let index = self.find(parent).ok_or_else(|| SceneError::NoSuchNode(parent.to_string()))?;
        let name = self.unique_name(parent, name);
        let mut attrs = vec![
            ("name".to_string(), quote(&name)),
            ("type".to_string(), quote(type_name)),
            ("parent".to_string(), quote(parent)),
        ];
        if let Some(id) = self.new_unique_id() {
            attrs.push(("unique_id".to_string(), id.to_string()));
        }
        let at = self.subtree_end(index);
        self.sections.insert(at, Section::new("node", attrs, "\n"));
        Ok(child_path(parent, &name))
    }

    /// Renames a node, updating the paths that lead through it. Returns the
    /// new path.
    pub fn rename_node(&mut self, path: &str, new_name: &str) -> Result<String, SceneError> {
        validate_name(new_name)?;
        let node = self.editable_node(path)?;
        let Some(parent) = node.parent else {
            self.sections[node.section].set_attr("name", quote(new_name));
            return Ok(".".to_string());
        };
        if node.name == new_name {
            return Ok(node.path);
        }
        if self.children(&parent).iter().any(|p| *p == child_path(&parent, new_name)) {
            return Err(SceneError::NameTaken(new_name.to_string()));
        }
        self.sections[node.section].set_attr("name", quote(new_name));
        let new_path = child_path(&parent, new_name);
        self.rewrite_paths(path, &new_path);
        Ok(new_path)
    }

    /// Removes a node and its descendants, with their signal connections and
    /// the resources only they used.
    pub fn remove_node(&mut self, path: &str) -> Result<(), SceneError> {
        let node = self.editable_node(path)?;
        if node.parent.is_none() {
            return Err(SceneError::Root);
        }
        let end = self.subtree_end(node.section);
        self.remove_sections(node.section..end);
        self.retain_sections(|s| match s.tag.as_str() {
            "connection" => ["from", "to"]
                .iter()
                .all(|key| s.attr_string(key).is_none_or(|p| !is_within(&p, path))),
            "editable" => s.attr_string("path").is_none_or(|p| !is_within(&p, path)),
            _ => true,
        });
        self.prune_resources();
        Ok(())
    }

    /// Moves a node (with its descendants) under `new_parent`, at `position`
    /// among the other children there (last when `None` or past the end).
    /// A taken name gets a number. Returns the new path.
    pub fn reparent_node(
        &mut self,
        path: &str,
        new_parent: &str,
        position: Option<usize>,
    ) -> Result<String, SceneError> {
        let node = self.editable_node(path)?;
        let Some(old_parent) = node.parent.clone() else {
            return Err(SceneError::Root);
        };
        if self.find(new_parent).is_none() {
            return Err(SceneError::NoSuchNode(new_parent.to_string()));
        }
        if is_within(new_parent, path) {
            return Err(SceneError::IntoItself);
        }
        let name = if old_parent == new_parent {
            node.name.clone()
        } else {
            self.unique_name(new_parent, &node.name)
        };
        let new_path = child_path(new_parent, &name);

        let end = self.subtree_end(node.section);
        let mut block: Vec<Section> = self.sections.drain(node.section..end).collect();
        let parent_index = self.find(new_parent).expect("the new parent is outside the moved block");
        let siblings = self.children(new_parent);
        let at = match position.and_then(|p| siblings.get(p)) {
            Some(sibling) => self.find(sibling).expect("listed child"),
            None => self.subtree_end(parent_index),
        };
        block[0].set_attr("name", quote(&name));
        block[0].set_attr("parent", quote(new_parent));
        if block[0].leading.is_empty() {
            block[0].leading = "\n".to_string();
        }
        self.sections.splice(at..at, block);
        if new_path != path {
            self.rewrite_paths(path, &new_path);
        }
        Ok(new_path)
    }

    /// Attaches the script at `res_path` (with its `uid://`, when it has one)
    /// to a node, replacing any script it had.
    pub fn attach_script(&mut self, path: &str, res_path: &str, uid: Option<&str>) -> Result<(), SceneError> {
        if self.find(path).is_none() {
            return Err(SceneError::NoSuchNode(path.to_string()));
        }
        let id = self.ext_resource("Script", res_path, uid);
        // Looked up again: adding the resource shifted the nodes.
        let index = self.find(path).expect("checked above");
        self.sections[index].set_prop("script", format!("ExtResource({})", quote(&id)));
        self.prune_resources();
        Ok(())
    }

    pub fn detach_script(&mut self, path: &str) -> Result<(), SceneError> {
        let index = self.find(path).ok_or_else(|| SceneError::NoSuchNode(path.to_string()))?;
        self.sections[index].remove_prop("script");
        self.prune_resources();
        Ok(())
    }
}

impl Scene {
    /// The id of the `[ext_resource]` for `res_path`, added after the others
    /// if the scene doesn't load it yet.
    fn ext_resource(&mut self, type_name: &str, res_path: &str, uid: Option<&str>) -> String {
        let existing = self.sections.iter().find(|s| {
            s.tag == "ext_resource" && s.attr_string("path").as_deref() == Some(res_path)
        });
        if let Some(id) = existing.and_then(|s| s.attr_string("id")) {
            return id;
        }
        let ids: HashSet<String> = self
            .sections
            .iter()
            .filter(|s| s.tag == "ext_resource")
            .filter_map(|s| s.attr_string("id"))
            .collect();
        let id = loop {
            let id = format!("{}_{}", ids.len() + 1, random_suffix());
            if !ids.contains(&id) {
                break id;
            }
        };
        let mut attrs = vec![("type".to_string(), quote(type_name))];
        if let Some(uid) = uid {
            attrs.push(("uid".to_string(), quote(uid)));
        }
        attrs.push(("path".to_string(), quote(res_path)));
        attrs.push(("id".to_string(), quote(&id)));
        // Right after the others, with no blank line, or after the header.
        let (at, leading) = match self.sections.iter().rposition(|s| s.tag == "ext_resource") {
            Some(i) => (i + 1, ""),
            None => (1.min(self.sections.len()), "\n"),
        };
        self.sections.insert(at, Section::new("ext_resource", attrs, leading));
        id
    }

    /// Drops the sub- and external resources nothing refers to any more, and
    /// keeps the header's `load_steps` in step.
    fn prune_resources(&mut self) {
        loop {
            let bodies: Vec<String> = self.sections.iter().map(Section::body).collect();
            let unused = self.sections.iter().enumerate().position(|(i, s)| {
                let reference = match s.tag.as_str() {
                    "sub_resource" => format!("SubResource({})", s.attr("id").unwrap_or_default()),
                    "ext_resource" => format!("ExtResource({})", s.attr("id").unwrap_or_default()),
                    _ => return false,
                };
                !bodies.iter().enumerate().any(|(j, body)| j != i && body.contains(&reference))
            });
            match unused {
                Some(i) => self.remove_sections(i..i + 1),
                None => break,
            }
        }
        let resources = self.sections.iter().filter(|s| matches!(s.tag.as_str(), "ext_resource" | "sub_resource"));
        let steps = resources.count() + 1;
        if let Some(header) = self.sections.first_mut().filter(|s| s.attr("load_steps").is_some()) {
            if steps > 1 {
                header.set_attr("load_steps", steps.to_string());
            } else {
                header.remove_attr("load_steps");
            }
        }
    }

    /// Removes sections, giving their blank lines to the one that follows
    /// when it has none (e.g. the next of a run of `[ext_resource]`s).
    fn remove_sections(&mut self, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let leading = self.sections[range.start].leading.clone();
        self.sections.drain(range.clone());
        if let Some(next) = self.sections.get_mut(range.start) {
            if next.leading.is_empty() {
                next.leading = leading;
            }
        }
    }

    fn retain_sections(&mut self, keep: impl Fn(&Section) -> bool) {
        let mut i = 0;
        while i < self.sections.len() {
            if keep(&self.sections[i]) {
                i += 1;
            } else {
                self.remove_sections(i..i + 1);
            }
        }
    }

    /// Points node parents, connections and editable children that lead
    /// through `old` at `new`.
    fn rewrite_paths(&mut self, old: &str, new: &str) {
        for section in &mut self.sections {
            let keys: &[&str] = match section.tag.as_str() {
                "node" => &["parent"],
                "connection" => &["from", "to"],
                "editable" => &["path"],
                _ => continue,
            };
            for key in keys {
                if let Some(path) = section.attr_string(key) {
                    if let Some(rest) = path.strip_prefix(old).filter(|r| r.is_empty() || r.starts_with('/')) {
                        section.set_attr(key, quote(&format!("{new}{rest}")));
                    }
                }
            }
        }
    }

    /// `name`, or `name` with a number when a child of `parent` has it:
    /// `Sprite2D` → `Sprite2D2`, `Enemy7` → `Enemy8`.
    fn unique_name(&self, parent: &str, name: &str) -> String {
        let taken: HashSet<String> = self
            .nodes()
            .into_iter()
            .filter(|n| n.parent.as_deref() == Some(parent))
            .map(|n| n.name)
            .collect();
        if !taken.contains(name) {
            return name.to_string();
        }
        let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
        let mut n: u64 = name[base.len()..].parse().map_or(2, |n: u64| n + 1);
        loop {
            let candidate = format!("{base}{n}");
            if !taken.contains(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// A fresh `unique_id`, when the scene's nodes have them (Godot 4.6+).
    fn new_unique_id(&self) -> Option<u32> {
        let used: HashSet<u32> = self
            .sections
            .iter()
            .filter_map(|s| s.attr("unique_id"))
            .filter_map(|id| id.parse().ok())
            .collect();
        if used.is_empty() {
            return None;
        }
        loop {
            let id = (random() as u32) & 0x7fff_ffff;
            if id != 0 && !used.contains(&id) {
                return Some(id);
            }
        }
    }
}

fn child_path(parent: &str, name: &str) -> String {
    if parent == "." {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Whether `path` is `ancestor` or below it.
fn is_within(path: &str, ancestor: &str) -> bool {
    ancestor == "."
        || path
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn validate_name(name: &str) -> Result<(), SceneError> {
    if name.is_empty() || name.contains(['.', ':', '@', '/', '"', '%']) {
        return Err(SceneError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// The id in `ExtResource("id")`.
fn ext_id(raw: &str) -> Option<String> {
    unquote(raw.trim().strip_prefix("ExtResource(")?.strip_suffix(')')?)
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new().hash_one(std::time::SystemTime::now())
}

/// Five lowercase letters and digits, like the ids Godot makes.
fn random_suffix() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut n = random();
    (0..5)
        .map(|_| {
            let c = CHARS[(n % CHARS.len() as u64) as usize] as char;
            n /= CHARS.len() as u64;
            c
        })
        .collect()
}

/// Strings and brackets open so far in variant text, fed piece by piece.
#[derive(Default)]
struct Nesting {
    depth: i32,
    in_string: bool,
    escaped: bool,
}

impl Nesting {
    fn feed(&mut self, c: char) {
        if self.in_string {
            match c {
                _ if self.escaped => self.escaped = false,
                '\\' => self.escaped = true,
                '"' => self.in_string = false,
                _ => {}
            }
            return;
        }
        match c {
            '"' => self.in_string = true,
            '(' | '[' | '{' => self.depth += 1,
            ')' | ']' | '}' => self.depth -= 1,
            _ => {}
        }
    }

    fn feed_str(&mut self, text: &str) {
        text.chars().for_each(|c| self.feed(c));
    }

    fn is_open(&self) -> bool {
        self.in_string || self.depth > 0
    }
}

/// `[tag key=value …]` → tag and attributes (values raw).
fn parse_header(text: &str) -> Option<(String, Vec<(String, String)>)> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let tag_len = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let tag = inner[..tag_len].to_string();
    let mut rest = inner[tag_len..].trim_start();
    let mut attrs = Vec::new();
    while !rest.is_empty() {
        let eq = rest.find('=')?;
        let key = rest[..eq].trim().to_string();
        let value = &rest[eq + 1..];
        let len = value_len(value);
        attrs.push((key, value[..len].to_string()));
        rest = value[len..].trim_start();
    }
    Some((tag, attrs))
}

/// Length of the raw value at the start of `text`: up to whitespace outside
/// strings and brackets.
fn value_len(text: &str) -> usize {
    let mut nesting = Nesting::default();
    for (i, c) in text.char_indices() {
        if c.is_whitespace() && !nesting.is_open() {
            return i;
        }
        nesting.feed(c);
    }
    text.len()
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"[gd_scene load_steps=4 format=3 uid="uid://b1"]

[ext_resource type="Script" uid="uid://c2" path="res://player.gd" id="1_abcde"]
[ext_resource type="PackedScene" path="res://gun.tscn" id="2_fghij"]

[sub_resource type="RectangleShape2D" id="RectangleShape2D_x"]
size = Vector2(10, 20)

[node name="Main" type="Node2D"]

[node name="Player" type="CharacterBody2D" parent="."]
script = ExtResource("1_abcde")

[node name="Shape" type="CollisionShape2D" parent="Player"]
shape = SubResource("RectangleShape2D_x")

[node name="Gun" parent="Player" instance=ExtResource("2_fghij")]

[node name="Muzzle" parent="Player/Gun" index="0"]
position = Vector2(4, 0)

[node name="Label" type="Label" parent="."]
text = "two
[lines]"

[connection signal="fired" from="Player/Gun" to="." method="_on_fired"]

[editable path="Player/Gun"]
"#;

    fn scene() -> Scene {
        Scene::parse(SCENE).unwrap()
    }

    fn paths(scene: &Scene) -> Vec<String> {
        scene.nodes().into_iter().map(|n| n.path).collect()
    }

    #[test]
    fn round_trips_unchanged() {
        assert_eq!(scene().to_text(), SCENE);
        let crlf = SCENE.replace('\n', "\r\n");
        assert_eq!(Scene::parse(&crlf).unwrap().to_text(), crlf);
        let odd = "[gd_scene format=3]\n\n; note\n[node name=\"A\" type=\"Node\"]\n\nx = 1\n\n\n";
        assert_eq!(Scene::parse(odd).unwrap().to_text(), odd);
        assert_eq!(Scene::parse("key = 1\n"), Err(SceneError::Syntax(0)));
    }

    #[test]
    fn lists_the_tree() {
        let s = scene();
        assert_eq!(paths(&s), [".", "Player", "Player/Shape", "Player/Gun", "Player/Gun/Muzzle", "Label"]);
        let player = s.node("Player").unwrap();
        assert_eq!(player.script.as_deref(), Some("res://player.gd"));
        assert_eq!(player.depth, 1);
        let gun = s.node("Player/Gun").unwrap();
        assert_eq!(gun.instance.as_deref(), Some("res://gun.tscn"));
        assert!(gun.is_editable());
        assert!(!s.node("Player/Gun/Muzzle").unwrap().is_editable());
        assert_eq!(s.node("Label").unwrap().section, 9);
        assert_eq!(s.sections()[9].prop("text"), Some("\"two\n[lines]\""));
        assert_eq!(s.children("Player"), ["Player/Shape", "Player/Gun"]);
        let lines = s.section_lines();
        assert_eq!((lines[0], lines[1], lines[2], lines[3], lines[4], lines[9]), (0, 2, 3, 5, 8, 21));
    }

    #[test]
    fn adds_nodes_with_free_names() {
        let mut s = scene();
        assert_eq!(s.add_node("Player", "Shape", "Area2D").unwrap(), "Player/Shape2");
        assert_eq!(s.add_node(".", "Label", "Label").unwrap(), "Label2");
        assert_eq!(s.add_node("Player", "Hit/box", "Area2D"), Err(SceneError::InvalidName("Hit/box".into())));
        assert_eq!(
            paths(&s),
            [".", "Player", "Player/Shape", "Player/Gun", "Player/Gun/Muzzle", "Player/Shape2", "Label", "Label2"]
        );
        let text = s.to_text();
        assert!(text.contains(
            "position = Vector2(4, 0)\n\n[node name=\"Shape2\" type=\"Area2D\" parent=\"Player\"]\n\n[node name=\"Label\""
        ));
        assert!(text.ends_with("[lines]\"\n\n[node name=\"Label2\" type=\"Label\" parent=\".\"]\n\n[connection signal=\"fired\" from=\"Player/Gun\" to=\".\" method=\"_on_fired\"]\n\n[editable path=\"Player/Gun\"]\n"));
    }

    #[test]
    fn new_nodes_get_unique_ids_when_the_scene_uses_them() {
        let mut s = Scene::parse("[gd_scene format=3]\n\n[node name=\"A\" type=\"Node\" unique_id=5]\n").unwrap();
        s.add_node(".", "B", "Node").unwrap();
        assert!(s.sections()[2].attr("unique_id").is_some_and(|id| id.parse::<u32>().is_ok()));
        let mut s = Scene::parse("[gd_scene format=3]\n\n[node name=\"A\" type=\"Node\"]\n").unwrap();
        s.add_node(".", "B", "Node").unwrap();
        assert_eq!(s.sections()[2].attr("unique_id"), None);
    }

    #[test]
    fn renames_and_follows_paths() {
        let mut s = scene();
        assert_eq!(s.rename_node("Player", "Hero").unwrap(), "Hero");
        assert_eq!(paths(&s), [".", "Hero", "Hero/Shape", "Hero/Gun", "Hero/Gun/Muzzle", "Label"]);
        let text = s.to_text();
        assert!(text.contains("[connection signal=\"fired\" from=\"Hero/Gun\" to=\".\""));
        assert!(text.contains("[editable path=\"Hero/Gun\"]"));
        assert_eq!(s.rename_node("Hero", "Label"), Err(SceneError::NameTaken("Label".into())));
        assert_eq!(s.rename_node("Hero/Gun/Muzzle", "Tip"), Err(SceneError::Inherited("Hero/Gun/Muzzle".into())));
        assert_eq!(s.rename_node(".", "World").unwrap(), ".");
        assert!(s.to_text().contains("[node name=\"World\" type=\"Node2D\"]"));
    }

    #[test]
    fn removes_subtrees_with_their_connections_and_resources() {
        let mut s = scene();
        s.remove_node("Player/Gun").unwrap();
        assert_eq!(paths(&s), [".", "Player", "Player/Shape", "Label"]);
        let text = s.to_text();
        assert!(!text.contains("gun.tscn") && !text.contains("[connection") && !text.contains("[editable"));
        assert!(text.starts_with("[gd_scene load_steps=3 format=3 uid=\"uid://b1\"]\n\n[ext_resource type=\"Script\""));

        s.remove_node("Player").unwrap();
        assert_eq!(
            s.to_text(),
            "[gd_scene format=3 uid=\"uid://b1\"]\n\n[node name=\"Main\" type=\"Node2D\"]\n\n\
             [node name=\"Label\" type=\"Label\" parent=\".\"]\ntext = \"two\n[lines]\"\n"
        );
        assert_eq!(s.remove_node("."), Err(SceneError::Root));
        assert_eq!(s.remove_node("Nope"), Err(SceneError::NoSuchNode("Nope".into())));
    }

    #[test]
    fn removing_the_first_resource_of_a_run_keeps_the_blank_line() {
        let mut s = scene();
        s.detach_script("Player").unwrap();
        assert!(s.to_text().starts_with(
            "[gd_scene load_steps=3 format=3 uid=\"uid://b1\"]\n\n[ext_resource type=\"PackedScene\" path=\"res://gun.tscn\" id=\"2_fghij\"]\n\n[sub_resource"
        ));
    }

    #[test]
    fn reparents_and_reorders() {
        let mut s = scene();
        assert_eq!(s.reparent_node("Label", "Player/Gun", None).unwrap(), "Player/Gun/Label");
        assert_eq!(paths(&s), [".", "Player", "Player/Shape", "Player/Gun", "Player/Gun/Muzzle", "Player/Gun/Label"]);

        let mut s = scene();
        assert_eq!(s.reparent_node("Player", ".", Some(1)).unwrap(), "Player");
        assert_eq!(paths(&s), [".", "Label", "Player", "Player/Shape", "Player/Gun", "Player/Gun/Muzzle"]);
        assert_eq!(s.reparent_node("Player/Gun", "Label", Some(0)).unwrap(), "Label/Gun");
        assert_eq!(paths(&s), [".", "Label", "Label/Gun", "Label/Gun/Muzzle", "Player", "Player/Shape"]);
        let text = s.to_text();
        assert!(text.contains("[node name=\"Muzzle\" parent=\"Label/Gun\" index=\"0\"]"));
        assert!(text.contains("from=\"Label/Gun\""));
        assert!(text.contains("[editable path=\"Label/Gun\"]"));

        let mut s = scene();
        s.add_node(".", "Shape", "Node2D").unwrap();
        assert_eq!(s.reparent_node("Shape", "Player", None).unwrap(), "Player/Shape2");
        assert_eq!(s.reparent_node("Player", "Player/Shape", None), Err(SceneError::IntoItself));
        assert_eq!(s.reparent_node(".", "Player", None), Err(SceneError::Root));
    }

    #[test]
    fn attaches_and_detaches_scripts() {
        let mut s = scene();
        s.attach_script("Label", "res://label.gd", Some("uid://d3")).unwrap();
        let label = s.node("Label").unwrap();
        assert_eq!(label.script.as_deref(), Some("res://label.gd"));
        let text = s.to_text();
        assert!(text.starts_with("[gd_scene load_steps=5 format=3"));
        let id = ext_id(s.sections()[label.section].prop("script").unwrap()).unwrap();
        assert!(id.starts_with("3_") && id.len() == 7, "{id}");
        assert!(text.contains(&format!(
            "id=\"2_fghij\"]\n[ext_resource type=\"Script\" uid=\"uid://d3\" path=\"res://label.gd\" id=\"{id}\"]\n\n[sub_resource"
        )));
        assert!(text.contains(&format!("[node name=\"Label\" type=\"Label\" parent=\".\"]\nscript = ExtResource(\"{id}\")\ntext")));

        // The same script again reuses its resource; replacing drops the old one.
        s.attach_script(".", "res://label.gd", None).unwrap();
        s.attach_script("Player", "res://label.gd", None).unwrap();
        assert!(!s.to_text().contains("res://player.gd"));
        assert_eq!(s.to_text().matches("res://label.gd").count(), 1);
        s.detach_script(".").unwrap();
        assert_eq!(s.node(".").unwrap().script, None);
    }

    #[test]
    fn attaching_to_a_scene_without_resources() {
        let mut s = Scene::parse("[gd_scene format=3]\n\n[node name=\"A\" type=\"Node\"]\n").unwrap();
        s.attach_script(".", "res://a.gd", None).unwrap();
        let id = ext_id(s.sections()[2].prop("script").unwrap()).unwrap();
        assert_eq!(
            s.to_text(),
            format!(
                "[gd_scene format=3]\n\n[ext_resource type=\"Script\" path=\"res://a.gd\" id=\"{id}\"]\n\n\
                 [node name=\"A\" type=\"Node\"]\nscript = ExtResource(\"{id}\")\n"
            )
        );
    }

    /// Reads every `.tscn` under `BECKETT_SCENES` and checks it is written
    /// back unchanged: `BECKETT_SCENES=path cargo test -p godot-project -- --ignored`.
    #[test]
    #[ignore]
    fn round_trips_real_scenes() {
        let root = std::env::var("BECKETT_SCENES").expect("BECKETT_SCENES");
        let mut scenes = Vec::new();
        collect(Path::new(&root), &mut scenes);
        assert!(!scenes.is_empty());
        for path in &scenes {
            let text = std::fs::read_to_string(path).unwrap();
            let scene = Scene::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(scene.to_text() == text, "{} changed", path.display());
            let _ = scene.nodes();
        }
        eprintln!("{} scenes round-tripped", scenes.len());
    }

    fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
                collect(&path, out);
            } else if path.extension().is_some_and(|e| e == "tscn") {
                out.push(path);
            }
        }
    }
}
