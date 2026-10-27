//! Lists the Godot installs found: `cargo run -p lsp-client --example installs`.

fn main() {
    for install in lsp_client::godot::discover() {
        println!("{:?} {}", install.version, install.path.display());
    }
}
