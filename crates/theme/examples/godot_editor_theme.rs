//! Converts the Godot editor colors found here (read only):
//! `cargo run -p theme --example godot_editor_theme`.

fn main() {
    for path in theme::godot_editor::settings_files() {
        match theme::godot_editor::import(&path) {
            Ok(theme) => println!(
                "{:22} {:?} bg {} text {} keyword {} string {} accent {}",
                theme.name, theme.kind, theme.ui.background, theme.syntax.text, theme.syntax.keyword, theme.syntax.string, theme.ui.accent
            ),
            Err(error) => println!("FAIL {error}"),
        }
    }
}
