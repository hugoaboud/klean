// Typed wrappers for the AI commands. The sessions live in Rust; this only
// renders them and sends prompts.

import { invoke } from "@tauri-apps/api/core";

export type EntryType = "context" | "input" | "output" | "mcp";
export type OutputKind = "chain_of_thought" | "text" | "markdown";
export type McpState = "running" | "waiting" | "ok" | "failed";

/** A declaration in a context, and how contexts refer to each other. */
export interface ContextNode {
  id: string;
  label: string;
  kind: string;
  /** True for the declarations the request started from. */
  seed: boolean;
}

export interface ContextGraph {
  nodes: ContextNode[];
  /** `[from, to]`: from refers to to. */
  edges: [string, string][];
}

export interface ContextEntry {
  type: "context";
  id: string;
  label: string;
  text: string;
  nodes: string[];
  graph: ContextGraph;
}

export interface InputEntry {
  type: "input";
  id: string;
  text: string;
  nodes: string[];
}

export interface OutputEntry {
  type: "output";
  id: string;
  kind: OutputKind;
  /** False while the text is still streaming in. */
  done: boolean;
  text: string;
}

export interface McpEntry {
  type: "mcp";
  id: string;
  tool: string;
  /** Arguments, as JSON text. */
  input: string;
  state: McpState;
  result: string | null;
  /** Options of an `ask`/`ask_multi` question. */
  options: string[];
  multi: boolean;
}

/** A declaration the model changed through `modify_ast`. */
export interface NodeChange {
  /** The node id, as the project names it; empty for a change already undone. */
  id: string;
  /** Fully qualified name, or the label a new declaration was given. */
  name: string;
  file: string;
  /** The Lean kind: `def`, `theorem`, … */
  kind: string;
  /** `added`, `edited` or `removed`. */
  change: string;
  /** The declaration before the change; empty for a new one. */
  before: string;
  /** The declaration after it; empty for a removed one. */
  after: string;
}

/** What one prompt changed, added to the feed when the model stops. */
export interface DiffEntry {
  type: "diff";
  id: string;
  changes: NodeChange[];
}

export type Entry = ContextEntry | InputEntry | OutputEntry | McpEntry | DiffEntry;

/** A question the model asked through `ask`, waiting for the user. */
export interface Question {
  entry: string;
  prompt: string;
  options: string[];
  multi: boolean;
}

/** The declaration a session is focused on. */
export interface Target {
  /** The node id, when KLEAN could resolve it. */
  id: string;
  /** Fully qualified name: namespace plus name, as `view_ast` prints it. */
  name: string;
}

export interface Session {
  id: string;
  title: string;
  /** `editor`, `logic`, `free`, … */
  origin: string;
  entries: Entry[];
  running: boolean;
  error: string | null;
  /** AST nodes this session carries as context. */
  nodes: string[];
  question: Question | null;
  /** The declaration this session focuses on, if it was started for one. */
  target: Target | null;
  /** Tokens used so far, when the provider reports them. */
  usage: Usage;
  /** What those tokens cost, in dollars; null when the model has no known price. */
  cost_usd: number | null;
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
}

export const aiSessions = () => invoke<Session[]>("ai_sessions");

export const aiOpen = (
  origin: string,
  title: string,
  nodes: string[],
  target: Target | null = null,
) => invoke<string>("ai_open", { origin, title, nodes, target });

export const aiPrompt = (session: string, text: string) =>
  invoke<void>("ai_prompt", { session, text });

export const aiAnswer = (session: string, entry: string, answer: string) =>
  invoke<void>("ai_answer", { session, entry, answer });

export const aiClose = (session: string) => invoke<void>("ai_close", { session });

export const aiReady = () => invoke<boolean>("ai_ready");
