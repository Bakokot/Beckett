//! Dockable views: each view (files, outline, terminal…) lives in the left
//! sidebar, the bottom panel or the right sidebar. Right-clicking a view's
//! tab moves it; where views were moved is kept in the settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    Files,
    Outline,
    /// The node tree of the scene in the active tab.
    Scene,
    /// Properties of the node under the cursor in the active scene.
    Inspector,
    /// Main scene, autoloads and Input Map.
    Project,
    Problems,
    Output,
    Terminal,
    Search,
    /// Files changed since the staged text, with their hunks.
    Changes,
    /// The debug session: call stack and variables.
    Debug,
}

impl View {
    /// In tab order.
    pub const ALL: [View; 11] = [
        View::Files,
        View::Outline,
        View::Scene,
        View::Inspector,
        View::Project,
        View::Problems,
        View::Output,
        View::Terminal,
        View::Search,
        View::Changes,
        View::Debug,
    ];

    /// Where the view is until it is moved.
    pub fn home(self) -> Dock {
        match self {
            View::Files | View::Outline | View::Scene | View::Project => Dock::Left,
            View::Inspector => Dock::Right,
            _ => Dock::Bottom,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            View::Files => "Files",
            View::Outline => "Outline",
            View::Scene => "Scene",
            View::Inspector => "Inspector",
            View::Project => "Project",
            View::Problems => "Problems",
            View::Output => "Output",
            View::Terminal => "Terminal",
            View::Search => "Search",
            View::Changes => "Changes",
            View::Debug => "Debug",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            View::Files => "Project files",
            View::Outline => "Functions, variables and signals of the current file (Ctrl+Shift+O to search them)",
            View::Scene => "Nodes of the scene in the active tab: right-click one to add, rename, move or delete nodes",
            View::Inspector => "Properties of the node under the cursor in the active scene",
            View::Project => "Main scene, autoloads and Input Map of project.godot",
            View::Problems => "Errors and warnings",
            View::Output => "What the running game prints",
            View::Terminal => "Shells, opened in the project folder",
            View::Search => "Project search, references and replace",
            View::Changes => "Files changed since the staged text (git)",
            View::Debug => "Call stack, variables and watches",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Left,
    Bottom,
    Right,
}

impl Dock {
    pub const ALL: [Dock; 3] = [Dock::Left, Dock::Bottom, Dock::Right];

    pub fn name(self) -> &'static str {
        match self {
            Dock::Left => "Left Sidebar",
            Dock::Bottom => "Bottom Panel",
            Dock::Right => "Right Sidebar",
        }
    }
}

/// Where each view is docked; only views moved from their home are stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Layout(BTreeMap<View, Dock>);

impl Layout {
    pub fn dock(&self, view: View) -> Dock {
        self.0.get(&view).copied().unwrap_or(view.home())
    }

    /// The views docked in `dock`, in tab order.
    pub fn views(&self, dock: Dock) -> Vec<View> {
        View::ALL.into_iter().filter(|&v| self.dock(v) == dock).collect()
    }

    pub fn move_to(&mut self, view: View, dock: Dock) {
        if dock == view.home() {
            self.0.remove(&view);
        } else {
            self.0.insert(view, dock);
        }
    }

    pub fn is_default(&self) -> bool {
        self.0.is_empty()
    }
}

/// The view a dock shows: `wanted` when it is docked there, else (for the
/// sidebars, always open while they hold a view) their first view.
pub fn shown_in(layout: &Layout, dock: Dock, wanted: Option<View>) -> Option<View> {
    let here = wanted.filter(|&v| layout.dock(v) == dock);
    match dock {
        Dock::Left | Dock::Right => here.or_else(|| layout.views(dock).first().copied()),
        Dock::Bottom => here,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_move_and_come_home() {
        let mut layout = Layout::default();
        assert_eq!(layout.views(Dock::Left), [View::Files, View::Outline, View::Scene, View::Project]);
        assert_eq!(layout.views(Dock::Right), [View::Inspector]);

        layout.move_to(View::Outline, Dock::Right);
        layout.move_to(View::Terminal, Dock::Right);
        assert_eq!(layout.views(Dock::Right), [View::Outline, View::Inspector, View::Terminal]);
        assert_eq!(layout.views(Dock::Left), [View::Files, View::Scene, View::Project]);

        layout.move_to(View::Outline, Dock::Left);
        layout.move_to(View::Terminal, Dock::Bottom);
        assert!(layout.is_default());
    }

    #[test]
    fn layouts_are_kept_by_name() {
        let mut layout = Layout::default();
        layout.move_to(View::Terminal, Dock::Right);
        let text = serde_json::to_string(&layout).unwrap();
        assert_eq!(text, r#"{"terminal":"right"}"#);
        assert_eq!(serde_json::from_str::<Layout>(&text).unwrap(), layout);
    }

    #[test]
    fn sidebars_always_show_a_view() {
        let mut layout = Layout::default();
        assert_eq!(shown_in(&layout, Dock::Left, Some(View::Outline)), Some(View::Outline));
        // The wanted view moved away: the first one left takes its place.
        layout.move_to(View::Outline, Dock::Right);
        assert_eq!(shown_in(&layout, Dock::Left, Some(View::Outline)), Some(View::Files));
        assert_eq!(shown_in(&layout, Dock::Right, Some(View::Outline)), Some(View::Outline));
        assert_eq!(shown_in(&layout, Dock::Right, None), Some(View::Outline));
        assert_eq!(shown_in(&layout, Dock::Bottom, None), None);
        for view in [View::Files, View::Scene, View::Project] {
            layout.move_to(view, Dock::Bottom);
        }
        assert_eq!(shown_in(&layout, Dock::Left, Some(View::Files)), None);
        layout.move_to(View::Outline, Dock::Left);
        assert_eq!(shown_in(&layout, Dock::Right, Some(View::Outline)), Some(View::Inspector));
        layout.move_to(View::Inspector, Dock::Bottom);
        assert_eq!(shown_in(&layout, Dock::Right, Some(View::Outline)), None);
    }
}
