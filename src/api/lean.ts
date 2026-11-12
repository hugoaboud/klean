// Typed wrappers around the Tauri commands. No logic lives here: the
// front-end only hands data to Rust and renders what comes back.

import { invoke } from "@tauri-apps/api/core";

export interface TreeNode {
  id: string;
  label: string;
  /** Fully qualified name; empty for containers and columns. */
  qual: string;
  kind: string;
  detail: string | null;
  doc: string | null;
  path: string | null;
  line: number | null;
  /** Proof state of a theorem: `proven`, `sorry` or `error`. */
  status: string | null;
  child_count: number;
  children: TreeNode[];
}

export interface DiagnosticDto {
  line: number;
  severity: string;
  message: string;
}

export interface NodeDetail {
  id: string;
  label: string;
  name: string | null;
  renamable: boolean;
  kind: string;
  path: string | null;
  text: string;
  editable: boolean;
  start_line: number;
  /** 0-based UTF-16 column of the node's first character on `start_line`. */
  start_column: number;
  end_line: number;
  doc: string | null;
  /** Plain comments above the node, editable. */
  comments: string | null;
  signature: string | null;
  diagnostics: DiagnosticDto[];
}

export interface Stats {
  libs: number;
  files: number;
  nodes: number;
}

export interface WorkspaceView {
  label: string;
  backend: string;
  root: string | null;
  tree: TreeNode[];
  stats: Stats;
  /**
   * True when the files have not been read yet: the tree is the file listing, and
   * the background scan fills in the declarations.
   */
  pending: boolean;
}

export interface GraphNode {
  id: string;
  /** Stable identity (kind + name); positions are stored against it. */
  key: string;
  label: string;
  kind: string;
  parent: string | null;
  child_count: number;
  expanded: boolean;
  path: string | null;
  /** Logic boards only: what the node holds (a theory's terms). */
  detail: string | null;
  x: number;
  y: number;
  /** Proof state; for groups the worst state inside them. */
  status: string | null;
}

export interface GraphLink {
  source: string;
  target: string;
  /** Set on logic edges: the kind of the term being used. */
  relation?: string | null;
  /** Where the user moved this link's middle segment, if anywhere. */
  bend?: number | null;
  /** A reference to a nested theory: structural, always drawn as a link. */
  structural?: boolean;
}

export interface FixedNode {
  id: string;
  x: number;
  y: number;
}

/** A remembered graph view. */
export interface Camera {
  x: number;
  y: number;
  k: number;
}

export interface ProjectGraph {
  nodes: GraphNode[];
  links: GraphLink[];
  camera: Camera | null;
}

export interface UpdateResult {
  id: string;
  detail: NodeDetail;
  tree: TreeNode[];
  stats: Stats;
  diagnostics: DiagnosticDto[];
  ok: boolean;
  /** True while the language server is still checking the file. */
  processing: boolean;
}

export const openProject = (path: string) =>
  invoke<WorkspaceView>("open_project", { path });

export const openProjectZip = (path: string) =>
  invoke<WorkspaceView>("open_project_zip", { path });

export const importProjectZip = (data: string, label?: string) =>
  invoke<WorkspaceView>("import_project_zip", { data, label: label ?? null });

export const newMemoryProject = (label?: string) =>
  invoke<WorkspaceView>("new_memory_project", { label: label ?? null });

export const closeProject = () => invoke<void>("close_project");

export const workspaceView = () => invoke<WorkspaceView>("workspace_view");

export const graphView = () => invoke<ProjectGraph>("graph_view");

export const toggleGraphNode = (id: string) =>
  invoke<ProjectGraph>("toggle_graph_node", { id });

export const focusGraphNode = (id: string) =>
  invoke<ProjectGraph>("focus_graph_node", { id });

export const nodeDetail = (id: string) => invoke<NodeDetail>("node_detail", { id });

export const updateNode = (id: string, text: string) =>
  invoke<UpdateResult>("update_node", { id, text });

export const renameNode = (id: string, name: string) =>
  invoke<UpdateResult>("rename_node", { id, name });

export const exportZipData = () => invoke<string>("export_zip_data");

export const exportZipTo = (path: string) =>
  invoke<number>("export_zip_to", { path });

export const exportOutline = () => invoke<string>("export_outline");

