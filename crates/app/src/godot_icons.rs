//! Godot's own editor icons (`assets/godot_icons.tsv`, MIT, see
//! `assets/GODOT_ICONS_LICENSE.txt`), by class name: `Node2D`, `Label`,
//! `PackedScene`… On light themes their colors are converted the way the
//! Godot editor converts them.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use iced::widget::svg;

static SOURCES: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| include_str!("../assets/godot_icons.tsv").lines().filter_map(|line| line.split_once('\t')).collect());

/// Whether Godot has an icon named `name`.
pub fn has(name: &str) -> bool {
    SOURCES.contains_key(name)
}

/// The icon named `name`, for a light or dark background; made once.
pub fn handle(name: &str, light: bool) -> Option<svg::Handle> {
    static HANDLES: LazyLock<Mutex<HashMap<(String, bool), svg::Handle>>> = LazyLock::new(Default::default);
    let source = SOURCES.get(name)?;
    let mut handles = HANDLES.lock().unwrap_or_else(|e| e.into_inner());
    let handle = handles.entry((name.to_string(), light)).or_insert_with(|| {
        let svg = if light && !EXCEPTIONS.contains(&name) { to_light(source) } else { source.to_string() };
        svg::Handle::from_memory(svg.into_bytes())
    });
    Some(handle.clone())
}

/// Whether a background is light enough for the light icon colors.
pub fn is_light(background: theme::Color) -> bool {
    let luminance = 0.299 * f32::from(background.r) + 0.587 * f32::from(background.g) + 0.114 * f32::from(background.b);
    luminance > 127.5
}

/// Replaces each `#rgb` / `#rrggbb` color Godot converts for light themes.
fn to_light(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let digits = rest.chars().take_while(char::is_ascii_hexdigit).count();
        let rgb = match digits {
            6 => u32::from_str_radix(&rest[..6], 16).ok(),
            3 => u32::from_str_radix(&rest[..3], 16).ok().map(|v| {
                let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
                ((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11)
            }),
            _ => None,
        };
        match rgb.and_then(|rgb| LIGHT_COLORS.iter().find(|(dark, _)| *dark == rgb)) {
            Some((_, light)) => {
                out.push_str(&format!("#{light:06x}"));
                rest = &rest[digits..];
            }
            None => out.push('#'),
        }
    }
    out.push_str(rest);
    out
}

/// Dark-theme color → light-theme color, from Godot's `editor_color_map.cpp`.
const LIGHT_COLORS: &[(u32, u32)] = &[
    (0x478cbf, 0x478cbf),
    (0x414042, 0x414042),
    (0xffffff, 0x414141),
    (0xfefefe, 0xfefefe),
    (0x000000, 0xbfbfbf),
    (0x010101, 0x010101),
    (0xff0000, 0xff0000),
    (0x00ff00, 0x00ff00),
    (0x0000ff, 0x0000ff),
    (0xe0e0e0, 0x5a5a5a),
    (0x808080, 0x808080),
    (0xb3b3b3, 0x363636),
    (0x699ce8, 0x699ce8),
    (0xf9f9f9, 0x606060),
    (0x5fb2ff, 0x0079f0),
    (0x003e7a, 0x2b74bb),
    (0xf7f5cf, 0x615f3a),
    (0xc38ef1, 0xa85de9),
    (0x8da5f3, 0x3d64dd),
    (0x7582a8, 0x6d83c8),
    (0xfc7f7f, 0xcd3838),
    (0xb56d6d, 0xbe6a6a),
    (0x99c4ff, 0x4589e6),
    (0x869ebf, 0x7097cd),
    (0xffa6bd, 0xe65c7f),
    (0xbf909c, 0xcd8b9c),
    (0x8eef97, 0x2fa139),
    (0x76ad7b, 0x64a66a),
    (0xf0caa0, 0x844b0e),
    (0xff4545, 0xff2929),
    (0xffe345, 0xffe337),
    (0x80ff45, 0x74ff34),
    (0x45ffa2, 0x2cff98),
    (0x45d7ff, 0x22ccff),
    (0x8045ff, 0x702aff),
    (0xff4596, 0xff2781),
    (0xe1da5b, 0xd6cf4b),
    (0x62aeff, 0x1678e0),
    (0x75d1e6, 0x41acc5),
    (0x84ffee, 0x49ccba),
    (0xf70000, 0xc91616),
    (0xeec315, 0xd58c0b),
    (0xdbee15, 0xb7d10a),
    (0x288027, 0x218309),
    (0xffca5f, 0xfea900),
    (0x2998ff, 0x68b6ff),
    (0xa2d2ff, 0x4998e3),
    (0x69c4d4, 0x29a3cc),
    (0xea7940, 0xbd5e2c),
    (0xff2b88, 0xbd165f),
    (0xeac840, 0xbd9d1f),
    (0x3cf34e, 0x16a827),
    (0x2877f6, 0x236be6),
    (0xeae440, 0x9f9722),
    (0xa448f0, 0x9853ce),
    (0x5ad5c4, 0x0a9c88),
    (0xd6d6d6, 0x474747),
    (0x474747, 0xd6d6d6),
    (0x919191, 0x6e6e6e),
    (0xfce00e, 0xaa8d24),
    (0x0e71fc, 0x0350bd),
    (0xc6ced4, 0x828f9b),
    (0x41ecad, 0x25e3a0),
    (0x6f91f0, 0x6d8eeb),
    (0x5abbef, 0x4fb2e9),
    (0x35d4f4, 0x27ccf0),
    (0x4593ec, 0x4690e7),
    (0xee5677, 0xee7991),
    (0xe1ec41, 0xb2bb19),
    (0x54ed9e, 0x57e99f),
    (0x417aec, 0x6993ec),
    (0x55f3e3, 0x12d5c3),
    (0xf74949, 0xf77070),
    (0x44bd44, 0x46b946),
    (0xec418e, 0xec69a3),
    (0xf1738f, 0xee758e),
    (0x41ec80, 0x2ce573),
    (0xb9ec41, 0x96ce1a),
    (0xf68f45, 0xf49047),
    (0xac73f1, 0xad76ee),
    (0xde66f0, 0xdc6aed),
    (0xf066bd, 0xed6abd),
    (0x77ce57, 0x67c046),
    (0xea686c, 0xd95256),
    (0xeac968, 0xd9b64f),
    (0xcf68ea, 0xc050dd),
];

