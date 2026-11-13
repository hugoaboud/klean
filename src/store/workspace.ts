// Reactive application state.
//
// This is a thin shell: every value here comes from Rust, every mutation is a
// Rust command. The only thing the front-end owns is which page is visible,
// whether the editor modal is open, and which board view is selected.

import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api/lean";

export type Page = "splash" | "welcome" | "project";
export type BoardView = "ast" | "logic";

export interface Printed {
  title: string;
  subtitle: string;
  text: string;
}

/**
 * One editor modal.
 *
 * A pane is a *view* of one node: the text it holds lives in its editor component
 * (with that component's own undo history), and what is here is what the rest of
 * the app needs — the node's detail, the goals under the caret, and whether there
 * are unsaved changes.
 */
export interface EditorPane {
  id: string;
  nodeId: string;
  detail: api.NodeDetail | null;
  goals: api.Goals | null;
  goalsBusy: boolean;
  /** The type of the term at the caret, as Lean renders it. */
  term: string | null;
  termBusy: boolean;
  /** Where the modal sits and how big it is, so the next one can cascade from it. */
  geometry: PaneGeometry | null;
  /** True while the editor holds more than one caret. */
  multi: boolean;
  /** Goals of the whole declaration, not of the caret: what the icon shows. */
  nodeGoals: api.Goals | null;
  /** The pane holds an edit the disk has not seen. */
  dirty: boolean;
  saving: boolean;
  error: string | null;
  /** The close button was pressed with an unsaved edit: ask before dropping it. */
  confirming: boolean;
  /**
   * A node the user asked for while this pane held an unsaved edit: once the edit
   * is discarded, the pane shows this instead of closing.
   */
  pending: string | null;
  /** The node's detail is being fetched; the old one stays on screen until it comes. */
  loading: boolean;
}

/** Where an editor modal sits, and how big it is. */
export interface PaneGeometry {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** How far a new modal steps up and to the left from the one it came from. */
const CASCADE = 28;

interface State {
  page: Page;
  boardView: BoardView;
  busy: string | null;
  error: string | null;
  notice: string | null;
  view: api.WorkspaceView | null;
  graph: api.ProjectGraph | null;
  logic: api.LogicGraph | null;
  /** Theory whose graph is open; empty means the root graph. */
  openTheory: string;
  recents: api.RecentProject[];
  selectedIds: string[];
  selectedId: string | null;
  /** Open editors, in stacking order: the last one is on top and takes keys. */
  editors: EditorPane[];
  /** Which pane a plain double-click, or the next keystroke, belongs to. */
  activeEditor: string | null;
  /** Where the last closed modal was: the next one opens there. */
  lastGeometry: PaneGeometry | null;
  printed: Printed | null;
  printedOpen: boolean;
  settings: api.SettingNode[];
  settingsOpen: boolean;
  /** Details of the node under the pointer, for the graph's info card. */
  hover: api.NodeDetail | null;
  /** Lean's toolchain and build state for this project. */
  lean: api.LeanStatus | null;
  /** Goals of the node open in the editor, from Lean's language server. */

  /** Last line the toolchain installer printed. */
  install: string | null;
  /** The `lake build` console (empty until a build is started). */
  build: api.BuildState | null;
  /** True while the build console is open. */
  buildOpen: boolean;
  /** True when the editor holds text Lean has checked but the disk has not seen. */

