//! Small icons for the explorer: one per kind of file, plus open and closed
//! folders and the expand arrows. Hand-drawn 16×16 SVGs with fixed colors
//! that read on both dark and light themes.

use std::path::Path;
use std::sync::LazyLock;

use iced::widget::svg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    FolderClosed,
    FolderOpen,
    ArrowRight,
    ArrowDown,
    Project,
    Script,
    Scene,
    Resource,
    Shader,
    Image,
    Audio,
    Video,
    Model,
    Font,
    Json,
    Text,
    Config,
    CSharp,
    Library,
    File,
}

impl Icon {
    pub fn for_folder(expanded: bool) -> Self {
        if expanded {
            Icon::FolderOpen
        } else {
            Icon::FolderClosed
        }
    }

    pub fn for_file(path: &Path) -> Self {
        if path.file_name().is_some_and(|n| n == "project.godot") {
            return Icon::Project;
        }
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match extension.as_str() {
            "gd" => Icon::Script,
            "tscn" | "scn" | "escn" => Icon::Scene,
            "tres" | "res" => Icon::Resource,
            "gdshader" | "gdshaderinc" | "shader" | "glsl" => Icon::Shader,
            "png" | "jpg" | "jpeg" | "webp" | "svg" | "bmp" | "tga" | "exr" | "hdr" | "ktx" | "dds" => Icon::Image,
            "wav" | "ogg" | "mp3" => Icon::Audio,
            "ogv" | "webm" | "mp4" => Icon::Video,
            "glb" | "gltf" | "fbx" | "obj" | "blend" | "dae" => Icon::Model,
            "ttf" | "otf" | "woff" | "woff2" | "fnt" => Icon::Font,
            "json" => Icon::Json,
            "md" | "txt" | "csv" => Icon::Text,
            "cfg" | "ini" | "toml" | "import" | "gdextension" | "godot" | "yml" | "yaml" => Icon::Config,
            "cs" => Icon::CSharp,
            "dll" | "so" | "dylib" | "wasm" => Icon::Library,
            _ => Icon::File,
        }
    }

    /// The icon's SVG, made once and shared (handles are cached by id).
    pub fn handle(self) -> svg::Handle {
        static HANDLES: LazyLock<Vec<svg::Handle>> =
            LazyLock::new(|| ALL.iter().map(|icon| svg::Handle::from_memory(source(*icon).as_bytes())).collect());
        HANDLES[ALL.iter().position(|&i| i == self).unwrap_or(0)].clone()
    }
}

const ALL: [Icon; 20] = [
    Icon::FolderClosed,
    Icon::FolderOpen,
    Icon::ArrowRight,
    Icon::ArrowDown,
    Icon::Project,
    Icon::Script,
    Icon::Scene,
    Icon::Resource,
    Icon::Shader,
    Icon::Image,
    Icon::Audio,
    Icon::Video,
    Icon::Model,
    Icon::Font,
    Icon::Json,
    Icon::Text,
    Icon::Config,
    Icon::CSharp,
    Icon::Library,
    Icon::File,
];

