# Beckett — Roadmap

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
| Git | the `git` command (status, staged text); own Myers line diff in `editor-core` |
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

**Deferred from Phase 0 → Phase 1**
- Tree-sitter — for folding/outline; the lexer stays as the fast highlighter
- ~~Explorer create / rename / delete / reveal in OS~~ — done in Phase 2 (Workspace)

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
- [x] IME composition (CJK input): candidate window placed at the cursor, composition drawn
      underlined at the cursor, commit inserted as typing — not yet tried with a real IME
- [x] Wide (CJK, full-width) characters take two columns, drawn on the grid — verified on screen
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
      (checked via `gdscript_client/changeWorkspace`) — verified live via "Open in Godot"
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
- [x] Hover docs after the mouse rests 500 ms on a word — seen live (Ctrl+hover test)
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
- [x] Live theme editor ("edit" in the status bar): all 38 colors, a color palette (saturation / brightness
      square, hue and opacity bars, click or drag), hex
      (MANAGE THEMES column: click to use, ▲ / ▼ reorder the theme list (kept in settings), delete your
      own themes to the Recycle Bin after a confirm; built-ins can only move)
      input, code + terminal preview; the whole IDE updates live; built-ins are saved as a copy
- [x] Built-in themes: Godot Dark (Godot editor colors), Godot Light, and the crimson trio: Crimson Night,
      Crimson Abyss (high contrast, near black) and Crimson Velvet (soft, plum)
- [x] Theme export/import: .toml file dialogs, drag & drop a .toml on the window
- [x] Custom font family, size, line height and ligatures (theme `[font]` section, theme editor row)
- [x] UI zoom (Ctrl+= / Ctrl+- / Ctrl+0, saved in settings.json); selected theme remembered;
      `BECKETT_CONFIG_DIR` (formerly `GODOT_IDE_CONFIG_DIR`) overrides the config folder (verified: Cascadia Code, `>=` ligature, 125%)

**Workspace**
- [x] Command palette (Ctrl+Shift+P, or `>` in the box): ~35 commands incl. editor actions and themes
- [x] Quick open (Ctrl+P): fuzzy file search (`editor_core::fuzzy`), file-name matches first,
      open tabs listed first; go to line with `:` (Ctrl+G), `:line:column` supported
- [x] Keyboard shortcuts page (F1, `?` in the status bar, or the palette): every binding by
      section, with a filter; Escape closes
- [x] Recent projects on the welcome screen (name, Godot version, path; missing folders flagged;
      × forgets one) and as "Open Recent" palette commands