export const exportOutlineTo = (path: string) =>
  invoke<string>("export_outline_to", { path });

export const printFile = (id: string) => invoke<string>("print_file", { id });

/** Removes a declaration from the file it lives in. */
export const deleteNode = (id: string) => invoke<void>("delete_node", { id });

/** The file a node lives in, exactly as it is on disk. */
export const fileSource = (id: string) => invoke<string>("file_source", { id });

/** One step of the logic breadcrumbs: the root graph, or a theory. */
export interface TheoryRef {
  key: string;
  name: string;
}

/** A horizontal divider inside a theory graph. */
export interface GraphLayer {
  id: string;
  name: string;
  y: number;
}

/** A node referenced from outside the open theory graph. */
export interface GraphTarget {
  label: string;
  kind: string;
  /** Namespace the referenced node lives in; empty at the top level. */
  namespace: string;
}

/** Everything one node refers to outside its own theory graph. */
export interface GraphExternal {
  source: string;
  /** `left` or `right`: the edge the dashed line points at. */
  side: string;
  targets: GraphTarget[];
}

export interface LogicGraph {
  /** Breadcrumbs, root graph first. */
  trail: TheoryRef[];
  /** Theory nodes on the root graph, terms and nested theories inside one. */
  nodes: GraphNode[];
  links: GraphLink[];
  /** Dividers of the open theory graph; empty at the root. */
  layers: GraphLayer[];
  external: GraphExternal[];
  /** Theory nodes as tree roots, each with its namespaces and terms. */
  tree: TreeNode[];
  /** Graph the positions and the camera are stored under. */
  graph: string;
  camera: Camera | null;
}

/** One entry of the project settings tree. */
export interface SettingNode {
  id: string;
  label: string;
  kind: "group" | "bool" | "enum" | "text" | "action";
  value: string;
  options: string[];
  help: string;
  /** Setting that must be switched on for this one to be shown. */
  depends_on: string | null;
  /** True for a value written over several lines: rendered as a text area. */
  multiline: boolean;
  children: SettingNode[];
}

export interface RecentProject {
  label: string;
  path: string;
  kind: string;
}

export const recentProjects = () => invoke<RecentProject[]>("recent_projects");

export const createFile = (path: string) => invoke<WorkspaceView>("create_file", { path });

export const deleteFile = (path: string) => invoke<WorkspaceView>("delete_file", { path });

export const logicGraph = (theory?: string | null) =>
  invoke<LogicGraph>("logic_graph", { theory: theory ?? null });

export const createTheory = (theory: string | null, name: string) =>
  invoke<LogicGraph>("create_theory", { theory, name });

export const renameTheory = (theory: string | null, key: string, name: string) =>
  invoke<LogicGraph>("rename_theory", { theory, key, name });

/** Adds an axiom to an existing file (`file`) or to a new one (`path`). */
export const addAxiom = (
  theory: string | null,
  name: string,
  file: string | null,
  path: string | null,
) => invoke<LogicGraph>("add_axiom", { theory, name, file, path });

export const deleteTheory = (theory: string | null, key: string) =>
  invoke<LogicGraph>("delete_theory", { theory, key });

export const assignTheory = (theory: string | null, ids: string[], target: string | null) =>
  invoke<LogicGraph>("assign_theory", { theory, ids, target });

export const createLayer = (theory: string, name: string, y: number) =>
  invoke<LogicGraph>("create_layer", { theory, name, y });

export const deleteLayer = (theory: string, id: string) =>
  invoke<LogicGraph>("delete_layer", { theory, id });

export const moveLayer = (theory: string, id: string, y: number) =>
  invoke<LogicGraph>("move_layer", { theory, id, y });

export const renameLayer = (theory: string, id: string, name: string) =>
  invoke<LogicGraph>("rename_layer", { theory, id, name });

export const savePositions = (graph: string, positions: FixedNode[]) =>
  invoke<void>("save_positions", { graph, positions });

export const settingsView = () => invoke<SettingNode[]>("settings_view");

export const saveSettings = (values: [string, string][]) =>
  invoke<SettingNode[]>("save_settings", { values });

export const saveCamera = (graph: string, camera: Camera) =>
  invoke<void>("save_camera", { graph, x: camera.x, y: camera.y, k: camera.k });

