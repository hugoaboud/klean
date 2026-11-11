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
  state that Lean itself reported: green check (checked, no `sorry`), amber warning (Lean's
  `declaration uses 'sorry'` message) or red mark (an error inside the declaration).
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
- the right-hand column of the modal is what Lean says about the node: the **proof state**
  (the goals left at the caret, asked of Lean's language server) and the **diagnostics**
  inside the node, worst first
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
- **A Lean 4 toolchain** for everything that touches Lean: the syntax tree, the
  diagnostics and the goals. KLEAN does not parse Lean itself, so it needs a real one —
  either the `lean`/`lake` on your `PATH` (elan, or a manual install) or a toolchain
  installed from *Settings → Lean* (the app downloads elan and the channel you pick).
  Archives and in-memory workspaces are browsable but cannot be checked: Lean needs a
  folder with a `lakefile` to run in.

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
lean4-ast/                   Lean's own parser, as a static library + Rust crate
  build.py                   pulls the pinned lean4 tag, extracts it, builds the
                             library and runs the crate's tests
  manifest.json              which lean4 files matter, and why (verified by build.py)
  wrapper/KleanAst.lean      thin wrapper: Lean.Parser → JSON syntax tree
  shim/klean_ast_shim.c      starts the Lean runtime, exposes three C functions
  crates/klean-ast/          the Rust API and its per-feature tests
src-tauri/src/lean/          the engine (no UI concerns)
  fs.rs                      project storage on rust-vfs: PhysicalFS on disk,
                             MemoryFS for archives, zip import/export, ignored dirs
  toolchain.rs               finds the `lean`/`lake` to use (PATH, elan, settings, or
                             the one KLEAN installs through elan), runs `lake env` and
                             `lake serve`, downloads elan on request
  bridge.rs                  keeps the Lean syntax helper alive and talks to it
  lsp.rs                     the language server on a worker thread: diagnostics,
                             processing progress, `$/lean/plainGoal`, incremental edits
  engine.rs                  one project's Lean: toolchain, parser, server, status
  ast.rs                     the syntax model the board renders, plus the rules that
                             turn Lean's syntax tree into rows (block nesting,
                             comments above a node, signature slicing)
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

## How Lean is used

KLEAN contains no Lean parser. Everything the board shows is what the official
implementation reported:

- **syntax tree** — `lean4-ast/` builds Lean's *parser* into a static library that KLEAN
  links: `Lean.Parser.parseHeader` + `parseCommand`, no elaboration. Parsing a file is a
  library call in this process (microseconds), so no project file ever spawns a process.
  A file is parsed twice when it imports something — once to read the imports, once with
  their syntax extensions loaded — and if an import has not been built yet the tree falls
  back to core syntax instead of disappearing.
- **diagnostics and proof goals** — `lake serve` (or `lean --server` outside a Lake
  package) is the only process KLEAN runs. It stays up for the whole project, on a worker
  thread, with incremental `didChange`s; the board reads the diagnostics and the processing
  progress it publishes, and the modal asks `$/lean/plainGoal` for the caret position. No
  UI call waits for Lean: the board shows *checking …* at the top right while the server
  works, and the modal says the same for the file it has open.
- **build** — `lake build` makes the project's `.olean` files, which is what lets a file
  see its imports — for the parser's syntax extensions and for the server's checks alike.

The tree KLEAN builds from those records is *structural*, like the sources: expressions and
proofs stay verbatim, so an edit replaces exactly a node's byte span and leaves everything
around it untouched. `namespace`/`section`/`mutual` blocks are nested by pairing their
commands with the matching `end`, and doc/plain comments are recovered from the text
between one command and the next (Lean's tree does not carry them).

### What opening a project costs

Nothing runs until you ask. Opening a project reads the file list and parses it with **one**
Lean environment (`Init`/`Lean`): no project module is loaded, so memory does not grow with
the project. Measured here: 150 files and 600 files both open at ~210 MB, and no Lean
process is started.

- **Parsing** a whole project is microseconds per file. Only the file you open in the editor
  is re-parsed with *its* imports loaded, and only once the project has been built; the
  parser keeps at most two such environments, so a session cannot accumulate them.
- **Checking** is per file: opening a node hands that one file to the language server, which
  keeps its own process and reports diagnostics as it finds them. Editing a node sends only
  that file's change (one replaced range) and re-checks only that file; the rest of the
  project is untouched, so typing stays responsive no matter how large the project is.
- **The welcome page does nothing**: no parsing, no server, no build — the binary sits at
  ~200 MB and no Lean process exists until a project is opened.
- **Building** is the one expensive action, so it is a question, not a side effect: when a
  project opens KLEAN asks whether to run `lake build` (the *Lean → Build on open* setting
  can answer `ask`, `always` or `never`). A build runs in the background with a console that
  streams `lake build`'s own output, a cancel button, and an address-space ceiling
  (*Lean → build memory*, `auto` by default) so a runaway build fails instead of taking the
  machine with it.

The parser library is pinned to one Lean version (its static link can only read that
version's `.olean` files), so KLEAN resolves a toolchain of exactly that version — the one
`lean4-ast/build.py` built with, or an installed elan toolchain of the same version — and
says so plainly when there is none.

KLEAN's library is built as `staticlib` + `rlib`, never `cdylib`: Lean's runtime uses
initial-exec TLS, which a shared library cannot carry. That is also why a mobile build of
this app would need its own parser story.

### The Lean settings

`Settings → Lean` (the settings tree lives in `src-tauri/src/lean/settings.rs`):

- **Use Lean** — off leaves the board read-only: files are listed, nothing is checked
- **Toolchain** — `auto` (the `lean` on PATH, then elan homebrew spots), `managed` (the one
  KLEAN installed), or `custom` with a path
- **toolchain override** — an elan channel such as `leanprover/lean4:stable`, or empty to
  use the project's `lean-toolchain`
- **install channel** and **Install toolchain** — downloads the elan release into the app
  data directory and installs that channel (~200 MB), then switches the toolchain to
  `managed`. Progress lines appear on the button.
- **Build on open** — `ask` (default), `always` or `never`; a build always streams into its
  console
- **build memory** — address-space ceiling for `lake build` (`auto` = the machine's memory)
- **timeout (s)** — how long one file may take before Lean is restarted

## Storage

Every file operation goes through [`vfs`](https://github.com/manuel-woelker/rust-vfs), so
the same engine runs against a real folder (`PhysicalFS`) or an in-memory workspace
(`MemoryFS`) restored from a zip archive. That is the seam a browser build would use;
the desktop app already exercises both backends, and the archive actions import/export
through it.
