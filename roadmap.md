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
| UI | Iced 0.14 (wgpu + cosmic-text), custom `CodeEditor` widget |
| Text buffer | `ropey`, multi-selection change sets, grouped undo |
| Syntax highlighting | Hand-written GDScript lexer (tree-sitter later for folding/outline) |
| LSP / DAP client | `serde_json` + std threads, no async runtime (TCP to Godot: LSP 6005, DAP 6006) |
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
- No bundled UI/icon font yet, so explorer arrows are ASCII

**Deferred from Phase 0 → Phase 1**
- Tree-sitter — for folding/outline; the lexer stays as the fast highlighter
- Explorer create / rename / delete / reveal in OS

## Phase 1 — MVP

**Editor** — custom widget (`app/src/code_editor.rs`) over `editor_core::Buffer`
- [x] Rope buffer, line-number gutter, 4-wide tabs, current-line highlight, blinking cursor
- [x] Mouse: click, shift+click, drag-select, double-click word, triple-click line, wheel scroll
- [x] Multi-cursor: Alt+click, Ctrl+Alt+Up/Down, Ctrl+D (select next occurrence), Esc to collapse
- [x] Line ops: Alt+Up/Down move, Shift+Alt+Up/Down duplicate, Ctrl+Shift+K delete,
      Ctrl+K or Ctrl+/ toggle comment, Tab/Shift+Tab indent/outdent
- [x] Smart Enter (keeps indent, indents after `:` and brackets), smart Home
- [x] Bracket matching, auto-close brackets/quotes, wrap selection, step over closers
- [x] Undo/redo grouped by typing runs; clipboard (multi-cursor paste distributes lines)
- [x] AltGr typing on AZERTY etc. (Ctrl+Alt is treated as text, not a shortcut)
- [x] Indentation guides (bridging blank lines inside a block)
- [x] Box (column) selection by Alt+drag (visual columns, so tabs line up; short lines skipped)
- [ ] IME composition (CJK input)
- [x] Find/replace in file: Ctrl+F / Ctrl+H bar, case / whole word / regex (with `$1` groups),
      match highlights, Enter / Shift+Enter / F3 to step, replace one / all (one undo step),
      "all" turns matches into cursors, Escape closes
- [x] Project search: Ctrl+Shift+F, SEARCH tab, results grouped by file (unsaved edits included)
- [x] Replace across the project (SEARCH tab: "replace all" counts and asks to confirm; open tabs
      edited in place as one undo step, other files rewritten on disk; regex `$1` groups)
- [x] GDScript snippets in the completion list (`snip`, 30 built-ins: virtuals, `func`, `export`,
      `onready`, signals, control flow, `tween`, `#region`…), placeholders with Tab / Shift+Tab,
      also without the language server; they replace the bare `for`/`if`/… keywords
- [x] External file change detection (`notify`): unchanged tabs reload silently; tabs with unsaved
      edits are flagged "(changed on disk)" with a Reload button; explorer folders refresh
- [x] UTF-8 byte order mark kept out of the editor and the language server, restored on save

**LSP** — `lsp-client` crate; verified against Godot 4.7.1 (`cargo run -p lsp-client --example probe`)
- [x] Detect Godot installs (env, PATH, Desktop/Downloads/Program Files/Steam/scoop/winget), pick
      the one matching the project version, spawn `godot --headless --editor --lsp-port N`
- [x] Spawned Godot is killed with the IDE, even on crash (Windows Job Object)
- [x] Attach to a running Godot editor on :6005 when it has the same project open
      (checked via `gdscript_client/changeWorkspace`) — attach path not yet tested live
- [x] Document sync: didOpen / didChange (full text) / didSave / didClose
- [x] Status bar LSP indicator (state, Godot version, port); click to restart
- [x] Problems panel (click to jump) + error/warning counts in status bar
- [x] Go to definition (F12)
- [x] Auto-reconnect after a drop (1, 2, 5, 10, 20 s, then manual), also when Godot is closed
      or the headless server crashes — verified by killing the spawned server
- [x] Warn when no installed Godot matches the project version (status message, yellow LSP
      label, tooltip)
- [x] Inline diagnostics: colored underlines plus the message, faded, at the end of the line
      (most severe one per line), updated live as you type
- [x] Completion: opens while typing identifiers, on `.` `$` quotes, and Ctrl+Space; client-side
      ranking (prefix > substring > subsequence); kind tags; docs pane via `completionItem/resolve`;
      Enter/Tab/click to accept, functions get `()` with the cursor inside
