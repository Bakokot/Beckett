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

Shortcuts: `Ctrl+O` open folder, `Ctrl+S` save, `Ctrl+W` close tab, `F12` go to definition,
`Shift+F12` find references, `F2` rename symbol,
`Ctrl+Space` completion, `Ctrl+J` (or `Ctrl+²`) toggle terminal, `F5` run project,
`F6` run current scene, `F8` stop, `Ctrl+F` find, `Ctrl+H` replace, `Ctrl+Shift+F` search in project,
`Ctrl+P` go to file, `Ctrl+Shift+P` command palette, `Ctrl+G` go to line.

## Themes

Themes are single TOML files. Built-ins live in `crates/theme/themes/`.
To install a shared theme, drop its `.toml` into:

- Windows: `%APPDATA%\godot-ide\themes\`
- Linux: `~/.config/godot-ide/themes/`
- macOS: `~/Library/Application Support/godot-ide/themes/`

and pick it from the status bar, or drop the file on the IDE window.

Click **edit** next to the theme picker to change colors live, save your own themes,
export or import `.toml` files, and copy or paste a one-line share code (`GDTHEME1:...`).

## Layout

| Crate | Purpose |
|---|---|
| `app` (`godot-ide`) | Iced application and UI |
| `editor-core` | Text buffer (multi-cursor, undo), GDScript lexer |
| `godot-project` | `project.godot` parsing, `res://` paths |
| `theme` | Theme format, built-in themes, loading |
| `lsp-client` | Godot language server client, Godot discovery |
| `terminal` | Integrated terminal (pty + VT parsing) |
