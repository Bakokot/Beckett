# Beckett

A lightweight native IDE for Godot, written in Rust (formerly "Godot IDE"). See [roadmap.md](roadmap.md).

## Requirements

- Rust (stable) — install via [rustup](https://rustup.rs)
- On Windows: Visual Studio Build Tools with the "Desktop development with C++" workload
- Godot 4.x (needed from Phase 1 for the language server)

## Run

```sh
cargo run -p beckett                    # welcome screen
cargo run -p beckett -- path/to/project # open a project directly
cargo test                              # run unit tests
```

Shortcuts: `Ctrl+O` open folder, `Ctrl+S` save, `Ctrl+W` close tab, `F12` go to definition,
`Shift+F12` find references, `F2` rename symbol,
`Ctrl+Space` completion, `Ctrl+J` (or `Ctrl+²`) toggle terminal, `F5` run project,
`F6` run current scene, `F8` stop, `Ctrl+F` find, `Ctrl+H` replace, `Ctrl+Shift+F` search in project,
`Ctrl+P` go to file, `Ctrl+Shift+P` command palette, `Ctrl+G` go to line,
`Ctrl+Shift+O` go to symbol in file, `Ctrl+T` go to symbol in project, `Shift+F1` class reference,
`F9` toggle breakpoint, `Alt+F5` / `Alt+F6` debug the main / current scene, `F10` / `F11` / `Shift+F11` step
(`F12` on an engine class or method opens its page there).
Press `F1` (or click `?` in the status bar) for the full list inside the IDE. Click a
command's shortcut there and press new keys to change it; changes are saved to
`keybindings.json` in the config folder (only the bindings that differ from the preset).
The page also switches between the Default, VS Code and Godot presets (Godot: `F1` class reference, `Ctrl+L` go to
line, `Ctrl+R` replace, `Shift+Alt+O` quick open, `Shift+Alt+Up/Down` add cursors, `Ctrl+Shift+D`
duplicate line); picking one resets your changes.

## Themes

Themes are single TOML files. Built-ins live in `crates/theme/themes/`.
To install a shared theme, drop its `.toml` into:

- Windows: `%APPDATA%\beckett\themes\`
- Linux: `~/.config/beckett/themes/`
- macOS: `~/Library/Application Support/beckett/themes/`

and pick it from the status bar, or drop the file on the IDE window.

The config folder was called `godot-ide` before the rename; Beckett moves it to `beckett` on its
first start (settings, keybindings, themes, sessions and recent projects come along).
`BECKETT_CONFIG_DIR` (or the older `GODOT_IDE_CONFIG_DIR`) points it elsewhere.

Click **edit** next to the theme picker to change colors live, save your own themes,
export or import `.toml` files, and copy or paste a one-line share code (`GDTHEME1:...`).

## Layout

| Crate | Purpose |
|---|---|
| `app` (`beckett`) | Iced application and UI |
| `editor-core` | Text buffer (multi-cursor, undo), GDScript lexer |
| `godot-project` | `project.godot` parsing, `res://` paths |
| `theme` | Theme format, built-in themes, loading |
| `lsp-client` | Godot language server and debug adapter clients, Godot discovery |
| `terminal` | Integrated terminal (pty + VT parsing) |
