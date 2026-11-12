// Reactive application state.
//
// This is a thin shell: every value here comes from Rust, every mutation is a
// Rust command. The only thing the front-end owns is which page is visible,
// whether the editor modal is open, and which board view is selected.

import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api/lean";

export type Page = "welcome" | "project";
export type BoardView = "ast" | "logic";

export interface Printed {
  title: string;
  subtitle: string;
  text: string;
}

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
  detail: api.NodeDetail | null;
  editorOpen: boolean;
  saving: boolean;
  saveError: string | null;
  printed: Printed | null;
  printedOpen: boolean;
  settings: api.SettingNode[];
  settingsOpen: boolean;
  /** Details of the node under the pointer, for the graph's info card. */
  hover: api.NodeDetail | null;
  /** Lean's toolchain and build state for this project. */
  lean: api.LeanStatus | null;
  /** Goals of the node open in the editor, from Lean's language server. */
  goals: api.Goals | null;
  goalsBusy: boolean;
  /** The type of the term at the caret, as Lean renders it. */
  term: string | null;
  termBusy: boolean;
  /** Last line the toolchain installer printed. */
  install: string | null;
  /** The `lake build` console (empty until a build is started). */
  build: api.BuildState | null;
  /** True while the build console is open. */
  buildOpen: boolean;
  /** True when the editor holds text Lean has checked but the disk has not seen. */
  editorDirty: boolean;
  /** How the background read of the project stands (null before one starts). */
  scan: api.ScanState | null;
  /** True while Lean is still processing at least one file. */
  processing: boolean;
  /** Files being processed, for the modal's indicator. */
  processingFiles: string[];
}

