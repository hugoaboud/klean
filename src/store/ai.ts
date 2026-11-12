// AI sessions, as the app sees them.
//
// The sessions themselves live in Rust (`klean-ai`), which owns the providers,
// the streaming and the tools. This store keeps the copy the modal renders: it
// polls while something is running and holds the UI state (which tab is open,
// whether the modal is visible).

import { reactive } from "vue";
import * as ai from "../api/ai";

interface State {
  /** Every session, newest last. */
  sessions: ai.Session[];
  /** The tab the modal shows. */
  active: string;
  /** Whether the AI modal is visible. Sessions survive it being closed. */
  open: boolean;
  /** True while a poll is scheduled. */
  polling: boolean;
  /** Set when the last command failed. */
  error: string | null;
  /** True when AI is switched on and has a key. */
  ready: boolean;
}

export const aiState = reactive<State>({
  sessions: [],
  active: "",
  open: false,
  polling: false,
  error: null,
  ready: false,
});

let timer: number | null = null;

function running(): boolean {
  return aiState.sessions.some((session) => session.running);
}

/** Reads the sessions from Rust and schedules the next read while one runs. */
export async function poll(): Promise<void> {
  try {
    const sessions = await ai.aiSessions();
    aiState.sessions = sessions;
    aiState.error = null;
    if (!sessions.some((session) => session.id === aiState.active)) {
      aiState.active = sessions.length > 0 ? sessions[sessions.length - 1].id : "";
    }
  } catch (error) {
    aiState.error = error instanceof Error ? error.message : String(error);
  }
  aiState.ready = await ai.aiReady().catch(() => false);
  if (running()) {
    schedule();
  } else {
    aiState.polling = false;
  }
}

function schedule() {
  if (aiState.polling) return;
  aiState.polling = true;
  timer = window.setTimeout(() => {
    aiState.polling = false;
    void poll();
  }, 220);
}

function kick() {
  if (timer !== null) window.clearTimeout(timer);
  aiState.polling = false;
  schedule();
}

export const aiActions = {
  /** Shows the modal, reading the sessions first. */
  async show() {
    aiState.open = true;
    await poll();
  },

  hide() {
    aiState.open = false;
  },

  select(id: string) {
    aiState.active = id;
  },

  /** Opens a session seeded with nodes and shows it; this is what every AI request does first. */
  async start(origin: string, title: string, nodes: string[]): Promise<string | null> {
    try {
      const id = await ai.aiOpen(origin, title, nodes);
      aiState.active = id;
      aiState.open = true;
      aiState.error = null;
      await poll();
      return id;
    } catch (error) {
      aiState.error = error instanceof Error ? error.message : String(error);
      aiState.open = true;
      return null;
    }
  },

  /** Sends a prompt to a session (or starts one when there is none). */
  async send(text: string, session?: string): Promise<void> {
    const id = session ?? aiState.active;
    if (!id) return;
    try {
      await ai.aiPrompt(id, text);
      aiState.error = null;
      kick();
    } catch (error) {
      aiState.error = error instanceof Error ? error.message : String(error);
    }
  },

  /** Answers an `ask`/`ask_multi`. */
  async answer(entry: string, answer: string): Promise<void> {
    const id = aiState.active;
    if (!id) return;
    try {
      await ai.aiAnswer(id, entry, answer);
      kick();
    } catch (error) {
      aiState.error = error instanceof Error ? error.message : String(error);
    }
  },

  /**
   * Reports how a `diagram` tool call went: the modal draws it, and mermaid's
   * complaint goes back to the model, which may try again.
   */
  async diagram(entry: string, error: string | null): Promise<void> {
    const id = aiState.active;
    if (!id) return;
    try {
      await ai.aiDiagram(id, entry, error);
    } catch {
      // The turn may already have moved on; the feed says why in that case.
    }
  },

  async close(id: string): Promise<void> {
    try {
      await ai.aiClose(id);
    } catch {
      /* the session is gone either way */
    }
    await poll();
    if (aiState.sessions.length === 0) aiState.open = false;
  },
};