  /** True while the parse modal holds the project page back. */
  parseOpen: boolean;
  /** Why the read cannot run at all, if it cannot (Lean off, no toolchain). */
  parseBlocked: string | null;
  /** The project the parse modal is reading, for its heading. */
  parseLabel: string;
  /** How the read of the project stands (null before one starts). */
  scan: api.ScanState | null;
  /** True while Lean is still processing at least one file. */
  processing: boolean;
  /** Files being processed, for the modal's indicator. */
  processingFiles: string[];
}

export const state = reactive<State>({
  page: "splash",
  boardView: "ast",
  busy: null,
  error: null,
  notice: null,
  view: null,
  graph: null,
  logic: null,
  openTheory: "",
  recents: [],
  selectedIds: [],
  selectedId: null,
  editors: [],
  activeEditor: null,
  lastGeometry: null,
  printed: null,
  printedOpen: false,
  settings: [],
  settingsOpen: false,
  hover: null,
  lean: null,
  install: null,
  build: null,
  buildOpen: false,
  parseOpen: false,
  parseBlocked: null,
  parseLabel: "",
  scan: null,
  processing: false,
  processingFiles: [],
});

async function run<T>(label: string, action: () => Promise<T>): Promise<T | null> {
  state.busy = label;
  state.error = null;
  try {
    return await action();
  } catch (error) {
    state.error = error instanceof Error ? error.message : String(error);
    return null;
  } finally {
    state.busy = null;
  }
}

function setSelection(ids: string[]) {
  state.selectedIds = ids;
  state.selectedId = ids.length === 1 ? ids[0] : null;
}

/** Only real source nodes have a detail; namespace/section ids do not. */
const DETAILABLE = /^(ast|file|lib|dir|astgrp):/;

/** The file a node id lives in, when the id says so. */
export function fileOfId(id: string): string | null {
  if (id.startsWith("file:")) return id.slice("file:".length);
  if (id.startsWith("ast:")) return id.slice("ast:".length).split("#")[0] ?? null;
  if (id.startsWith("astgrp:")) return id.slice("astgrp:".length);
  return null;
}

// ------------------------------------------------------------------ editors

let paneSeq = 0;

/** The pane the user last opened, clicked or dragged: the one that takes input. */
export function activePane(): EditorPane | null {
  return state.editors.find((pane) => pane.id === state.activeEditor) ?? null;
}

/** Puts a pane on top: it is now the one a plain double-click retargets. */
function raisePane(pane: EditorPane) {
  const index = state.editors.indexOf(pane);
  if (index >= 0) state.editors.splice(index, 1);
  state.editors.push(pane);
  state.activeEditor = pane.id;
}

/**
 * Where a new modal goes.
 *
 * With one open, it keeps that modal's size and steps up and to the left from it,
 * so a stack of editors reads as a stack. With none open, it takes the place the
 * last closed one had. With neither, the modal centres itself.
 */
function nextGeometry(): PaneGeometry | null {
  const open = [...state.editors].reverse().find((pane) => pane.geometry);
  if (open?.geometry) {
    return {
      x: Math.max(0, open.geometry.x - CASCADE),
      y: Math.max(0, open.geometry.y - CASCADE),
      w: open.geometry.w,
      h: open.geometry.h,
    };
  }
  return state.lastGeometry;
}

function createPane(nodeId: string): EditorPane {
  paneSeq += 1;
  const pane: EditorPane = {
    id: `pane-${paneSeq}`,
    nodeId,
    detail: null,
    goals: null,
    goalsBusy: false,
    term: null,
    termBusy: false,
    geometry: nextGeometry(),
    multi: false,
    nodeGoals: null,
    dirty: false,
    saving: false,
    error: null,
    confirming: false,
    pending: null,
    loading: false,
  };
  state.editors.push(pane);
  state.activeEditor = pane.id;
  return pane;
}

/**
 * The modal a plain double-click should replace: one showing the same file, or the
 * one the user last touched.
 */
function relatedPane(id: string): EditorPane | null {
  const wanted = fileOfId(id);
  const wantedIsFile = id.startsWith("file:");
  if (wanted) {
    // A file modal and a node modal of the same file are never open together: the
    // one that is there is the one that changes place.
    const related = state.editors.find((pane) => {
      const path = pane.detail?.path ?? fileOfId(pane.nodeId);
      return path === wanted && !pane.nodeId.startsWith("file:") !== !wantedIsFile;
    });
    if (related) return related;
  }
  return activePane();
}

/**
 * True when another editor of the same file holds an unsaved edit.
 *
 * Two editors of one file with two different unsaved versions is a merge problem
 * nobody asked for: one of them is read-only until the other has been saved.
 */
export function isLocked(pane: EditorPane): boolean {
  const rel = pane.detail?.path;
  if (!rel) return false;
  return state.editors.some(
    (other) => other.id !== pane.id && other.dirty && other.detail?.path === rel,
  );
}

/** Loads one pane's node, replacing whatever it showed. */
async function loadPane(pane: EditorPane) {
  // The old detail stays until the new one arrives: emptying it would unmount the
  // editor (and with it Monaco) on every step between two nodes.
  const wanted = pane.nodeId;
  pane.loading = true;
  pane.goals = null;
  pane.term = null;
  pane.nodeGoals = null;
  pane.multi = false;
  pane.dirty = false;
  pane.error = null;
  try {
    const detail = await api.nodeDetail(wanted);
    // The pane may have been pointed elsewhere while this was in flight.
    if (pane.nodeId === wanted) pane.detail = detail;
  } catch (error) {
    pane.error = error instanceof Error ? error.message : String(error);
  } finally {
    if (pane.nodeId === wanted) {
      pane.loading = false;
      void loadNodeGoals(pane);
    }
  }
}

/**
 * Asks Lean for the goals at the *end of the declaration*, which is what says
 * whether the node is proved — independently of where the caret happens to be.
 */
async function loadNodeGoals(pane: EditorPane) {
  if (!pane.detail || pane.detail.kind === "file") {
    pane.nodeGoals = null;
    return;
  }
  try {
    pane.nodeGoals = await api.leanGoals(pane.nodeId);
  } catch {
    pane.nodeGoals = null;
  }
}

/** Re-reads a pane's node, keeping its text and history (for details arriving later). */
async function refreshPane(pane: EditorPane) {
  try {
    pane.detail = await api.nodeDetail(pane.nodeId);
  } catch {
    // The node may be gone after an external change; the pane keeps what it has.
  }
}

/**
 * Brings every other editor of a file up to date, after the file changed.
 *
 * Two editors of one file are never edited at once: while one holds an unsaved
 * edit the others are locked (see [`isLocked`]). This is what happens when that
 * edit is saved or thrown away — the file on disk is now the truth for all of them,
 * so they take it and unlock.
 */
async function refreshFileEditors(rel: string | null | undefined, except: EditorPane) {
  if (!rel) return;
  for (const pane of state.editors) {
    if (pane === except || pane.detail?.path !== rel) continue;
    await refreshPane(pane);
    pane.dirty = false;
  }
}

function adopt(view: api.WorkspaceView) {
  state.view = view;
  state.boardView = "ast";
  setSelection([]);
  state.editors = [];
  state.activeEditor = null;
  state.printed = null;
  state.printedOpen = false;
  state.logic = null;
  state.openTheory = "";
  state.page = "project";
  state.lean = null;
  state.processing = false;
  state.processingFiles = [];
  state.scan = null;
  state.parseOpen = false;
  state.parseBlocked = null;
  void (async () => {
    await loadSettings();
    await loadLean();
    await pollBuild();
  })();
}

async function loadGraph() {
  const graph = await run("graph", () => api.graphView());
  if (graph) state.graph = graph;
}

/**
 * Re-reads Lean's state. `build` asks for `lake build` first, which is what
 * makes imports resolve for every later check.
 */
async function loadLean() {
  const status = await run("lean", () => api.leanStatus());
  if (!status) return;
  state.lean = status;
}

/** True when this project can be built at all (a Lake package with `lake`). */
export function canBuild(): boolean {
  return Boolean(state.lean?.available && state.lean?.lake);
}

/** Whether to ask before building when a project opens. */
export function buildPrompt(): "ask" | "always" | "never" {
  const value = settingValue("lean.build.prompt", "ask");
  return value === "always" || value === "never" ? value : "ask";
}

let parseTimer: number | null = null;
/** Set when the user closes the modal: the read must not open the project. */
let parseStopped = false;

/**
 * Reads a project's files, with the modal that reports it.
 *
 * This is the only way onto the project page: opening a folder walks the tree and
 * stops there, the scan parses every file, and the board is built from the result.
 * Closing the modal stops the read and closes the project, so no board is ever
 * shown for files that were not parsed.
 */
async function beginParse(summary: api.ProjectSummary) {
  // A second open replaces the first: its poll must not outlive it.
  if (parseTimer !== null) {
    window.clearTimeout(parseTimer);
    parseTimer = null;
  }
  parseStopped = false;
  state.parseOpen = true;
  state.parseBlocked = null;
  state.scan = null;
  state.notice = null;
  state.error = null;
  state.page = "welcome";
  state.parseLabel = summary.label;
  const scan = await api.leanScan().catch((error) => {
    state.parseBlocked = error instanceof Error ? error.message : String(error);
    return null;
  });
  if (!scan) return;
  state.scan = scan;
  // Nothing can be read here: Lean is off, there is no toolchain, or this is not a
  // folder. The modal says so and offers to open the project as it is.
  if (scan.blocked) {
    state.parseBlocked = scan.blocked;
    return;
  }
  if (!scan.running) {
    await finishParse();
    return;
  }
  pollParse();
}

/** Follows the read, then puts the board on screen. */
function pollParse() {
  if (parseTimer !== null) return;
  const tick = async () => {
    const scan = await api.leanScanState().catch(() => null);
    if (parseStopped) {
      parseTimer = null;
      return;
    }
    if (!scan) {
      parseTimer = null;
      return;
    }
    state.scan = scan;
    if (scan.running) {
      parseTimer = window.setTimeout(tick, 200);
      return;
    }
    parseTimer = null;
    await finishParse();
  };
  parseTimer = window.setTimeout(tick, 200);
}

/**
 * The board catches up with the files once the last editor closes.
 *
 * While an editor is open its modals cover the board, so re-reading the whole
 * project on every keystroke would only cost: this is the one refresh that matters.
 */
async function refreshAfterClose() {
  const view = await api.workspaceView().catch(() => null);
  if (view) state.view = view;
  await reloadBoards();
}

/** Every file is in: build the board and hand the project page over. */
async function finishParse() {
  if (parseStopped) return;
  const view = await api.workspaceView().catch((error) => {
    state.error = error instanceof Error ? error.message : String(error);
    return null;
  });
  if (!view) {
    state.parseOpen = false;
    return;
  }
  adopt(view);
  await Promise.all([loadGraph(), loadRecents()]);
  if (state.boardView === "logic") void loadLogic();
}

/** Reads the build console from Rust (cheap: it is a snapshot). */
async function pollBuild() {
  const build = await api.leanBuildState().catch(() => null);
  if (!build) return;
  state.build = build;
  if (build.running) {
    window.setTimeout(() => void pollBuild(), 700);
    return;
  }
  if (build.summary && !build.failed) {
    // A finished build changes what Lean can resolve.
    const view = await api.workspaceView().catch(() => null);
    if (view) state.view = view;
    await reloadBoards();
  }
}

/**
 * Polls Lean while it is still processing: the worker keeps checking in the
 * background, and the boards are re-read whenever it becomes idle again.
 */
async function pollProcessing() {
  if (pollTimer !== null) return;
  const tick = async () => {
    const status = await api.leanStatus().catch(() => null);
    if (!status) {
      pollTimer = null;
      return;
    }
    state.lean = status;
    state.processing = status.processing.length > 0;
    state.processingFiles = status.processing;
    if (state.processing) {
      pollTimer = window.setTimeout(tick, 600);
      return;
    }
    pollTimer = null;
    // Lean settled: pick up the diagnostics it published.
    //
    // While the editor holds an unsaved edit, only the node is refreshed: walking
    // the whole project for the boards after every keystroke is what made checking
    // feel slow, and the boards are about saved work anyway.
    // The boards are behind the editor modals while one is open: walking the whole
    // project for them after every keystroke is what made the editor feel stuck.
    const editing = state.editors.length > 0;
    if (!editing) {
      const view = await api.workspaceView().catch(() => null);
      if (view) state.view = view;
      await reloadBoards();
    }
    for (const pane of state.editors) await refreshPane(pane);
  };
  pollTimer = window.setTimeout(tick, 120);
}

/** A whole-number setting; anything else falls back. */
export function numberSetting(id: string, fallback: number): number {
  const value = Number.parseInt(settingValue(id, String(fallback)), 10);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

/** How long typing has to pause before Lean is asked about the node. */
export function checkDebounce(): number {
  return numberSetting("editor.check_debounce", 300);
}

/** The editor's shortcuts, as the settings spell them. */
export function editorKeys() {
  return {
    undo: settingValue("editor.keys.undo", "Ctrl+Z"),
    redo: settingValue("editor.keys.redo", "Ctrl+Shift+Z"),
    find: settingValue("editor.keys.find", "Ctrl+F"),
    addOccurrence: settingValue("editor.keys.add_occurrence", "Ctrl+D"),
    allOccurrences: settingValue("editor.keys.all_occurrences", "Ctrl+Shift+L"),
    cursorAbove: settingValue("editor.keys.cursor_above", "Alt+Shift+ArrowUp"),
    cursorBelow: settingValue("editor.keys.cursor_below", "Alt+Shift+ArrowDown"),
    duplicateUp: settingValue("editor.keys.duplicate_up", "Ctrl+Alt+Shift+ArrowUp"),
    duplicateDown: settingValue("editor.keys.duplicate_down", "Ctrl+Alt+Shift+ArrowDown"),
  };
}

/** The shortcut that closes the editor modal, from settings. */
export function closeKey(): string {
  return settingValue("editor.keys.close", "Ctrl+W");
}

/** The modifier held with a double-click to open another editor. */
export function newEditorKey(): string {
  return settingValue("editor.keys.new_editor", "Ctrl");
}

/** The modifier held while dragging a board to look around. */
export function panKey(): string {
  return settingValue("editor.keys.pan", "Ctrl");
}

/** Current value of one setting leaf, or `fallback` when it is unknown. */
function settingValue(id: string, fallback: string): string {
  const walk = (nodes: api.SettingNode[]): string | null => {
    for (const node of nodes) {
      if (node.id === id) return node.value;
      const inner = walk(node.children);
      if (inner !== null) return inner;
    }
    return null;
  };
  return walk(state.settings) ?? fallback;
}

/**
 * Goals for the node in the editor at `line`/`character`, both relative to the
 * node's own text. Without a position, the end of the declaration is asked.
 */
/** Asks Lean what the term under a pane's caret is. */
async function loadTerm(pane: EditorPane | null, line?: number, character?: number) {
  if (!pane) return;
  if (pane.multi) {
    // Several carets, several terms: there is no single type to ask about.
    pane.term = null;
    return;
  }
  pane.termBusy = true;
  try {
    const term = await api.leanHover(pane.nodeId, line, character);
    // The caret may have moved on to another pane's node in the meantime.
    if (pane.nodeId) pane.term = term;
  } catch {
    pane.term = null;
  } finally {
    pane.termBusy = false;
  }
}

/** The goals a pane's caret is looking at. */
async function loadGoals(pane: EditorPane | null, line?: number, character?: number) {
  if (!pane) return;
  pane.goalsBusy = true;
  try {
    pane.goals = await api.leanGoals(pane.nodeId, line, character);
  } catch (error) {
    pane.goals = {
      goals: [],
      rendered: "",
      note: error instanceof Error ? error.message : String(error),
    };
  } finally {
    pane.goalsBusy = false;
  }
}

async function loadSettings() {
  const tree = await run("settings", () => api.settingsView());
  if (tree) state.settings = tree;
}

async function loadLogic() {
  const logic = await run("logic", () => api.logicGraph(state.openTheory || null));
  if (!logic) return;
  const view = overlay(logic);
  // A theory that no longer exists drops the board back to the root.
  state.openTheory = view.trail.length > 1 ? (view.trail[view.trail.length - 1]?.key ?? "") : "";
  state.logic = view;
}

async function loadRecents() {
  try {
    state.recents = await api.recentProjects();
  } catch {
    state.recents = [];
  }
}

/** True when `AI → Enable AI Features` is switched on. */
export function aiEnabled(): boolean {
  const walk = (nodes: api.SettingNode[]): string | null => {
    for (const node of nodes) {
      if (node.id === "ai.enabled") return node.value;
      const inner = walk(node.children);
      if (inner !== null) return inner;
    }
    return null;
  };
  return walk(state.settings) === "true";
}

/** Writes are batched: `.klean.json` is rewritten at most every 5 seconds. */
const FLUSH_MS = 5000;
const pending: {
  positions: Record<string, api.FixedNode[]>;
  camera: Record<string, api.Camera>;
} = { positions: {}, camera: {} };
let flushTimer: number | null = null;
/** Set while Lean is being polled after an edit or a project switch. */
let pollTimer: number | null = null;
/** Guards against an older hover response overwriting a newer one. */
let hoverSeq = 0;
let flushAgain = false;

function mergePositions(
  current: api.FixedNode[] | undefined,
  next: api.FixedNode[],
): api.FixedNode[] {
  const merged = new Map((current ?? []).map((entry) => [entry.id, entry]));
  for (const entry of next) merged.set(entry.id, entry);
  return [...merged.values()];
}

/** Mirrors drag results into the loaded graph so a redraw keeps them. */
/**
 * Re-reads both boards. Pages never keep their own copy of a board, so the AST
 * and the logic view can never disagree after an edit.
 */
async function reloadBoards() {
  const [graph, logic] = await Promise.all([
    api.graphView(),
    api.logicGraph(state.openTheory || null),
  ]);
  state.graph = graph;
  state.logic = overlay(logic);
}

/**
 * Re-reads the board that is on screen.
 *
 * Both boards redraw the whole picture, which is the expensive part of a save, and
 * the one behind the modal is not being looked at: a save pays for its own board
 * only.
 */
async function reloadBoard() {
  if (state.boardView === "logic") {
    const logic = await api.logicGraph(state.openTheory || null).catch(() => null);
    if (logic) state.logic = overlay(logic);
    return;
  }
  const graph = await api.graphView().catch(() => null);
  if (graph) state.graph = graph;
}

/**
 * The logic graph as the user sees it: positions dragged less than a flush ago
 * only live in this tab, so a fresh graph from Rust would otherwise snap nodes
 * back to their last saved spot.
 */
function overlay(graph: api.LogicGraph): api.LogicGraph {
  const entries = pending.positions[graph.graph];
  if (!entries || entries.length === 0) return graph;
  const byKey = new Map(entries.map((entry) => [entry.id, entry]));
  return {
    ...graph,
    nodes: graph.nodes.map((node) => {
      const moved = byKey.get(node.key);
      return moved ? { ...node, x: moved.x, y: moved.y } : node;
    }),
  };
}

/** Graph key the board's positions are stored under. */
function graphKey(board: "ast" | "logic"): string {
  if (board === "ast") return "ast";
  return state.logic?.graph ?? "logic";
}

function applyPositions(board: "ast" | "logic", positions: api.FixedNode[]) {
  const byKey = new Map(positions.map((entry) => [entry.id, entry]));
  const update = (nodes: api.GraphNode[]) =>
    nodes.map((node) => {
      const moved = byKey.get(node.key);
      return moved ? { ...node, x: moved.x, y: moved.y } : node;
    });
  if (board === "ast" && state.graph) {
    state.graph = { ...state.graph, nodes: update(state.graph.nodes) };
  }
  if (board === "logic" && state.logic) {
    state.logic = { ...state.logic, nodes: update(state.logic.nodes) };
  }
}

function scheduleFlush() {
  if (flushTimer !== null) return;
  flushTimer = window.setTimeout(() => {
    flushTimer = null;
    void flushPending();
  }, FLUSH_MS);
}

async function flushPending() {
  if (flushTimer !== null) {
    // A write is already scheduled; it will pick these up.
    flushAgain = true;
    return;
  }
  const positions = pending.positions;
  const camera = pending.camera;
  pending.positions = {};
  pending.camera = {};

  for (const [graph, entries] of Object.entries(positions)) {
    if (entries.length > 0) await run("layout", () => api.savePositions(graph, entries));
  }
  for (const [graph, value] of Object.entries(camera)) {
    await run("layout", () => api.saveCamera(graph, value));
  }
  if (flushAgain) {
    flushAgain = false;
    scheduleFlush();
  }
}

if (typeof window !== "undefined") {
  // Do not lose the last few seconds of dragging when the window goes away.
  window.addEventListener("beforeunload", () => {
    void flushPending();
  });
}

export const actions = {
  /**
   * Leaves the splash screen.
   *
   * The splash has already waited for Lean's parser and for the editor's code to
   * arrive; what is left is the state a page renders from — recents, settings, and
   * a project handed on the command line.
   */
  async finishStartup() {
    state.page = "welcome";
    await actions.bootstrap();
  },

  /** Adopts a project Rust already has open (e.g. given on the command line). */
  async bootstrap() {
    // The welcome page does no Lean work at all: it lists recents and stops.
    await loadRecents();
    try {
      // Only a project given on the command line is open at this point.
      const summary = await api.projectSummary();
      if (!summary) return;
      // It opens like any other project: the files are read first, with the modal
      // reporting them, and the board follows.
      await beginParse(summary);
    } catch {
      state.page = "welcome";
    }
  },

  async loadRecents() {
    await loadRecents();
  },

  async openFolder(path: string) {
    await flushPending();
    const summary = await run("open", () => api.openProject(path));
    if (!summary) return;
    await loadRecents();
    // A folder project is read before its board exists: the modal reports the read,
    // and closing it closes the project again.
    await beginParse(summary);
  },

  async openZip(path: string) {
    await flushPending();
    const summary = await run("open", () => api.openProjectZip(path));
    if (!summary) return;
    await beginParse(summary);
  },

  async openRecent(entry: api.RecentProject) {
    if (entry.kind === "zip") await actions.openZip(entry.path);
    else await actions.openFolder(entry.path);
  },

  async importZip(data: string, label?: string) {
    await flushPending();
    const summary = await run("open", () => api.importProjectZip(data, label));
    if (!summary) return;
    await beginParse(summary);
  },

  async newMemory(label?: string) {
    await flushPending();
    const summary = await run("open", () => api.newMemoryProject(label));
    if (!summary) return;
    await beginParse(summary);
  },

  /**
   * Stops reading and puts the project away.
   *
   * The parse modal's close button does this: stopping the read leaves no board to
   * show, so the project is closed rather than opened half-read.
   */
  async cancelParse() {
    if (!state.parseOpen) return;
    parseStopped = true;
    if (parseTimer !== null) {
      window.clearTimeout(parseTimer);
      parseTimer = null;
    }
    state.parseOpen = false;
    state.parseBlocked = null;
    const label = state.parseLabel;
    state.parseLabel = "";
    state.scan = null;
    await api.leanScanCancel().catch(() => undefined);
    await api.closeProject().catch(() => undefined);
    state.page = "welcome";
    state.view = null;
    state.graph = null;
    state.logic = null;
    setSelection([]);
    state.editors = [];
    state.activeEditor = null;
    state.notice = `stopped reading ${label || "the project"}: nothing was opened`;
    await loadRecents();
  },

  /**
   * Opens the project anyway, when its files cannot be read at all (Lean switched
   * off, no toolchain). The board then shows the file tree with no declarations.
   */
  async parseAnyway() {
    state.parseBlocked = null;
    await finishParse();
  },

  /**
   * Loads the details behind a hovered graph node. Failures stay quiet: the
   * card simply shows what the graph already knows.
   */
  async loadHover(id: string | null) {
    if (!id) {
      hoverSeq += 1;
      state.hover = null;
      return;
    }
    if (state.hover?.id === id) return;
    const token = (hoverSeq += 1);
    try {
      const detail = await api.nodeDetail(id);
      if (token === hoverSeq) state.hover = detail;
    } catch {
      if (token === hoverSeq) state.hover = null;
    }
  },

  /** Opens or closes the settings modal. */
  setSettingsOpen(open: boolean) {
    state.settingsOpen = open;
  },

  /**
   * The one source of truth: both boards and the workspace are re-read from the
   * VFS together. Every page renders from `state`, so nothing can go stale.
   */
  async refresh() {
    const view = await run("refresh", () => api.workspaceView());
    if (!view) return;
    state.view = view;
    await Promise.all([reloadBoards(), loadSettings()]);
  },

  async close() {
    await flushPending();
    await run("close", () => api.closeProject());
    state.page = "welcome";
    state.view = null;
    state.graph = null;
    state.logic = null;
    setSelection([]);
    state.editors = [];
    state.activeEditor = null;
    state.printed = null;
    state.printedOpen = false;
    state.lean = null;
    state.install = null;
    state.processing = false;
    state.processingFiles = [];
    await loadRecents();
  },

  /** Switches the board between the AST tree view and the logic graph. */
  async setBoardView(view: BoardView) {
    if (state.boardView === view) return;
    state.boardView = view;
    if (view === "logic" && state.logic === null) await loadLogic();
  },

  /**
   * Replaces the selection. The pane the user last touched follows a single
   * selection, the way one editor always did; the others keep what they show.
   */
  async select(id: string, options: { focus?: boolean } = {}) {
    setSelection([id]);
    const pane = activePane();
    // A pane holding an unsaved edit is not retargeted: the edit is the user's, and
    // it goes only when they save it, discard it or close the pane.
    if (pane && !pane.dirty && DETAILABLE.test(id) && pane.nodeId !== id) {
      pane.nodeId = id;
      await loadPane(pane);
      void loadGoals(pane);
      void loadTerm(pane);
    }
    if (options.focus) {
      const graph = await run("focus", () => api.focusGraphNode(id));
      if (graph) state.graph = graph;
    }
  },

  /**
   * Replaces the selection with several items (rubber band, Ctrl-click).
   *
   * It does not retarget an open pane: with the modifier held the user is either
   * building a selection or asking for another editor on the double-click that
   * follows, and neither should disturb the pane they were reading.
   */
  async selectMany(ids: string[]) {
    setSelection(ids);
  },

  /** Ctrl/Cmd-click: adds or removes one item. */
  async toggleSelected(id: string) {
    const next = state.selectedIds.includes(id)
      ? state.selectedIds.filter((current) => current !== id)
      : [...state.selectedIds, id];
    await actions.selectMany(next);
  },

  /**
   * Opens a node in an editor.
   *
   * Three rules, in order:
   *
   * 1. a node already open is raised, never opened twice;
   * 2. `fresh` (Ctrl + double-click) opens another modal — unless the same node is
   *    already in one, which rule 1 has answered;
   * 3. a plain double-click replaces the modal that shows the *same file* — a file
   *    modal is replaced by a node of it, and a node modal by the file — and
   *    otherwise the one the user last touched. A modal holding an unsaved edit is
   *    never retargeted: the edit is the user's, and a new modal opens instead.
   */
  async openNode(id: string, options: { fresh?: boolean } = {}) {
    setSelection([id]);
    if (!DETAILABLE.test(id)) return;

    const same = state.editors.find((pane) => pane.nodeId === id);
    if (same) {
      raisePane(same);
      return;
    }

    const target = options.fresh ? null : relatedPane(id);
    if (target?.dirty) {
      // The modal the double-click would reload holds an unsaved edit: it asks
      // first, and stays where it is until the answer comes.
      target.pending = id;
      target.confirming = true;
      raisePane(target);
      return;
    }
    const open = target ?? null;
    const pane = open ?? createPane(id);
    if (open) {
      open.nodeId = id;
      raisePane(open);
    }
    await loadPane(pane);
    void loadGoals(pane);
    void loadTerm(pane);
    // Only the file the user opened is checked, never the whole project.
    void api.leanCheckFile(id).then(() => pollProcessing()).catch(() => {});
  },

  /** Brings a pane to the front: it is now the one a double-click retargets. */
  focusPane(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (pane && state.activeEditor !== paneId) raisePane(pane);
  },

  /** Closes one pane, asking first when it holds an unsaved edit. */
  requestClosePane(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    if (pane.dirty) {
      pane.confirming = true;
      return;
    }
    void actions.closePane(paneId);
  },

  /** "keep editing": the close question goes away and the pane stays. */
  keepEditing(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (pane) pane.confirming = false;
  },

  /**
   * Throws the pane's unsaved edit away.
   *
   * When the warning came from a double-click it continues to the node that was
   * asked for; when it came from the close button, it closes the pane.
   */
  async discardAndClose(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    const pending = pane.pending;
    pane.confirming = false;
    pane.pending = null;
    await actions.discardPane(paneId);
    if (pending) {
      pane.nodeId = pending;
      await loadPane(pane);
      void loadGoals(pane);
      void loadTerm(pane);
      return;
    }
    await actions.closePane(paneId);
  },

  async closePane(paneId: string) {
    const index = state.editors.findIndex((pane) => pane.id === paneId);
    if (index < 0) return;
    const closing = state.editors[index];
    if (closing.geometry) state.lastGeometry = closing.geometry;
    state.editors.splice(index, 1);
    if (state.activeEditor === paneId) {
      state.activeEditor = state.editors.at(-1)?.id ?? null;
    }
    // The file's memory is only held while an editor of that file is open.
    const rel = closing.detail?.path;
    const shared = rel ? state.editors.some((pane) => pane.detail?.path === rel) : false;
    if (rel && !shared) await api.discardDraft(closing.nodeId).catch(() => undefined);
    // The last editor is gone: the board can catch up with what was saved.
    if (state.editors.length === 0) await refreshAfterClose();
  },

  /**
   * Marks a pane dirty or clean; the editor component reports it.
   *
   * A pane that stops being dirty has nothing left to discard, so the close
   * question goes away with the edit — saving while it is showing closes it.
   */
  setPaneDirty(paneId: string, dirty: boolean) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    pane.dirty = dirty;
    // Typing while the warning is up is an answer in itself: the user is keeping
    // the edit, so the question goes away.
    if (dirty) {
      pane.confirming = false;
      pane.pending = null;
    } else {
      pane.confirming = false;
    }
  },

  /** Remembers where a modal sits, so the next one can step out from it. */
  setPaneGeometry(paneId: string, geometry: PaneGeometry) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (pane) pane.geometry = geometry;
  },

