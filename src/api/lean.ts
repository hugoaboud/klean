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
  kind: "group" | "bool" | "enum" | "text";
  value: string;
  options: string[];
  help: string;
  /** Setting that must be switched on for this one to be shown. */
  depends_on: string | null;
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


export const updateComments = (id: string, text: string) =>
  invoke<UpdateResult>("update_comments", { id, text });
