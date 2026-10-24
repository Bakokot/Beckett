# Godot IDE

A lightweight native IDE for Godot, written in Rust. See [roadmap.md](roadmap.md).

## Requirements

- Rust (stable) — install via [rustup](https://rustup.rs)
- On Windows: Visual Studio Build Tools with the "Desktop development with C++" workload
- Godot 4.x (needed from Phase 1 for the language server)

## Run

```sh
cargo run -p godot-ide                    # welcome screen
cargo run -p godot-ide -- path/to/project # open a project directly
cargo test                                # run unit tests
```

Shortcuts: `Ctrl+O` open folder, `Ctrl+S` save, `Ctrl+W` close tab.

## Themes

Themes are single TOML files. Built-ins live in `crates/theme/themes/`.
To install a shared theme, drop its `.toml` into:

- Windows: `%APPDATA%\godot-ide\themes\`
- Linux: `~/.config/godot-ide/themes/`
- macOS: `~/Library/Application Support/godot-ide/themes/`

and pick it from the status bar.

## Layout

| Crate | Purpose |
|---|---|
| `app` (`godot-ide`) | Iced application and UI |
| `editor-core` | Language detection, GDScript lexer |
| `godot-project` | `project.godot` parsing, `res://` paths |
| `theme` | Theme format, built-in themes, loading |