- [x] File icons in the explorer: one per kind (script, scene, resource, shader, image, audio, video,
      3D model, font, JSON, text, config, C#, library, project.godot), open/closed folders, SVG
      arrows — seen on screen (open folder icon not yet)

## Phase 2 — v1

**Editor**
- [x] Code folding: indentation blocks and `#region` / `#endregion`; Alt+F (as in Godot), gutter
      click, "..." marker, Fold / Unfold / Fold All / Unfold All; folds follow edits and undo, Up/Down
      step over them, a cursor landing inside opens them — seen on screen (restored session)
- [x] Split view: two sides with their own tabs; Ctrl+\ (or the palette) moves the current tab to
      the other side, a side left empty closes the split; click a side to focus it; sides kept in
      the session — seen on screen. (One file can't show on both sides: one buffer, one cursor)
      Drag the bar between the sides to resize them (150 px minimum each; the share is kept in
      the session) — seen on screen with a mouse drag
- [x] Hot exit: unsaved edits kept in the session on close (and every 30 s), restored as unsaved;
      flagged "(changed on disk)" when the file changed meanwhile — seen on screen
- [x] Persistent undo: each file's undo/redo history (last 500 steps of each) is kept in `<config>/undo/` when
      its tab closes and with the session (on exit and every 30 s), and comes back when the file opens with
      the text the history ends at (hash-checked, then replayed both ways before use, so a stale or damaged
      history is ignored, never applied); unsaved tabs restored by hot exit get theirs too; histories untouched
      for 30 days are removed — unit-tested, not yet seen on screen
- [x] Format Document (Shift+Alt+F) and opt-in format on save (`"format_on_save": true` in
      settings.json): gdformat when installed (Godot's LSP has no formatter), else trailing whitespace
      and final newline tidied; one undo step — unit-tested, gdformat path untested (not installed)
- [x] Highlighting for `.gdshader`, `.tscn`, `.tres`, `project.godot`, `.cfg`, `.import`, JSON, Markdown
      (theme syntax colors; Toggle Comment uses `//` in shaders, `;` in config files) — seen on screen

**LSP / Godot**
- [x] Outline panel (sidebar OUTLINE tab, nested, current symbol highlighted, remembered) and go to
      symbol in file (Ctrl+Shift+O / `@` in the palette) — seen on screen
      Filter at its top: by name (every word, any case) and by kind (class, func, signal, var, const, enum chips;
      click hides or shows a kind, right-click shows it alone); what matches inside a class or enum keeps its parent
      (faded) unless that kind is hidden, then it moves out a level; kind tags in one column, names indented; the filter stays from one file to the next — seen on screen
- [x] Go to symbol in project (Ctrl+T / `#` in the palette): classes, functions, signals, constants,
      variables and enums of every script, inner-class members included, open tabs read unsaved.
      Godot 4.7.1 has no `workspace/symbol`, so declarations are scanned client-side
      (`editor_core::symbols`); checked on a 26-script project — not yet tried on screen
- [x] Class reference (Shift+F1, palette; F1 in the Godot preset), in a "Docs" tab beside the files of
      its side (one at most; links reuse it; Ctrl+W closes it). It splits like a file tab: Ctrl+\,
      right-click menu (Close, Close Others, Move to the Other Side), focus follows clicks, and a
      side left with it alone stays open: every engine class, searchable, with
      inheritance (both ways, clickable), description, properties, methods, signals and constants;
      Godot's BBCode rendered, `[Class]` / `[method x]` references followed, Back, member filter;
      F12 or Ctrl+click on an engine class or member opens it there, and holding Ctrl underlines it
      (found from the hover signature). Comes from
      the language server (`gdscript/capabilities` + `textDocument/nativeSymbol`), not `--doctool`,
      whose XML has no descriptions in release builds — seen on screen (Node2D, Vector2.lerp);
      F12 path unit-tested only
- [x] PROJECT sidebar tab (palette: "View: Project"): main scene, autoloads (global or not; click
      opens the script/scene, `uid://` paths resolved) and Input Map actions with readable events
      ("A (Physical)", "Ctrl+S", "Joypad Start", "Left Stick Left"); click shows the line in
      `project.godot`, "uses" lists them in the SEARCH tab; reread when `project.godot` changes.
      Action names complete in strings passed to `is_action_pressed(` / `get_axis(` / `get_vector(`…
      (project actions, then the built-in `ui_*` ones; events shown as docs). Parser checked on 3 real
      projects (quoted names with spaces, uids) — not yet seen on screen
- [x] Drag a file from the explorer into code: `preload("res://…")` at the drop caret, the bare path
      inside a string or outside GDScript, Ctrl+drop gives `const BIG_ENEMY = preload(…)` (as in Godot);
      either side of the split — unit-tested, not yet seen on screen
- [x] New Script bar (right-click a folder or file, or "File: New Script..."): `res://` path with the
      name selected, extends, template (Godot's Default and Empty, plus the project's
      `script_templates/<Base>/` and `Object/` ones with `# meta-name/default/space-indent` and
      `_BASE_` / `_CLASS_` / `_CLASS_SNAKE_CASE_` / `_TS_`); creates missing folders, opens the file —
      unit-tested, not yet seen on screen
- [x] Godot version picker: click "Godot x.y" in the status bar (or "Godot: Choose Version..."): Automatic
      or any installed Godot, each marked as matching / newer / older than the project; kept in the
      project's session; used by the language server (restarted), Run, Open in Godot and new terminals —
      not yet seen on screen

**Debug**
- [x] DAP debugger, first stage: breakpoints (F9 or click a line number; red dots that follow edits and
      undo; kept in the session), Debug: Start (Alt+F5 main scene, Alt+F6 current scene) through the debug
      adapter of the Godot serving the language server (`--dap-port`, or the editor's 6006), paused line
      highlighted and shown, F5 continue, F10 / F11 / Shift+F11 step over / into / out, Shift+F5 stop; a
      DEBUG panel with the call stack (click a frame) and expandable variables (Locals / Members / Globals,
      expanded rows kept from one pause to the next); runtime errors pause at their line with the message;
      game prints go to Output. Stage two: while paused, hovering a name (or `a.b` chain, never a call)
      shows its value; watch expressions (re-evaluated at each pause, expandable, kept in the session).
      Protocol checked live against Godot 4.7.1 (examples `dap_probe`,
      `dap_client`); UI not yet seen on screen. Not yet: push_error / warnings in Output while debugging
      (Godot's adapter doesn't forward them), conditional breakpoints, editing values

**Terminal**
- [x] Multiple terminal tabs: + in the panel header (or "Terminal: New Terminal"), one chip per shell with
      its title, × or middle-click ends one, "kill" ends the active one, the last one closes the panel —
      not yet seen on screen (splits not done)
- [x] Clickable paths and URLs: Ctrl+click (Ctrl+hover underlines) `res://`, absolute and relative paths
      with `:line:column` or `(line,column)`, and http(s) URLs; files open at the line, folders in the file
      manager, URLs in the browser — detection unit-tested, not yet seen on screen

**Theming**
- [x] Shareable copy-paste theme codes (`GDTHEME1:` + compressed TOML, ~1 KB)
- [x] Import VS Code themes: "Theme: Import from VS Code..." lists the themes of the installed VS Code
      (built-in and extensions, by their labels), or Theme: Import File... takes a `.json`; UI, terminal and
      TextMate syntax colors mapped (most specific scope wins, `include`s merged, JSONC accepted), the rest
      from Godot Dark / Light; "Import all" at the top of the list takes every one at once (also for the
      Godot editor colors) — all 9 themes installed here convert, not yet seen on screen
- [x] Import the Godot editor's colors ("Theme: Import from the Godot Editor..."): text editor highlighting
      and interface base / accent colors of `editor_settings-4.x.tres` (one per Godot version, read only) —
      the 4 versions installed here convert, not yet seen on screen
- [x] Follow the OS light/dark mode ("Theme: Follow the OS Light/Dark Mode"): switches between a dark and a
      light theme (Godot Dark / Godot Light until another is picked: while following, picking a theme makes
      it the one for its kind); read at start and on every OS change; not while the theme editor is open —
      not yet seen on screen

**Workspace**
- [x] Resizable sidebar: drag the bar on its right (160 px minimum, the editor keeps 400 px), double-click
      it for the default 260 px; the width is kept in settings — not yet seen on screen
- [x] Dockable panels: every view (Files, Outline, Project, Problems, Output, Terminal, Search, Changes, Debug)
      lives in the left sidebar, the bottom panel or a new right sidebar; right-click its tab to move it ("Reset
      Panel Layout" there or in the palette puts them all back). Like the left one, the right sidebar is always shown
      while it holds a view (it goes once the last one moves out) and is resizable (double-click its edge for the
      default width); a sidebar showing the terminal starts a shell when a project
      opens. Layout and widths kept in settings.json. No drag and drop of tabs yet — seen on screen (moving Outline
      and Terminal to the right, restart, reset)
- [x] Remappable keybindings: click a shortcut on the F1 page and press the new keys (Backspace
      unbinds, "reset" restores); overrides in `keybindings.json`; menus and palette show the
      current keys; terminal passthrough follows the bindings
- [x] Explorer: right-click the empty space for the project root (New Script, New Folder); New Folder, Rename (Godot's `.uid` / `.import` companions follow, so `uid://` paths
      keep working; open tabs follow; files still using the old `res://` path are listed in SEARCH) and
      Delete (to the Recycle Bin with its companions, after a confirmation bar; its tabs close) —
      unit-tested, not yet seen on screen
- [x] Middle-click a tab to close it (the class reference tab too; unsaved tabs ask once, like ×)
- [x] Keybinding presets: Default, VS Code and Godot (buttons on the F1 page, or the palette);
      changes are kept on top of the preset in `keybindings.json`, picking a preset resets them
- [x] Settings UI + user/project JSON: a Settings page (Ctrl+, or "Preferences: Open Settings") with a filter,
      toggles and − / + steppers, "reset" on changed ones, and "Open JSON" to edit the file in a tab (rereads it
      on save). User scope writes `settings.json` in the config folder; Project scope writes overrides to
      `<project>/.beckett/settings.json` (Godot skips dot folders; the file goes once nothing is overridden;
      reread when it changes on disk), for the settings that allow it (others show "yours only"). New settings:
      auto-close brackets and quotes, suggest while typing, hover documentation delay, inline diagnostics,
      indentation guides, blinking cursor (editor and terminal); format on save and zoom moved onto the page —
      seen on screen (user and project scopes, an override applied live to an open tab)
- [x] Session restore: per-project tabs, active tab, cursors, folds; the last project reopens at
      launch; "File: Close Folder" goes back to the welcome screen

**Git**
- [x] Gutter change markers against the staged text (added / modified bars, removed-lines wedge; theme
      success / accent / danger), live while typing; explorer names colored (changed, new, conflict, and
      folders holding changes). Refreshed after saves, file changes, window focus and every 30 s — through
      the `git` command (no git or no repository: nothing shown); unit-tested on this repo, not yet seen on screen
- [x] Diff view: CHANGES tab in the bottom panel (palette "Git: Show Changes", or click a gutter marker for
      that file): every changed file with its hunks against the staged text (removed lines red, added
      green, 3 lines of context, unsaved edits of open tabs included), click a line to jump there, "revert"
      puts the staged lines back in the tab as one undo step (left unsaved) — revert unit-tested on the
      end-of-file newline cases, panel not yet seen on screen
- [x] Godot-aware `.gitignore` / `.gitattributes` ("Git: Add Godot .gitignore and .gitattributes"): the lines
      Godot 4 writes (`.godot/`, `/android/`, `* text=auto eol=lf`), appended only where missing

## Phase 3 — Godot editor replacement

- [ ] **ER1** Scene tree editor (add/remove/reparent/rename nodes, attach scripts) — `.tscn` read/write
  - [x] `.tscn` reader/writer (`godot-project::Scene`): untouched sections are written back as read (all 475
        scenes on disk round-trip byte for byte, CRLF kept); edited ones in Godot's layout. Node tree with paths,
        types, instanced scenes and scripts. Edits: add (a taken name gets a number, `unique_id` when the scene
        has them), rename, remove (with its connections, editable children and the resources only it used),
        reparent / reorder, attach / detach script; paths in parents, connections and `[editable]` follow,
        `load_steps` kept in step. Nodes of an instanced scene can't be renamed, moved or removed.
        Not yet: `NodePath` property values aren't rewritten when a node moves
  - [x] SCENE view lists the project's scenes as a folder tree like FILES (only folders holding scenes, and the
        `.tscn`s; arrows open / close folders), with a filter (every
        word, any case); a click opens the scene, the open one is highlighted, right-click gives the file menu;
        the list folds away (its header) and is refreshed when scenes or folders change on disk — not yet seen on screen
  - [x] SCENE view (left sidebar, dockable): the node tree of the `.tscn` in the active tab, indented, with
        each node's class, `tscn` / `gd` links to its instanced scene and script, the node under the cursor
        highlighted; a click shows the node's section. Right-click a node: Add Child Node (type picked in the
        palette, from the language server's classes or a common list), Rename (in place, Enter / Escape),
        Move Up / Down, Move To (palette), Attach / Change / Detach Script (palette of the project's scripts,
        `uid` read from the `.uid` file), Copy Node Path, Delete. An edit changes the tab's text as one undo step
        (Ctrl+S saves). Nodes of an instanced scene can't be renamed, moved or deleted — not yet seen on screen
  - [x] Godot's own node icons in the SCENE tree (all 1047 editor and module icons of Godot 4.7.1, MIT, packed in
        `assets/godot_icons.tsv`, notice in `assets/GODOT_ICONS_LICENSE.txt`). An instanced scene shows its root's
        class icon, and its inner nodes theirs (read from the instanced scene); a class without an icon takes its
        nearest parent's, else Node's. On light themes the colors are converted as the Godot editor does — not yet
        seen on screen. Not yet: icons of GDExtension classes (e.g. Terrain3D) and of `@icon` script classes
  - [x] Fold / unfold a node's children (arrow before nodes that have some). Instance Child Scene (right-click a
        node; palette of the project's other scenes; named after the scene's root, `uid` from its header). New
        Scene (right-click a folder in FILES or SCENES, or "Scene: New Scene..." for the active file's folder): root
        type in the palette, then its name; the file is the name in snake_case (`MainMenu` → `main_menu.tscn`),
        never replacing one, and opens; Godot adds its `uid` on its next save — not yet seen on screen
  - [ ] Drag and drop of nodes in the tree
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