export const saveLinkBend = (graph: string, key: string, value: number | null) =>
  invoke<void>("save_link_bend", { graph, key, value });

/** Lean as this project sees it: the toolchain, the build, the last failure. */
export interface LeanStatus {
  /** Lean integration switched on in settings. */
  enabled: boolean;
  /** A toolchain was found. */
  available: boolean;
  /** `system`, `elan`, `managed` or `custom`. */
  origin: string;
  /** `Lean (version 4.35.0, …)`. */
  version: string;
  lean: string | null;
  /** Why Lean cannot be used, when it cannot. */
  error: string | null;
  /** `folder`, `archive` or `memory`: only folders can run Lean. */
  backend: string;
  /** The project is a Lake package. */
  lake: boolean;
  /** The in-process parser is ready. */
  parser: boolean;
  /** The language server is running. */
  server: boolean;
  /** Files the server is still working on (project-relative paths). */
  processing: string[];
  building: boolean;
  /** Last `lake build` outcome, one line. */
  build: string | null;
}

/** Proof goals at one position, as Lean's language server renders them. */
export interface Goals {
  goals: string[];
  rendered: string;
  /** Shown when there is no goal at the position. */
  note: string | null;
}

export const leanStatus = () => invoke<LeanStatus>("lean_status");

/** The build console: the tail of `lake build`'s output and how it stands. */
export interface BuildState {
  running: boolean;
  /** The command being run, for the console header. */
  command: string;
  /** Output lines, oldest first. */
  lines: string[];
  /** One-line verdict once the build is over. */
  summary: string | null;
  failed: boolean;
  /** Seconds since the build started. */
  seconds: number;
}

/**
 * How the background read of the project stands: KLEAN parses every file after a
 * project opens, and this is what the console follows.
 */
export interface ScanState {
  running: boolean;
  /** Files to read. */
  total: number;
  /** Files read so far. */
  done: number;
  /** The file being read. */
  current: string;
  /** Progress lines, oldest first. */
  lines: string[];
  /** One-line verdict once the read is over. */
  summary: string | null;
  failed: boolean;
  /** Seconds since the read started. */
  seconds: number;
}

/** Reads the open project's files in the background; returns at once. */
export const leanScan = () => invoke<ScanState>("scan_project");

/** How the background read stands. */
export const leanScanState = () => invoke<ScanState>("scan_state");

/** Stops the background read. */
export const leanScanCancel = () => invoke<void>("scan_cancel");

/**
 * The answer to a keystroke: the node as it now reads and what Lean says about it.
 * Nothing is written to disk.
 */
export interface PreviewResult {
  id: string;
  detail: NodeDetail;
  diagnostics: DiagnosticDto[];
  /** True when Lean reported no errors for the edited file. */
  ok: boolean;
  /** True while the server is still checking: ask again shortly. */
  processing: boolean;
}

/** Checks an edited node without saving it; the text goes to Lean at once. */
export const previewNode = (id: string, text: string) =>
  invoke<PreviewResult>("preview_node", { id, text });

/** Throws an unsaved edit away and re-checks the file as it is on disk. */
export const discardDraft = (id: string) => invoke<void>("discard_draft", { id });

/** Starts `lake build` in the background and returns immediately. */
export const leanBuild = () => invoke<BuildState>("lean_build");

/** The console's current contents. */
export const leanBuildState = () => invoke<BuildState>("lean_build_state");

/** Stops a running build. */
export const leanBuildCancel = () => invoke<BuildState>("lean_build_cancel");

/** Asks Lean to check one file in the background. */
export const leanCheckFile = (id: string) => invoke<void>("lean_check_file", { id });

/** Goals at a position: 1-based line, 0-based UTF-16 column. */
export const leanGoals = (id: string, line?: number, character?: number) =>
  invoke<Goals>("lean_goals", { id, line: line ?? null, character: character ?? null });

/** The type of the term at a position, as Lean renders it. */
export const leanHover = (id: string, line?: number, character?: number) =>
  invoke<string | null>("lean_hover", { id, line: line ?? null, character: character ?? null });

/** Downloads elan + the configured channel; resolves with the `lean` path. */
export const leanInstall = () => invoke<string>("lean_install");


export const updateComments = (id: string, text: string) =>
  invoke<UpdateResult>("update_comments", { id, text });
