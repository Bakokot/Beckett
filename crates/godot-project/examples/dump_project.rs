//! Prints the autoloads and Input Map of a project: `dump_project <folder>`.

fn main() {
    let root = std::env::args().nth(1).expect("usage: dump_project <project folder>");
    let project = godot_project::GodotProject::load(std::path::Path::new(&root)).expect("no project.godot");
    println!("main scene: {:?}", project.main_scene());
    for a in project.autoloads() {
        println!("autoload {} = {} (global: {}, line {})", a.name, a.path, a.global, a.line + 1);
    }
    for a in project.input_actions() {
        println!("action {} (line {}, deadzone {:?}): {}", a.name, a.line + 1, a.deadzone, a.events.join(" · "));
    }
}
