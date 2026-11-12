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
- a click selects, ctrl/cmd **or shift** + click adds to the selection, and only a **double
  click** opens the editor modal — a single click updates an open modal only when exactly
  one item is selected. Shift works the same way with the rubber band: the rectangle adds
  what it touches to the selection instead of replacing it
- the modal marks every diagnostic line with an icon in the gutter and a soft line wash,
  lists only the diagnostics that fall inside the node, and offers a comments box that
  writes plain `--` comments back above the declaration
- the right-hand column of the modal is what Lean says about the node, with syntax
  highlighting and a divider you can drag to give either side more room: the **term** under
  the caret (`textDocument/hover`), the **proof state** (the goals left at the caret from
  `$/lean/plainGoal`) and the **diagnostics** inside the node, worst first
- a node's **comments** are the `--` lines sitting directly above it — a blank line ends the
  note, and block comments are prose, not a note. KLEAN writes them back as `--` lines,
  wrapped at 200 characters
- `.klean.json` is the project file: graph positions, the camera of each board, hidden
  logic items and the project settings. Writes are batched — dragging updates memory and
  the file is rewritten at most every five seconds (and whenever the project is switched)
- AI settings are off by default; the model URL and the API key live in `.klean.secrets`
  (added to the project's `.gitignore` on first write), never in `.klean.json`
- **AI** is a modal of its own (`src-tauri/klean-ai`): every request opens a *session*
  (one tab), the modal keeps them all until you close them, and the `AI` button in the top
  bar brings it back. A session's feed holds four kinds of entry — KLEAN's **context** (the
  reduced Lean source), your **input**, the model's **output** (reasoning, text, or a
  generated markdown document) and the **MCP tool calls** it made. Closing the modal keeps
  the sessions; nothing is persisted across restarts.
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

### The AI modal and its tools

An AI request starts where it makes sense — the editor's prompt box asks about the node you
are reading — and opens a session seeded with that node. Before a prompt is sent, KLEAN
writes the **reduced Lean source** for the request: the declaration, everything it uses
(followed recursively through the logic board's use edges), their doc comments and notes,
and the imports those files declare. A model therefore sees a small compilable file rather
than the project.

Models can look further with the six tools KLEAN implements itself (`Settings → AI →
Tools` switches each one off or on):

| tool | what it does |
| --- | --- |
| `view_ast` | lists the project's declarations as text, filtered by a query |
| `expand` | pulls more nodes into the session's context, by fully qualified name |
| `lean_check` | Lean's own `#check` for an expression, with the whole project in scope |
| `diagram` | shows a mermaid diagram to the user |
| `ask` | asks the user, with answers to pick from; the worker waits for the answer |
| `ask_multi` | the same, allowing several answers |

`Settings → AI → Customize → System Prompt` appends your own instructions to KLEAN's built-in
system prompt for every session — the built-in rules stay (a user shouldn't have to restate
what the tools are), and the customisation is introduced as taking precedence. The built-in
text lives in `src-tauri/klean-ai/src/prompts.rs`; the custom one is stored in `.klean.json`.

The **+** button in the tab row starts a session with **no context at all** — nothing from the
board, no Lean source — for a question that is not about one theorem. The model finds what it
needs with `view_ast` and `expand`, or asks Lean directly with `lean_check`. The prompt box
belongs to the open session, so with no tab there is none.

`lean_check` runs `lake env lean --stdin --json` over a source that imports every module the
project has built and then the model's expression, so Mathlib and the project's own
declarations are in scope. `lake env` is what puts the dependencies on Lean's search path; only
built modules are imported, because one unbuilt import would end the check. The run happens
outside the project lock, so the board stays usable while Lean thinks, and it is bounded by the
*Lean → timeout* setting. Without build output the tool says so instead of guessing.

A `diagram` call is drawn *before* the model is told it worked: the modal renders the call's own
mermaid source and reports back whether it parsed. When it did not, mermaid's complaint goes to
the model as the tool result and it may retry — up to three retries, after which `diagram` is
refused and it is asked to answer in words. (The wire the model reads says "the diagram is on
screen"; the picture comes from the call's `mermaid` argument, never from that sentence.)

Every text a model is shown — the system prompt, each tool's description and parameters, the
line that introduces the context, and each tool's result — lives in
`src-tauri/klean-ai/src/prompts.rs`, so it can be reworded without touching code.

Answers are shown as a markdown preview (`marked`, sanitised with `dompurify`) and
```mermaid fences are drawn as diagrams (mermaid is loaded the first time one appears).
Every entry except the model's own output folds to a one-line preview — a context, a prompt
and each tool call with the arguments it was given. The model's answer is rendered markdown at
1.5× the surrounding text, since it is what the tab is for.

A context entry has two views: **graph**, which draws the declarations it holds and which one
pulled which in, and **lean**, the reduced file that is actually sent. Declarations stay
inside their original namespaces, so a name that resolves among its siblings still resolves
there. `expand` answers with the *new* context, which also appears in the feed as its own
entry.

The line under the feed reports tokens **in · out · total** and, when the model has a
published price, what the session has cost in dollars (four decimals); a model KLEAN does not
know the price of shows tokens only.

Three provider families are configured in `Settings → AI → Model`: **DeepSeek**,
**Claude** (Anthropic's Messages API) and anything **OpenAI-compatible**. Streaming is plain
blocking IO on a thread per session, which is what lets several sessions run at once, and
reasoning is shown when the provider exposes it (`reasoning_content` on DeepSeek,
`thinking_delta` on Claude).

### What opening a project costs

Opening a project parses **nothing**: the board is built from the file listing, so it is on
screen in a few hundred milliseconds however large the project is. The declarations arrive
afterwards, from a background job (`src-tauri/src/lean/scan.rs`) that reads the files one at
a time and reports into the build console — `3/16 files · Core/Types.lean` — with a stop
button. Until it reaches a file, that file's node says `not parsed yet`.

The read reuses **one** Lean environment (`Init`/`Lean`) for the whole project: no project
module is loaded, so memory does not grow with the project. Measured here: 150 files and
600 files both open at ~210 MB, and no Lean process is started.

- **Building that environment is the one slow step** — Lean imports its own `Init` and
  `Lean` modules, seconds rather than milliseconds — so it happens on the scan's own thread,
  *off* the project lock, and everything after it is milliseconds. While it runs, commands
  that would need the parser answer with a note instead of queueing behind it.
- **Parsing** a file is microseconds. Only the file you open in the editor is re-parsed with
  *its* imports loaded, and only once the project has been built; the parser keeps at most
  two such environments, so a session cannot accumulate them.
- **Checking** is per file: opening a node hands that one file to the language server, which
  keeps its own process and reports diagnostics as it finds them. Editing a node sends only
  that file's change (one replaced range) and re-checks only that file; the rest of the
  project is untouched, so typing stays responsive no matter how large the project is.
- **Typing is checked, not saved.** KLEAN never writes while you type: the edited declaration
  is spliced into the file *in memory*, handed to Lean at once, and the diagnostics come back
  under the caret — the board's tree is left out of that path, which is what keeps a keystroke
  cheap. The file on disk changes only when you save (Ctrl/Cmd+S or the *save* button), and
  closing the editor with an unsaved edit asks first, like the settings modal. Discarding puts
  the saved text back in front of Lean, so nothing stale is left behind.
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