  /** Several carets: the term under them is not one term, so nothing is asked. */
  setPaneSelections(paneId: string, count: number) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    const multi = count > 1;
    if (pane.multi === multi) return;
    pane.multi = multi;
    if (multi) pane.term = null;
  },

  /** Re-reads Lean's state for this project (toolchain, server, build). */
  async refreshLean() {
    await loadLean();
    await pollBuild();
  },

  /** Starts `lake build`, and opens the console that follows it. */
  async buildLean() {
    state.buildOpen = true;
    const build = await run("build", () => api.leanBuild());
    if (!build) return;
    state.build = build;
    void pollBuild();
  },

  /** Opens the console for the last build, without starting a new one. */
  async showBuild() {
    state.buildOpen = true;
    await pollBuild();
  },

  setBuildOpen(open: boolean) {
    state.buildOpen = open;
  },

  /** Remembers whether to ask before building (`ask` / `always` / `never`). */
  async setBuildPrompt(value: "ask" | "always" | "never") {
    const tree = await api
      .saveSettings([["lean.build.prompt", value]])
      .catch(() => null);
    if (tree) state.settings = tree;
  },

  /** Stops the background read; what was read stays. */
  /** Stops a running build. */
  async cancelBuild() {
    const build = await run("build", () => api.leanBuildCancel()).catch(() => null);
    if (build) state.build = build;
  },

  /** Goals at a position in a pane's node. */
  async loadPaneGoals(paneId: string, line?: number, character?: number) {
    await loadGoals(state.editors.find((pane) => pane.id === paneId) ?? null, line, character);
  },

  /** The term at a position in a pane's node. */
  async loadPaneTerm(paneId: string, line?: number, character?: number) {
    await loadTerm(state.editors.find((pane) => pane.id === paneId) ?? null, line, character);
  },

  /** Installs Lean through the app, reporting progress as it goes. */
  async installLean() {
    state.install = "starting…";
    const path = await run("lean", () => api.leanInstall());
    state.install = null;
    if (!path) return;
    state.notice = `Lean installed at ${path}`;
    await loadSettings();
    await loadLean();
    await actions.refresh();
  },

  /** Subscribes to the background work's progress; called once at startup. */
  async listen() {
    await listen<string>("lean-install", (event) => {
      state.install = event.payload;
    });
    await listen<string>("lean-build-output", (event) => {
      if (!state.build) {
        state.build = {
          running: true,
          command: "lake build",
          lines: [],
          summary: null,
          failed: false,
          seconds: 0,
        };
      }
      state.build.lines = [...state.build.lines, event.payload].slice(-2000);
    });
  },

  /** Moves terms (or namespaces) into a theory, or back to unassigned. */
  async assignTheory(ids: string[], theory: string | null) {
    if (ids.length === 0) return;
    await flushPending();
    const logic = await run("logic", () =>
      api.assignTheory(state.openTheory || null, ids, theory),
    );
    if (logic) state.logic = overlay(logic);
  },

  /** Opens a theory graph (`""` for the root) and remembers the framing. */
  async openTheory(key: string) {
    if (state.openTheory === key) return;
    await flushPending();
    state.openTheory = key;
    await loadLogic();
  },

  /** Creates a theory in the open graph (or the root one). */
  async createTheory(name: string) {
    await flushPending();
    const logic = await run("logic", () =>
      api.createTheory(state.openTheory || null, name),
    );
    if (logic) state.logic = overlay(logic);
  },

  /** Renames a theory node; its key (and so its place) is kept. */
  async renameTheory(key: string, name: string) {
    await flushPending();
    const logic = await run("logic", () =>
      api.renameTheory(state.openTheory || null, key, name),
    );
    if (logic) state.logic = overlay(logic);
  },

  /** Adds an axiom to a file, or to a new one at `path`, and shows it. */
  async addAxiom(name: string, file: string | null, path: string | null) {
    await flushPending();
    const logic = await run("logic", () =>
      api.addAxiom(state.openTheory || null, name, file, path),
    );
    if (!logic) return false;
    state.logic = overlay(logic);
    // The axiom is now a term of the project: refresh the tree and the AST board.
    await reloadBoards();
    state.notice = `added axiom ${name}`;
    return true;
  },

  /** Deletes a theory; its terms return to the unassigned pile. */
  async deleteTheory(key: string) {
    await flushPending();
    if (state.openTheory === key) state.openTheory = "";
    const logic = await run("logic", () =>
      api.deleteTheory(state.openTheory || null, key),
    );
    if (logic) {
      const view = overlay(logic);
      state.openTheory =
        view.trail.length > 1 ? (view.trail[view.trail.length - 1]?.key ?? "") : "";
      state.logic = view;
    }
  },

  /** Adds a layer divider to the open theory graph. */
  async createLayer(name: string, y: number) {
    if (!state.openTheory) return;
    await flushPending();
    const logic = await run("logic", () => api.createLayer(state.openTheory, name, y));
    if (logic) state.logic = overlay(logic);
  },

  async deleteLayer(id: string) {
    if (!state.openTheory) return;
    await flushPending();
    const logic = await run("logic", () => api.deleteLayer(state.openTheory, id));
    if (logic) state.logic = overlay(logic);
  },

  async moveLayer(id: string, y: number) {
    if (!state.openTheory) return;
    // The board may hold drags that have not been written yet; they must not be
    // lost (or undone) by the graph this returns.
    await flushPending();
    const logic = await run("logic", () => api.moveLayer(state.openTheory, id, y));
    if (logic) state.logic = overlay(logic);
  },

  /** Renames a layer divider of the open theory graph. */
  async renameLayer(id: string, name: string) {
    if (!state.openTheory) return;
    await flushPending();
    const logic = await run("logic", () => api.renameLayer(state.openTheory, id, name));
    if (logic) state.logic = overlay(logic);
  },

  /** Applies drag results locally and queues a write to `.klean.json`. */
  savePositions(board: "ast" | "logic", positions: api.FixedNode[]) {
    applyPositions(board, positions);
    const key = graphKey(board);
    pending.positions[key] = mergePositions(pending.positions[key], positions);
    scheduleFlush();
  },

  /** Remembers a graph camera; written with the same flush as positions. */
  saveCamera(board: "ast" | "logic", camera: api.Camera) {
    if (board === "ast" && state.graph) state.graph = { ...state.graph, camera };
    if (board === "logic" && state.logic) state.logic = { ...state.logic, camera };
    pending.camera[graphKey(board)] = camera;
    scheduleFlush();
  },

  /** Remembers where a link's middle segment was dragged to. */
  async saveLinkBend(board: "ast" | "logic", key: string, value: number | null) {
    const target = board === "ast" ? state.graph : state.logic;
    if (target) {
      const links = target.links.map((link) =>
        `${link.source}->${link.target}` === key ? { ...link, bend: value } : link,
      );
      if (board === "ast" && state.graph) state.graph = { ...state.graph, links };
      if (board === "logic" && state.logic) state.logic = { ...state.logic, links };
    }
    const result = await run("bend", () => api.saveLinkBend(graphKey(board), key, value));
    void result;
  },

  /** Loads the settings tree. */
  async loadSettings() {
    const tree = await run("settings", () => api.settingsView());
    if (tree) state.settings = tree;
  },

  /** Applies edited settings and re-reads the boards. */
  async saveSettings(values: [string, string][]) {
    const tree = await run("settings", () => api.saveSettings(values));
    if (!tree) return false;
    state.settings = tree;
    await actions.refresh();
    return true;
  },

  /** Saves the plain comments above a pane's node. */
  async applyComments(paneId: string, text: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    pane.saving = true;
    pane.error = null;
    const rel = pane.detail?.path;
    try {
      const result = await api.updateComments(pane.nodeId, text);
      pane.detail = result.detail;
      if (state.view) {
        state.view.tree = result.tree;
        state.view.stats = result.stats;
      }
      // Only the board on screen is redrawn: the other one is behind the modal.
      await reloadBoard();
      await refreshFileEditors(rel, pane);
      void pollProcessing();
    } catch (error) {
      pane.error = error instanceof Error ? error.message : String(error);
    } finally {
      pane.saving = false;
    }
  },

  async toggleGraph(id: string) {
    const graph = await run("graph", () => api.toggleGraphNode(id));
    if (graph) state.graph = graph;
  },

  /**
   * Checks the editor's text without saving it.
   *
   * The text goes to Rust, which splices it into the file in memory and hands it to
   * Lean straight away; the answers (diagnostics, processing) come back as the
   * detail the editor already shows. Nothing is written to disk.
   */
  async previewText(paneId: string, text: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    // A node renamed or replaced under the pane cannot take the text any more.
    if (!pane || pane.detail?.id !== pane.nodeId) return;
    pane.error = null;
    try {
      const result = await api.previewNode(pane.nodeId, text);
      // Only the node: the board's tree is about saved work, and rebuilding it on
      // every keystroke is exactly what made checking feel slow.
      if (pane.detail?.id === pane.nodeId) pane.detail = result.detail;
      // The node's own proof state follows the text too: the icon beside the
      // comments should not wait for a save to turn green.
      void loadNodeGoals(pane);
      // The server publishes a moment later: follow it, so the diagnostics under
      // the caret are Lean's, not the previous ones.
      void pollProcessing();
    } catch (error) {
      pane.error = error instanceof Error ? error.message : String(error);
    }
  },

  /** Throws a pane's unsaved edit away and re-checks what is on disk. */
  async discardPane(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    pane.dirty = false;
    const rel = pane.detail?.path;
    try {
      await api.discardDraft(pane.nodeId);
      await loadPane(pane);
      // The file is back to what is on disk, for every editor showing it.
      await refreshFileEditors(rel, pane);
    } catch (error) {
      pane.error = error instanceof Error ? error.message : String(error);
    }
  },

  /** Writes a pane's text to disk. The only path that saves. */
  async applyText(paneId: string, text: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    if (!pane) return;
    pane.saving = true;
    pane.error = null;
    const rel = pane.detail?.path;
    try {
      const result = await api.updateNode(pane.nodeId, text);
      pane.detail = result.detail;
      pane.dirty = false;
      pane.confirming = false;
      if (state.view) {
        state.view.tree = result.tree;
        state.view.stats = result.stats;
      }
      // Only the board on screen is redrawn: the other one is behind the modal.
      await reloadBoard();
      await refreshFileEditors(rel, pane);
      void loadGoals(pane);
      void loadNodeGoals(pane);
      void pollProcessing();
    } catch (error) {
      pane.error = error instanceof Error ? error.message : String(error);
    } finally {
      pane.saving = false;
    }
  },

  /** Creates a Lean file relative to the project root. */
  async createFile(path: string) {
    const view = await run("create", () => api.createFile(path));
    if (!view) return;
    state.view = view;
    await reloadBoards();
    state.notice = `created ${path}`;
  },

  async deleteFile(path: string) {
    const view = await run("delete", () => api.deleteFile(path));
    if (!view) return;
    state.view = view;
    const gone = state.editors.filter((pane) => pane.detail?.path === path);
    for (const pane of gone) await actions.closePane(pane.id);
    if (gone.length > 0) setSelection([]);
    await reloadBoards();
    state.notice = `deleted ${path}`;
  },

  async showOutline() {
    const outline = await run("outline", () => api.exportOutline());
    if (outline !== null) {
      state.printed = {
        title: "Lean outline",
        subtitle: "whole project · printed by Rust",
        text: outline,
      };
      state.printedOpen = true;
    }
  },

  /** Removes an axiom (or any declaration) from the file that holds it. */
  async deleteNode(id: string) {
    await flushPending();
    // The command answers with nothing, so success is "no error was reported".
    await run("delete", () => api.deleteNode(id));
    if (state.error) return false;
    // The term is gone from the project: re-read the boards and the tree, so it
    // leaves the graph immediately instead of on the next refresh.
    await reloadBoards();
    const view = await run("view", () => api.workspaceView());
    if (view) state.view = view;
    for (const pane of state.editors.filter((current) => current.nodeId === id)) {
      await actions.closePane(pane.id);
    }
    if (state.selectedIds.includes(id)) {
      setSelection(state.selectedIds.filter((current) => current !== id));
    }
    state.hover = null;
    state.notice = "declaration removed";
    return true;
  },

  /**
   * Opens the file a pane's node lives in, as an editor of its own.
   *
   * It is a normal editable document: a file contains its nodes, so editing it here
   * is editing them, and a save reaches the disk the same way.
   */
  async openFilePane(paneId: string) {
    const pane = state.editors.find((current) => current.id === paneId);
    const rel = pane?.detail?.path;
    if (!rel) return;
    await actions.openNode(`file:${rel}`, { fresh: true });
  },

  closePrinted() {
    state.printedOpen = false;
  },

  async exportZip(path: string) {
    const bytes = await run("export", () => api.exportZipTo(path));
    if (bytes !== null) state.notice = `wrote ${(bytes / 1024).toFixed(1)} KiB zip`;
  },

  async saveOutline(path: string) {
    const text = await run("export", () => api.exportOutlineTo(path));
    if (text !== null) state.notice = `wrote ${text.split("\n").length} lines of Lean`;
  },

  dismissNotice() {
    state.notice = null;
  },

  dismissError() {
    state.error = null;
  },
};
