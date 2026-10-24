# Godot IDE — Roadmap

A lightweight, native IDE built specifically for Godot, with integrated GDScript LSP,
easily shareable themes, an integrated terminal — and the long-term goal of replacing
the Godot editor.

## Decisions

| Topic | Decision |
|---|---|
| Implementation | Native Rust |
| Godot dependency | Godot must be installed (it provides the LSP, DAP, docs and asset import) |
| Scope | Code editor first, Godot editor replacement eventually |

## Tech stack

| Concern | Choice |
|---|---|
| UI | Iced (wgpu + cosmic-text) |
| Text buffer | `ropey` |
| Syntax highlighting | `tree-sitter` + `tree-sitter-gdscript` |
| LSP / DAP client | `lsp-types` + `tokio` (TCP to Godot: LSP 6005, DAP 6006) |
| Terminal | `alacritty_terminal` + `portable-pty` |
| File watching | `notify` |
| Fuzzy finder | `nucleo` |
| Themes / settings | `serde` + TOML |
| Git | `gix` |
| `.tscn` / `.tres` | Custom parser/writer |

### Workspace layout

```
crates/
  app/            # Iced application, UI shell, panels
  editor-core/    # rope buffer, cursors, undo, highlighting
  lsp-client/     # LSP + DAP clients, Godot headless process management
  terminal/       # PTY + terminal emulation widget
  theme/          # theme format, loading, share codes, importers
  godot-project/  # project.godot, res:// / uid:// resolution, .tscn/.tres parsing
```

## Non-functional targets

- Cold start < ~1s, idle RAM < ~150 MB
- Single small binary / portable zip
- Windows, Linux, macOS
- No telemetry, fully offline
- Opt-in auto-update

---

## Phase 0 — Skeleton

> Builds with the `x86_64-pc-windows-gnu` toolchain; 17 unit tests pass. Launch + highlighting
> verified on a sample project. Not yet clicked through by hand: folder dialog, save, theme switch.
> `lsp-client` and `terminal` crates are added in Phase 1.

- [x] Cargo workspace (`app`, `editor-core`, `godot-project`, `theme`), `git init`
- [x] Iced window: explorer sidebar / tab bar + editor / status bar
- [x] File explorer (lazy tree, hides `.import`, `.uid`, `.godot/`, `.git`)
- [x] Editor tabs with save, dirty marker, close guard (uses iced's `text_editor` for now)
- [x] GDScript highlighting via a hand-written line lexer (Godot editor colors)
- [x] Theme file loading (TOML): 2 built-ins + user themes dir, switcher in status bar
- [x] Open folder (dialog or CLI arg) + `project.godot` parsing (name, version, `res://`)

**Known Phase 0 issues**
- Tabs render 8 columns wide (Godot uses 4); fix with the custom editor widget, or sooner if iced exposes tab width
- No bundled UI/icon font yet, so explorer arrows are ASCII

**Deferred from Phase 0 → Phase 1**
- Custom rope-based (`ropey`) editor widget — required for multi-cursor, column selection, folding
- Tree-sitter — for folding/outline; the lexer stays as the fast highlighter
- Explorer create / rename / delete / reveal in OS

## Phase 1 — MVP

**Editor**
- [ ] Multi-cursor, column selection, line duplicate/move, comment toggle
- [ ] Bracket matching / auto-close, indentation guides
- [ ] Find/replace in file and across project (regex, case, whole word)
- [ ] GDScript snippets (`_ready`, `_process`, `@export`, `@onready`, signals…)
- [ ] External file change detection and reload

**LSP**
- [ ] Detect Godot install, spawn `godot --headless --editor --lsp-port N` per project
- [ ] Attach to an already-running Godot editor if present; auto-reconnect
- [ ] Completion, hover, signature help
- [ ] Go to definition, find references, rename
- [ ] Diagnostics inline + Problems panel
- [ ] Status bar LSP indicator (state, Godot version, port)

**Godot awareness**
- [ ] `res://` / `uid://` resolution: click to open, path completion in strings
- [ ] "Open in Godot editor" for current scene

**Run**
- [ ] Run project / run current scene / stop
- [ ] Output panel with clickable `res://…:line` links

**Terminal**
- [ ] Integrated PTY terminal (PowerShell, cmd, bash, zsh)
- [ ] Opens in project root with `godot` on PATH
- [ ] Colors follow IDE theme

**Theming**
- [ ] Single-file theme: UI, syntax, terminal ANSI colors, fonts
- [ ] Live theme editor with instant preview
- [ ] Built-in themes, including a Godot-default look-alike
- [ ] Theme export/import (file, drag & drop)
- [ ] Custom fonts, ligatures, line height, UI scale

**Workspace**
- [ ] Command palette (Ctrl+Shift+P)
- [ ] Quick open (Ctrl+P, fuzzy)
- [ ] Recent projects / welcome screen

## Phase 2 — v1

**Editor**
- [ ] Code folding (incl. `#region`)
- [ ] Split views
- [ ] Hot exit / persistent undo
- [ ] Format on save (gdformat or LSP)
- [ ] Highlighting for `.gdshader`, `.tscn`, `.tres`, `project.godot`, `.cfg`, JSON, Markdown

**LSP / Godot**
- [ ] Outline panel, document & workspace symbols
- [ ] Offline class reference browser (generated via `godot --doctool`)
- [ ] Autoloads / Input Map / project settings panel + completion for action names
- [ ] Drag file into code → `preload("res://…")`
- [ ] Script templates / "New Script" dialog
- [ ] Godot version picker (choose among installed versions per project)

**Debug**
- [ ] DAP debugger: breakpoints, stepping, call stack, variables

**Terminal**
- [ ] Multiple terminal tabs / splits
- [ ] Clickable paths and URLs

**Theming**
- [ ] Shareable copy-paste theme codes
- [ ] Import VS Code themes and Godot editor theme settings
- [ ] Light/dark variants, follow OS

**Workspace**
- [ ] Dockable / resizable panels
- [ ] Remappable keybindings with VS Code and Godot presets
- [ ] Settings UI + user/project JSON
- [ ] Session restore

**Git**
- [ ] Explorer status + gutter change markers
- [ ] Diff view
- [ ] Godot-aware `.gitignore` generator

## Phase 3 — Godot editor replacement

- [ ] **ER1** Scene tree editor (add/remove/reparent/rename nodes, attach scripts) — `.tscn` read/write
- [ ] **ER2** Inspector for node properties, exports and resources (metadata from `godot --dump-extension-api`)
- [ ] **ER3** Signal connection editor
- [ ] **ER4** Project settings and Input Map editors
- [ ] **ER5** Resource editors (Theme, TileSet, AnimationPlayer…), one at a time
- [ ] **ER6** 2D/3D viewport — via LibGodot embedding or the Godot editor's embedded game window
- [ ] Asset import delegated to `godot --headless --import`

## Later / backlog

- Minimap, breadcrumbs, zen mode
- LSP code actions, inlay hints
- `$NodePath` / `%UniqueName` completion from the owning scene
- Signal browser
- Rename/move file with `res://` reference updates
- Shader editor helpers
- Watch expressions / debug console, run configurations, remote scene tree inspector
- Task runner
- Community theme gallery (GitHub-backed), file icon themes
- Git stage/commit/branch/push
- gdlint integration, GUT / gdUnit4 test runner, TODO panel
- Plugin API, snippet/theme packs
- Localization
- C# support (OmniSharp / csharp-ls) — possibly out of scope