- [x] Signature help on `(` and `,`, active parameter highlighted
- [x] Hover docs after the mouse rests 500 ms on a word — not yet tested live (needs a real mouse)
- [ ] Member completion after `.` not yet verified live (test harness can't type Shift chars on AZERTY)
- [x] Find references (Shift+F12), listed in the SEARCH tab grouped by file
- [x] Rename symbol (F2): `prepareRename` check, rename bar, open tabs edited in place (one undo
      step, left unsaved), other files rewritten on disk — protocol verified against 4.7.1, UI not
      yet clicked through by hand

**Godot awareness**
- [x] `res://` / `uid://` paths: Ctrl+click or F12 opens them (also in `.tscn`/`.tres`; uids read from
      `.uid`, `.import` and scene headers; binary assets open in the default app); `res://` path
      completion in strings and `load(`/`preload(` (fuzzy on the path; Godot's own file items dropped);
      holding Ctrl underlines clickable targets (paths that exist, words with a definition)
- [x] "Open in Godot editor" (status bar "Godot" / palette): starts the editor at the current scene
      (or the scene using the current script); first stops the IDE's headless server and attaches to
      the editor's (one Godot per project: two fight over GDExtension DLLs); refuses to open a
      project in an older Godot than it was saved with

**Run** — `app/src/runner.rs`
- [x] F5 run project, F6 run current scene (a `.tscn`, or the scene a `.gd` is attached to),
      F8 / Shift+F5 stop; status bar Run/Stop button; unsaved tabs are saved first
- [x] Output tab: ANSI stripped, errors/warnings colored (backtrace lines inherit), clickable
      `res://…:line` links, exit code; the game (and its child process) dies with the IDE
- [x] Bottom panel is now tabbed: Problems / Output / Terminal
- [x] Pick among several scenes using the same script: F6 asks once in the palette and remembers
      the choice per script; "Run: Choose Scene for This Script..." asks again
- [x] Runtime errors and warnings in the Problems panel and inline (paired with the first
      `res://` location after them, repeats counted, cleared on the next run)

**Terminal** — `terminal` crate (`alacritty_terminal` + ConPTY), panel widget in `app`
- [x] Integrated PTY terminal (PowerShell 7 if installed, else Windows PowerShell; login shell elsewhere)
- [x] Opens in project root; `godot` on PATH via a shim (console build on Windows), `$GODOT` set
- [x] Colors follow IDE theme (`[terminal]` ANSI palette in theme files)
- [x] Ctrl+J / Ctrl+` / Ctrl+² or status bar button to toggle; shell survives hiding; "kill" restarts
- [x] Selection (drag, double-click word, triple-click line), Ctrl+C copies when selected, Ctrl+V paste
      (bracketed paste), wheel scrollback, keyboard focus switches between editor and terminal
- [x] Resizable bottom panel: drag the line above it (kept between 100 px and the window minus 200)
- [x] Mouse reporting for TUI apps (click/drag/motion tracking, SGR and X10 encodings; Shift+drag
      still selects; wheel sends arrows in full-screen apps without tracking) — verified with WSL
- [x] Blinking cursor (solid while typing)

**Theming**
- [x] Single-file theme (TOML): UI, syntax and terminal ANSI colors (fonts still to come)
- [x] Live theme editor ("edit" in the status bar): all 38 colors, HSV + opacity sliders, hex
      input, code + terminal preview; the whole IDE updates live; built-ins are saved as a copy
- [x] Built-in themes: Godot Dark (Godot editor colors) and Godot Light
- [x] Theme export/import: .toml file dialogs, drag & drop a .toml on the window
- [x] Custom font family, size, line height and ligatures (theme `[font]` section, theme editor row)
- [x] UI zoom (Ctrl+= / Ctrl+- / Ctrl+0, saved in settings.json); selected theme remembered;
      `GODOT_IDE_CONFIG_DIR` overrides the config folder — fonts/zoom not yet checked on screen

**Workspace**
- [x] Command palette (Ctrl+Shift+P, or `>` in the box): ~35 commands incl. editor actions and themes
- [x] Quick open (Ctrl+P): fuzzy file search (`editor_core::fuzzy`), file-name matches first,
      open tabs listed first; go to line with `:` (Ctrl+G), `:line:column` supported
- [x] Keyboard shortcuts page (F1, `?` in the status bar, or the palette): every binding by
      section, with a filter; Escape closes
- [x] Recent projects on the welcome screen (name, Godot version, path; missing folders flagged;
      × forgets one) and as "Open Recent" palette commands

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
- [x] Shareable copy-paste theme codes (`GDTHEME1:` + compressed TOML, ~1 KB)
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
