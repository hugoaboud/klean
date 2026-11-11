# KLEAN

A visual editor for Lean projects: pick a project root, browse its syntax tree as a
board, and rewrite the sources from a node editor.

Two pages:

- **Welcome** — a single centred card asking for a Lean project root (a folder, a zip
  archive, or an empty in-memory workspace).
- **Project** — a board with two resizable columns.
  - **Right:** the AST item tree — `Lib → Folder → File → AST nodes` — as small, dense
    rows. Clicking an item opens a draggable modal showing that node's source text in a
    highlight.js editor; editing it writes the file back. Declarations can also be
    renamed in place.
  - **Left:** a D3 force graph. Libraries, folders and files are **squares**: a square
    node with its name underneath, plus a square frame that encloses everything the
    group holds — so a file's AST visibly sits inside that file's square, inside its
    folder's square, inside its library's square. Declarations are drawn as pills with
    links only where one syntax node nests inside another. Libraries and folders start
    expanded; a file expands on click to reveal its AST, which expands again into
    declarations. Selection is synchronised both ways, and selecting in the tree
    expands and centres the graph. Geometry lives in `src/lib/graph-layout.ts`, kept
    free of Vue and D3 so the frame maths can be checked on its own.

The bottom **snackbar** switches the board between two views:

- **AST** — the graph and tree described above. Files can be created (the prompt starts in
  the selected file's folder) and deleted from the tree header; theorem rows carry a proof
  state: green check (no `sorry`), amber warning (`sorry`/`admit` in the body) or red mark
  (structural error). This is read from the source — KLEAN does not elaborate, so it is a
  signal, not a proof.
- **Logic** — the same board shape (graph left, tree right, movable divider), but the
  tree lists only axioms, definitions and theorems, grouped by namespace, and the graph's
  squares are those namespaces instead of files. Edges are *uses*: an identifier in one
  declaration that names another, coloured by the target kind (axiom / def / theorem).
  The eye on each tree row hides that item (a namespace hides its subtree) from the graph
  and dims it in the tree; the state lives in Rust. Clicking a term opens the same editor
  modal, smaller and resizable from its corner.

The logic board is split into three columns — **Types** (structures and `axiom … : Type`),
**Premises** (other axioms and definitions) and **Theorems** — with a dashed divider, and
nodes are clamped inside their column so the grouping holds while the layout works. Link
crossings are reduced by seeding each node's vertical target from the average height of its
partners (a barycentre pass) before the forces run.

The AST graph leaves `import` commands out — they are edges between files, not structure
inside one. Both graphs settle in about a second rather than drifting into place.

Interaction, on either board:

- drag a node, or a group's **header bar**, to move it; drag the background for a
  rubber-band selection (the tree supports the same rectangle); ctrl/cmd + drag to pan;
  wheel to zoom
- a click selects, ctrl/cmd + click adds to the selection, and only a **double click**
  opens the editor modal — a single click updates an open modal only when exactly one item
  is selected
- the modal marks every diagnostic line with an icon in the gutter and a soft line wash,
  lists only the diagnostics that fall inside the node, and offers a comments box that
  writes plain `--` comments back above the declaration
- `.klean.json` is the project file: graph positions, the camera of each board, hidden
  logic items and the project settings. Writes are batched — dragging updates memory and
  the file is rewritten at most every five seconds (and whenever the project is switched)
- AI settings are off by default; the model URL and the API key live in `.klean.secrets`
  (added to the project's `.gitignore` on first write), never in `.klean.json`
- the settings tree lives in Rust (`src-tauri/src/lean/settings.rs`); the settings button in
  the graph's top-right corner opens it. `Logic → Sections` decides which column each kind
  of declaration lands in, so renaming a section renames the column in both the graph and
  the tree
- hovering a graph node raises a small card with its state, type, namespace, name and the
  plain comments above it (loaded on demand through `node_detail`)
- every node has a fixed position: Rust places new nodes once, in tree order next to the
  rest of their namespace, spiralling out until nothing overlaps, and stores the result in
  `.klean.json` in the project root (positions survive edits and disappear with their node).
  Dragging a node saves it; dragging a group's header moves the whole group rigidly. There
  is no force simulation, so the picture never drifts. Links are routed orthogonally with
  rounded corners, draw.io style
- typing `\alpha` + space in the editor inserts `α`, the way the Lean VS Code extension
  does (`src/lib/lean-tex.ts` holds the table)

The welcome page also lists the three most recently opened projects (persisted in the app
config directory as `recent-projects.json`).

An **outline** action prints the whole project as human-readable Lean with comments that
describe the sections, and can save it as a `.lean` file.

## Requirements

- Rust (stable) and the Tauri prerequisites for your platform
- Node 20+ and npm
- No Lean toolchain: parsing is done in Rust, so KLEAN works without Lean or lake
  installed.

## Running

```bash
npm install
npm run tauri dev          # development (Vite + Tauri)
npm run tauri build        # production bundle
```

The binary also takes an optional project path, which skips the welcome page:

```bash
cargo run --manifest-path src-tauri/Cargo.toml -- /path/to/lean/project
cargo run --manifest-path src-tauri/Cargo.toml -- project.zip
```

### If `tauri dev` reports a Vue style error

```
[vite] Internal server error: Cannot read properties of undefined (reading 'scoped')
  File: .../src/App.vue?vue&type=style&index=1&lang.css
```

A WebKit webview caches the compiled modules it has already fetched, so after a
component loses a `<style>` block the cached module can still ask Vite for that
block's index. `@vitejs/plugin-vue` throws on such a request, which used to turn
into a 500 and an error overlay. `vite.config.ts` now answers out-of-range style
requests with an empty module and logs `[klean] ignored stale style request …`, so
the app keeps loading. To clear the cached module outright, stop the app and
delete the webview data directory:

```bash
rm -rf ~/.local/share/com.aboud.klean
```

## Layout

```
src-tauri/src/lean/          the engine (no UI concerns)
  fs.rs                      project storage on rust-vfs: PhysicalFS on disk,
                             MemoryFS for archives, zip import/export, ignored dirs
  syntax/lexer.rs            Lean 4 tokenizer (comments, doc comments, strings,
                             escaped and dotted names, brackets)
  syntax/ast.rs              syntax tree types: spans, node kinds, doc comments
  syntax/parser.rs           structural parser: commands, namespaces, sections,
                             structure fields, inductive constructors, where blocks
  project.rs                 lakefile discovery, Lib/Folder/File model, node ids,
                             tree/graph DTOs, detail lookup, edits, write-back
  printer.rs                 human-readable Lean printer with outline comments
  dto.rs                     the serialisable surface the front-end renders
  error.rs                   error type
src-tauri/src/commands.rs    Tauri commands, one open workspace behind a mutex
src/                         Vue front-end: components, pages, store, API wrappers
```

All project logic lives in Rust. Vue owns only presentation and view state (which page
is visible, whether the modal is open, which tree rows are unfolded, split ratio, graph
expansion is *not* view state — Rust owns it).

## How the parser works

Lean has no command terminator: a command ends where the next one begins. The parser
splits a file at command starts, detected as a command keyword (or an attribute/modifier
run such as `@[simp] private`) at the top bracket level of the current block, at the
start of a line, and not indented deeper than the command it would otherwise continue.
`namespace`, `section` and `mutual` blocks nest until their `end`.

This is a *structural* tree, not an elaborated Lean `Syntax` tree: expressions, proofs
and tactic blocks are kept verbatim, which is what makes editing safe — an edit replaces
exactly the node's byte span and leaves everything around it untouched. Diagnostics
report structural problems only (unbalanced brackets, missing `end`); no type checking
happens here.

## Storage

Every file operation goes through [`vfs`](https://github.com/manuel-woelker/rust-vfs), so
the same engine runs against a real folder (`PhysicalFS`) or an in-memory workspace
(`MemoryFS`) restored from a zip archive. That is the seam a browser build would use;
the desktop app already exercises both backends, and the archive actions import/export
through it.
