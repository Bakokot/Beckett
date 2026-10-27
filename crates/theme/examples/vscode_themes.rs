//! Lists the VS Code themes installed here and converts each:
//! `cargo run -p theme --example vscode_themes`.

fn main() {
    let installed = theme::vscode::installed();
    println!("{} themes found", installed.len());
    for found in installed {
        match theme::vscode::import(&found.path) {
            Ok(theme) => println!(
                "ok   {:32} {:?} bg {} keyword {} string {} comment {}",
                found.label, theme.kind, theme.ui.background, theme.syntax.keyword, theme.syntax.string, theme.syntax.comment
            ),
            Err(error) => println!("FAIL {:32} {error}", found.label),
        }
    }
}