export const state = reactive<State>({
  page: "welcome",
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
  detail: null,
  editorOpen: false,
  saving: false,
  saveError: null,
  printed: null,
  printedOpen: false,
  settings: [],
  settingsOpen: false,
  hover: null,
  lean: null,
  goals: null,
  goalsBusy: false,
  term: null,
  termBusy: false,
  install: null,
  build: null,
  buildOpen: false,
  editorDirty: false,
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

async function loadDetail(id: string) {
  const detail = await run("detail", () => api.nodeDetail(id));
  if (detail) state.detail = detail;
}

function adopt(view: api.WorkspaceView) {
  state.view = view;
  state.boardView = "ast";
  setSelection([]);
  state.detail = null;
  state.editorOpen = false;
  state.saveError = null;
  state.printed = null;
  state.printedOpen = false;
  state.logic = null;
  state.openTheory = "";
  state.page = "project";
  state.lean = null;
  state.goals = null;
  state.processing = false;
  state.processingFiles = [];
  state.scan = null;
  // The board is on screen already (a folder project opens from its file listing).
  // What is left — reading the files — happens in the background, where the console
  // can show it; checking and building stay the user's call.
  if (view.pending) void startScan();
  void (async () => {
    await loadSettings();
    await loadLean();
    await pollBuild();
  })();
}

async function loadGraph() {
  await waitForScan();
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

let scanTimer: number | null = null;
/** Set when the user stops the read: its end must not reload the boards. */
let scanStopped = false;

/**
 * Reads the open project's files in the background.
 *
 * Opening a project shows the file listing at once; the declarations arrive
 * afterwards. While that runs, the console reports it file by file, and the board
 * is refreshed as soon as it is done.
 */
async function startScan() {
  if (scanTimer !== null) {
    window.clearTimeout(scanTimer);
    scanTimer = null;
  }
  scanStopped = false;
  const scan = await api.leanScan().catch(() => null);
  if (!scan) return;
  state.scan = scan;
  watchScan();
}

/** Polls the scan, and reloads the board once the files are read. */
function watchScan() {
  if (scanTimer !== null) return;
  const tick = async () => {
    const scan = await api.leanScanState().catch(() => null);
    if (!scan) {
      scanTimer = null;
      return;
    }
    state.scan = scan;
    if (scan.running) {
      scanTimer = window.setTimeout(tick, 300);
      return;
    }
    scanTimer = null;
    // Stopped by the user: the boards keep what they have rather than pulling in
    // the files that were just refused.
    if (scanStopped) return;
    await refreshAfterScan();
  };
  scanTimer = window.setTimeout(tick, 300);
}

/** Fills the tree in once the read is over. */
async function refreshAfterScan() {
  const view = await api.workspaceView().catch(() => null);
  if (view) state.view = view;
  await reloadBoards();
  const id = state.detail?.id;
  if (id && state.editorOpen) {
    const detail = await api.nodeDetail(id).catch(() => null);
    if (detail) state.detail = detail;
  }
}

/**
 * Waits for the background read, for the boards that need the files: the graph
 * and the logic tree are built from declarations, not from file names.
 */
async function waitForScan(): Promise<void> {
  const deadline = Date.now() + 120000;
  while (state.scan?.running && Date.now() < deadline) {
    // Asked of Rust, not of the poller: the boards must not depend on it.
    const scan = await api.leanScanState().catch(() => null);
    if (!scan || !scan.running) return;
    state.scan = scan;
    await new Promise((resolve) => window.setTimeout(resolve, 200));
  }
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
    const editing = state.editorOpen && state.editorDirty;
    if (!editing) {
      const view = await api.workspaceView().catch(() => null);
      if (view) state.view = view;
      await reloadBoards();
    }
    const id = state.detail?.id;
    if (id && state.editorOpen) {
      const detail = await api.nodeDetail(id).catch(() => null);
      if (detail) state.detail = detail;
    }
  };
  pollTimer = window.setTimeout(tick, 120);
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
/** Asks Lean what the term under the caret is. */
async function loadTerm(line?: number, character?: number) {
  const id = state.detail?.id;
  if (!id) {
    state.term = null;
    return;
  }
  state.termBusy = true;
  try {
    state.term = await api.leanHover(id, line, character);
  } catch {
    state.term = null;
  } finally {
    state.termBusy = false;
  }
}

async function loadGoals(line?: number, character?: number) {
  const id = state.detail?.id;
  if (!id) {
    state.goals = null;
    return;
  }
  state.goalsBusy = true;
  try {
    state.goals = await api.leanGoals(id, line, character);
  } catch (error) {
    state.goals = {
      goals: [],
      rendered: "",
      note: error instanceof Error ? error.message : String(error),
    };
  } finally {
    state.goalsBusy = false;
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
  await waitForScan();
  const [graph, logic] = await Promise.all([
    api.graphView(),
    api.logicGraph(state.openTheory || null),
  ]);
  state.graph = graph;
  state.logic = overlay(logic);
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
  /** Adopts a project Rust already has open (e.g. given on the command line). */
  async bootstrap() {
    // The welcome page does no Lean work at all: it lists recents and stops.
    await loadRecents();
    try {
      // Only a project given on the command line is open at this point.
      const view = await api.workspaceView();
      state.view = view;
      state.page = "project";
      // A project given on the command line opens the same way as any other: the
      // listing first, the files in the background.
      if (view.pending) void startScan();
      // Settings and Lean's status come from the project, not from its files, so
      // they do not wait for the read; the boards do.
      await Promise.all([loadSettings(), loadLean()]);
      void pollBuild();
      // A board that starts on the logic view needs its graph loaded too.
      if (state.boardView === "logic") void loadLogic();
      void loadGraph();
    } catch {
      state.page = "welcome";
    }
  },

  async loadRecents() {
    await loadRecents();
  },

  async openFolder(path: string) {
    await flushPending();
    const view = await run("open", () => api.openProject(path));
    if (!view) return;
    adopt(view);
    await loadRecents();
    // A folder project fills itself in from the background read; an archive or an
    // in-memory workspace is complete the moment it opens.
    if (!view.pending) await loadGraph();
  },

  async openZip(path: string) {
    await flushPending();
    const view = await run("open", () => api.openProjectZip(path));
    if (!view) return;
    adopt(view);
    await Promise.all([loadGraph(), loadRecents()]);
  },

  async openRecent(entry: api.RecentProject) {
    if (entry.kind === "zip") await actions.openZip(entry.path);
    else await actions.openFolder(entry.path);
  },

  async importZip(data: string, label?: string) {
    await flushPending();
    const view = await run("open", () => api.importProjectZip(data, label));
    if (!view) return;
    adopt(view);
    await loadGraph();
  },

  async newMemory(label?: string) {
    await flushPending();
    const view = await run("open", () => api.newMemoryProject(label));
    if (!view) return;
    adopt(view);
    await loadGraph();
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
    state.detail = null;
    state.editorOpen = false;
    state.printed = null;
    state.printedOpen = false;
    state.lean = null;
    state.goals = null;
    state.install = null;
    state.processing = false;
    state.processingFiles = [];
    await loadRecents();
  },

  /** Switches the board between the AST tree view and the logic graph. */
  async setBoardView(view: BoardView) {
    if (state.boardView === view) return;
    state.boardView = view;
    state.editorOpen = false;
    if (view === "logic" && state.logic === null) await loadLogic();
  },

  /** Selects a node in every panel and (optionally) opens the editor. */
  /** Replaces the selection; a single item updates an open modal. */
  async select(id: string, options: { focus?: boolean } = {}) {
    setSelection([id]);
    state.saveError = null;
    // A modal that is open follows a single selection; with several selected
    // items it keeps showing what it was opened with.
    if (state.editorOpen && DETAILABLE.test(id)) await loadDetail(id);
    if (options.focus) {
      const graph = await run("focus", () => api.focusGraphNode(id));
      if (graph) state.graph = graph;
    }
  },

  /** Replaces the selection with several items (rubber band, group click). */
  async selectMany(ids: string[]) {
    setSelection(ids);
    state.saveError = null;
    if (state.editorOpen && ids.length === 1 && DETAILABLE.test(ids[0])) await loadDetail(ids[0]);
  },

  /** Ctrl/Cmd-click: adds or removes one item. */
  async toggleSelected(id: string) {
    const next = state.selectedIds.includes(id)
      ? state.selectedIds.filter((current) => current !== id)
      : [...state.selectedIds, id];
    await actions.selectMany(next);
  },

  /** Double click: opens the editor for one item. */
  async openNode(id: string) {
    setSelection([id]);
    state.saveError = null;
    if (!DETAILABLE.test(id)) return;
    state.editorOpen = true;
    state.goals = null;
    state.term = null;
    await loadDetail(id);
    void loadGoals();
    void loadTerm();
    // Only the file the user opened is checked, never the whole project.
    void api.leanCheckFile(id).then(() => pollProcessing()).catch(() => {});
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
  async cancelScan() {
    scanStopped = true;
    await api.leanScanCancel().catch(() => undefined);
    const scan = await api.leanScanState().catch(() => null);
    if (scan) state.scan = scan;
  },

  /** Stops a running build. */
  async cancelBuild() {
    const build = await run("build", () => api.leanBuildCancel()).catch(() => null);
    if (build) state.build = build;
  },

  /** Goals at a position in the open node. */
  async loadGoals(line?: number, character?: number) {
    await loadGoals(line, character);
  },

  /** The term at a position in the open node. */
  async loadTerm(line?: number, character?: number) {
    await loadTerm(line, character);
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

  /** Saves the plain comments above a node. */
  async applyComments(text: string) {
    const id = state.detail?.id;
    if (!id) return;
    state.saving = true;
    state.saveError = null;
    try {
      const result = await api.updateComments(id, text);
      state.detail = result.detail;
      if (state.view) {
        state.view.tree = result.tree;
        state.view.stats = result.stats;
      }
      await reloadBoards();
      void pollProcessing();
    } catch (error) {
      state.saveError = error instanceof Error ? error.message : String(error);
    } finally {
      state.saving = false;
    }
  },

  closeEditor() {
    state.editorOpen = false;
    state.saveError = null;
    state.goals = null;
    state.term = null;
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
  async previewText(text: string) {
    const id = state.detail?.id;
    if (!id) return;
    state.saveError = null;
    try {
      const result = await api.previewNode(id, text);
      // Only the node: the board's tree is about saved work, and rebuilding it on
      // every keystroke is exactly what made checking feel slow.
      state.detail = result.detail;
      // The server publishes a moment later: follow it, so the diagnostics under
      // the caret are Lean's, not the previous ones.
      void pollProcessing();
    } catch (error) {
      state.saveError = error instanceof Error ? error.message : String(error);
    }
  },

  /** Throws the unsaved edit away and re-checks what is on disk. */
  async discardDraft() {
    const id = state.detail?.id;
    state.editorDirty = false;
    if (!id) return;
    try {
      await api.discardDraft(id);
      await loadDetail(id);
    } catch (error) {
      state.saveError = error instanceof Error ? error.message : String(error);
    }
  },

  async applyText(text: string) {
    const id = state.detail?.id;
    if (!id) return;
    state.saving = true;
    state.saveError = null;
    try {
      const result = await api.updateNode(id, text);
      state.detail = result.detail;
      state.editorDirty = false;
      if (state.view) {
        state.view.tree = result.tree;
        state.view.stats = result.stats;
      }
      await reloadBoards();
      void loadGoals();
      void pollProcessing();
    } catch (error) {
      state.saveError = error instanceof Error ? error.message : String(error);
    } finally {
      state.saving = false;
    }
  },

  async rename(name: string) {
    const id = state.detail?.id;
    if (!id) return;
    state.saving = true;
    state.saveError = null;
    try {
      const result = await api.renameNode(id, name);
      state.detail = result.detail;
      setSelection([result.id]);
      if (state.view) {
        state.view.tree = result.tree;
        state.view.stats = result.stats;
      }
      await reloadBoards();
      void pollProcessing();
    } catch (error) {
      state.saveError = error instanceof Error ? error.message : String(error);
    } finally {
      state.saving = false;
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
    if (state.detail?.path === path) {
      state.detail = null;
      state.editorOpen = false;
      setSelection([]);
    }
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
    if (state.detail?.id === id) {
      state.detail = null;
      state.editorOpen = false;
    }
    if (state.selectedIds.includes(id)) {
      setSelection(state.selectedIds.filter((current) => current !== id));
    }
    state.hover = null;
    state.notice = "declaration removed";
    return true;
  },

  /** The whole file the node came from, untouched. */
  async showFile() {
    const id = state.detail?.id;
    if (!id) return;
    const text = await run("file", () => api.fileSource(id));
    if (text === null) return;
    state.printed = {
      title: state.detail?.path ?? "File",
      subtitle: "the file as it is on disk",
      text,
    };
    state.printedOpen = true;
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