/// Icons Godot never converts.
const EXCEPTIONS: &[&str] = &["EditorPivot", "EditorHandle", "EditorHandleDisabled", "EditorHandleAdd", "EditorCurveHandle", "EditorPathSharpHandle", "EditorPathSmoothHandle", "EditorBoneHandle", "Editor3DHandle", "Godot", "Sky", "EditorControlAnchor", "DefaultProjectIcon", "ZoomMore", "ZoomLess", "ZoomReset", "LockViewport", "GroupViewport", "StatusSuccess", "OverbrightIndicator", "MaterialPreviewCube", "MaterialPreviewSphere", "MaterialPreviewQuad", "Camera3DDarkBackground", "GuiTabMenuHlDarkBackground", "ViewportSpeed", "ViewportZoom", "MaterialPreviewLight1", "MaterialPreviewLight2", "Gizmo3DSamplePlayer", "GizmoAreaLight", "GizmoAudioListener3D", "GizmoCamera3D", "GizmoCPUParticles3D", "GizmoDecal", "GizmoDirectionalLight", "GizmoFogVolume", "GizmoGPUParticles3D", "GizmoLight", "GizmoLightmapGI", "GizmoLightmapProbe", "GizmoReflectionProbe", "GizmoSpotLight", "GizmoVoxelGI", "GuiChecked", "GuiRadioChecked", "GuiIndeterminate", "GuiCloseCustomizable", "GuiGraphNodePort", "GuiResizer", "GuiMiniCheckerboard", "PickerCursorBg", "CenterView", "GuiTab", "GuiSpace", "CodeFoldedRightArrow", "CodeFoldDownArrow", "CodeRegionFoldedRightArrow", "CodeRegionFoldDownArrow", "TextEditorPlay", "Breakpoint",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_icons_by_class() {
        for name in ["Node", "Node2D", "Node3D", "Control", "CharacterBody2D", "Label", "PackedScene", "Script"] {
            assert!(has(name), "{name}");
        }
        assert!(!has("NoSuchClass"));
        assert!(handle("Node2D", true).is_some());
    }

    #[test]
    fn converts_colors_for_light_themes() {
        assert_eq!(to_light(r##"<path fill="#8da5f3" stroke="#fff"/>"##), r##"<path fill="#3d64dd" stroke="#414141"/>"##);
        assert_eq!(to_light(r##"<path fill="#123456"/><a href="#x"/>"##), r##"<path fill="#123456"/><a href="#x"/>"##);
        assert!(is_light(theme::Color { r: 240, g: 240, b: 240, a: 255 }));
        assert!(!is_light(theme::Color { r: 30, g: 30, b: 30, a: 255 }));
    }
}
