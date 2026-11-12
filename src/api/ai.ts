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
  /**
   * For `diagram`: the mermaid source as the model wrote it. The result carries a
   * sentence for the model, so the drawing has to come from here.
   */
  source?: string | null;
}

export type Entry = ContextEntry | InputEntry | OutputEntry | McpEntry;

/** A question the model asked through `ask`, waiting for the user. */
export interface Question {
  entry: string;
  prompt: string;
  options: string[];
  multi: boolean;
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

export const aiOpen = (origin: string, title: string, nodes: string[]) =>
  invoke<string>("ai_open", { origin, title, nodes });

export const aiPrompt = (session: string, text: string) =>
  invoke<void>("ai_prompt", { session, text });

/** Reports whether the board could draw a `diagram` call; `error` = mermaid's. */
export const aiDiagram = (session: string, entry: string, error: string | null) =>
  invoke<void>("ai_diagram", { session, entry, error });

export const aiAnswer = (session: string, entry: string, answer: string) =>
  invoke<void>("ai_answer", { session, entry, answer });

export const aiClose = (session: string) => invoke<void>("ai_close", { session });

export const aiReady = () => invoke<boolean>("ai_ready");