fn source(icon: Icon) -> &'static str {
    match icon {
        Icon::FolderClosed => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M1.5 3.5a1 1 0 0 1 1-1h3.6l1.5 1.5h5.9a1 1 0 0 1 1 1v7.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" fill="#6f9fd8"/><path d="M1.5 6h13" stroke="#4f7fb8" stroke-width="1"/></svg>"##
        }
        Icon::FolderOpen => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M1.5 3.5a1 1 0 0 1 1-1h3.6l1.5 1.5h5a1 1 0 0 1 1 1V6.5H4.2a1 1 0 0 0-.95.7L1.5 12.5z" fill="#4f7fb8"/><path d="M3.3 7.2a1 1 0 0 1 .95-.7h10.6a.6.6 0 0 1 .57.8l-1.9 5.5a1 1 0 0 1-.95.7H1.6z" fill="#8bb5ea"/></svg>"##
        }
        Icon::ArrowRight => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M6 4l4 4-4 4" fill="none" stroke="#8a919c" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        }
        Icon::ArrowDown => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4" fill="none" stroke="#8a919c" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        }
        // Godot's robot head, simplified.
        Icon::Project => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3 5.5l1.2-2 2 .9.9-1.6h1.8l.9 1.6 2-.9 1.2 2-.6 1v5.2a1.3 1.3 0 0 1-1.3 1.3H4.9a1.3 1.3 0 0 1-1.3-1.3V6.5z" fill="#478cbf"/><circle cx="6" cy="8.3" r="1.5" fill="#fff"/><circle cx="10" cy="8.3" r="1.5" fill="#fff"/><circle cx="6.2" cy="8.4" r=".7" fill="#414042"/><circle cx="9.8" cy="8.4" r=".7" fill="#414042"/><path d="M6 11.3h4" stroke="#fff" stroke-width="1" stroke-linecap="round"/></svg>"##
        }
        // A page with code brackets.
        Icon::Script => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3.5 1.5h6l3 3v10h-9z" fill="#478cbf"/><path d="M9.5 1.5v3h3" fill="#6fa8d6"/><path d="M6.6 7.5L5 9.5l1.6 2M9.4 7.5L11 9.5l-1.6 2" fill="none" stroke="#fff" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        }
        // A clapperboard, like Godot's PackedScene icon.
        Icon::Scene => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="6.5" width="13" height="7.5" rx="1" fill="#e8a948"/><path d="M1.6 5.6l12.6-3.1.6 2.3-12.6 3.1z" fill="#e8a948"/><path d="M4.4 4.9l1.8 1.6M7.8 4.1l1.8 1.6M11.2 3.3l1.8 1.6" stroke="#2b2f36" stroke-width="1.1"/></svg>"##
        }
        // A gem, for saved resources.
        Icon::Resource => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4.5 2.5h7L14.5 6 8 14 1.5 6z" fill="#e0786a"/><path d="M1.5 6h13M5.8 6L8 14l2.2-8M4.5 2.5L5.8 6 8 2.5 10.2 6l1.3-3.5" fill="none" stroke="#b5544a" stroke-width=".9" stroke-linejoin="round"/></svg>"##
        }
        // A shaded sphere.
        Icon::Shader => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs><radialGradient id="g" cx=".35" cy=".35" r=".75"><stop offset="0" stop-color="#e3d6ff"/><stop offset=".5" stop-color="#9a7ff0"/><stop offset="1" stop-color="#4b3a9a"/></radialGradient></defs><circle cx="8" cy="8" r="6.5" fill="url(#g)"/></svg>"##
        }
        // A picture: frame, hills and sun.
        Icon::Image => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="2.5" width="13" height="11" rx="1.2" fill="#4fb38a"/><path d="M2.5 12.5l3.5-4.5 2.5 3 2-2 3 3.5z" fill="#2c7a5a"/><circle cx="10.5" cy="5.8" r="1.4" fill="#f4e27a"/></svg>"##
        }
        // A music note.
        Icon::Audio => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M6 11.5V3.2l7-1.4v8.2" fill="none" stroke="#e3c454" stroke-width="1.5" stroke-linejoin="round"/><ellipse cx="4.3" cy="11.7" rx="2.2" ry="1.7" fill="#e3c454"/><ellipse cx="11.3" cy="10.2" rx="2.2" ry="1.7" fill="#e3c454"/></svg>"##
        }
        // A play button in a frame.
        Icon::Video => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="3" width="13" height="10" rx="1.5" fill="#d65c5c"/><path d="M6.5 5.8v4.4L10.3 8z" fill="#fff"/></svg>"##
        }
        // A shaded cube.
        Icon::Model => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M8 1.5l6 3.2L8 8 2 4.7z" fill="#f2a66a"/><path d="M2 4.7L8 8v6.5l-6-3.3z" fill="#e07b39"/><path d="M14 4.7L8 8v6.5l6-3.3z" fill="#b45d24"/></svg>"##
        }
        // A capital A.
        Icon::Font => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3 14L8 2l5 12M5 10h6" fill="none" stroke="#b6bcc6" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        }
        // Curly braces.
        Icon::Json => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M5.5 2.5C4 2.5 4 3.5 4 5s0 2.5-1.5 3C4 8.5 4 9.5 4 11s0 2.5 1.5 2.5M10.5 2.5C12 2.5 12 3.5 12 5s0 2.5 1.5 3C12 8.5 12 9.5 12 11s0 2.5-1.5 2.5" fill="none" stroke="#cbb94a" stroke-width="1.4" stroke-linecap="round"/></svg>"##
        }
        // A page with lines of text.
        Icon::Text => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3.5 1.5h6l3 3v10h-9z" fill="#8b95a3"/><path d="M9.5 1.5v3h3" fill="#aeb6c1"/><path d="M5.5 7.5h5M5.5 9.5h5M5.5 11.5h3" stroke="#fff" stroke-width="1" stroke-linecap="round"/></svg>"##
        }
        // Sliders.
        Icon::Config => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2.5 4h11M2.5 8h11M2.5 12h11" stroke="#8b95a3" stroke-width="1.3" stroke-linecap="round"/><circle cx="5.5" cy="4" r="1.8" fill="#c3cad3"/><circle cx="10.5" cy="8" r="1.8" fill="#c3cad3"/><circle cx="7" cy="12" r="1.8" fill="#c3cad3"/></svg>"##
        }
        // A hash sign on a hexagon, as C# logos go.
        Icon::CSharp => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M8 1l6.1 3.5v7L8 15l-6.1-3.5v-7z" fill="#9b4f96"/><path d="M6.5 5L5.5 11M9.5 5l-1 6M4.6 7h6.4M4.2 9h6.4" stroke="#fff" stroke-width="1" stroke-linecap="round"/></svg>"##
        }
        // A puzzle piece, for native libraries.
        Icon::Library => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2.5 4.5h3.2a1.8 1.8 0 1 1 3.6 0h3.2v3.2a1.8 1.8 0 1 1 0 3.6v3.2h-10z" fill="#7d8796"/></svg>"##
        }
        // A blank page with a folded corner.
        Icon::File => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3.5 1.5h6l3 3v10h-9z" fill="none" stroke="#8b95a3" stroke-width="1.1" stroke-linejoin="round"/><path d="M9.5 1.5v3h3" fill="none" stroke="#8b95a3" stroke-width="1.1" stroke-linejoin="round"/></svg>"##
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_name_and_extension() {
        assert_eq!(Icon::for_file(Path::new("game/project.godot")), Icon::Project);
        assert_eq!(Icon::for_file(Path::new("player.gd")), Icon::Script);
        assert_eq!(Icon::for_file(Path::new("level.TSCN")), Icon::Scene);
        assert_eq!(Icon::for_file(Path::new("theme.tres")), Icon::Resource);
        assert_eq!(Icon::for_file(Path::new("water.gdshader")), Icon::Shader);
        assert_eq!(Icon::for_file(Path::new("icon.svg")), Icon::Image);
        assert_eq!(Icon::for_file(Path::new("other.godot")), Icon::Config);
        assert_eq!(Icon::for_file(Path::new("LICENSE")), Icon::File);
        assert_eq!(ALL.len(), ALL.iter().map(|&i| source(i)).collect::<std::collections::HashSet<_>>().len());
    }
}
