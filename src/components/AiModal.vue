<script setup lang="ts">
// The AI modal: one tab per session, a feed of what was said and done, and a
// prompt box at the bottom.
//
// Sessions live in Rust (`klean-ai`); this renders the snapshot the store polls
// while a model answers. Every entry except the model's own output folds away
// behind a one-line preview, so a long session stays readable.
import { computed, nextTick, ref, watch } from "vue";
import type { ContextEntry, Entry, McpEntry, OutputEntry, Session } from "../api/ai";
import hljs from "../lib/highlight";
import { aiActions, aiState } from "../store/ai";
import ContextGraph from "./ContextGraph.vue";
import MarkdownView from "./MarkdownView.vue";
import MermaidDiagram from "./MermaidDiagram.vue";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiSpinner from "./ui/UiSpinner.vue";
import UiModal from "./ui/UiModal.vue";

const text = ref("");
const feed = ref<HTMLElement | null>(null);
/** Explicit fold choices; anything not in here falls back to its type's default. */
const folded = ref<Record<string, boolean>>({});
/** Which view a context entry shows: its graph or its Lean text. */
const contextTab = ref<Record<string, "graph" | "text">>({});

const active = computed<Session | null>(
  () => aiState.sessions.find((session) => session.id === aiState.active) ?? null,
);
const busy = computed(() => Boolean(active.value?.running));
const question = computed(() => active.value?.question ?? null);

/** Highlighted Lean. */
function code(source: string): string {
  try {
    return hljs.highlight(source, { language: "lean", ignoreIllegals: true }).value;
  } catch {
    return source.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
}

/**
 * Whether an entry is unfolded. Outputs stay open — they are what the tab is for —
 * and an input starts open because it is the anchor of the exchange.
 */
function isOpen(entry: Entry): boolean {
  const choice = folded.value[entry.id];
  if (choice !== undefined) return !choice;
  if (entry.type === "input") return true;
  // A diagram is a picture: showing its source (or a collapsed row) is not the
  // point, and drawing it is also what tells the model whether it was valid.
  return entry.type === "mcp" && entry.tool === "diagram";
}

function toggle(entry: Entry) {
  folded.value = { ...folded.value, [entry.id]: isOpen(entry) };
}

function tabOf(entry: ContextEntry): "graph" | "text" {
  return contextTab.value[entry.id] ?? "graph";
}

function setTab(entry: ContextEntry, tab: "graph" | "text") {
  contextTab.value = { ...contextTab.value, [entry.id]: tab };
}

function firstLine(value: string): string {
  const line = value
    .split("\n")
    .find((candidate) => candidate.trim() && !candidate.trimStart().startsWith("--"));
  return line?.trim() ?? "";
}

function parseInput(entry: McpEntry): Record<string, unknown> {
  try {
    const value = JSON.parse(entry.input || "{}");
    return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** What a tool was asked for, in words rather than raw JSON. */
function toolSummary(entry: McpEntry): string {
  const input = parseInput(entry);
  switch (entry.tool) {
    case "view_ast": {
      const query = String(input.query ?? "").trim();
      const parts = [query ? `query “${query}”` : "the whole tree"];
      if (input.depth !== undefined) parts.push(`depth ${String(input.depth)}`);
      return parts.join(" · ");
    }
    case "expand": {
      const names = Array.isArray(input.names) ? input.names : [];
      return names.length ? names.join(", ") : "no names";
    }
    case "diagram":
      return firstLine(String(input.mermaid ?? "")) || "a diagram";
    case "ask":
    case "ask_multi": {
      const options = Array.isArray(input.options) ? input.options : [];
      const allowed = entry.multi ? "several allowed" : "one";
      return `${String(input.question ?? "").trim()} · ${options.length} answers, ${allowed}`;
    }
    default:
      return entry.input.trim() || "no arguments";
  }
}

/** One line that stands for an entry while it is folded. */
function preview(entry: Entry): string {
  if (entry.type === "context") return firstLine(entry.text) || entry.label;
  if (entry.type === "input") return entry.text;
  if (entry.type === "mcp") return toolSummary(entry);
  return "";
}

/** The full arguments of a tool call, one row per key. */
function inputRows(entry: McpEntry): { key: string; value: string }[] {
  return Object.entries(parseInput(entry)).map(([key, value]) => ({
    key,
    value: typeof value === "string" ? value : JSON.stringify(value, null, 2),
  }));
}

/**
 * The mermaid a `diagram` call asked for. It comes from the call's own arguments:
 * the tool *result* is the sentence the model reads ("the diagram is on screen"),
 * which mermaid cannot draw — that is what used to send the sentence to the
 * renderer and fail.
 */
function diagramSource(entry: McpEntry): string {
  if (entry.source) return entry.source;
  const source = parseInput(entry).mermaid;
  return typeof source === "string" ? source : "";
}

/** Options picked so far, per `ask_multi` entry. */
const picked = ref<Record<string, string[]>>({});

/** `ask`: one click is the answer. `ask_multi`: clicks collect, then send. */
function choose(entry: McpEntry, option: string) {
  if (!entry.multi) {
    void aiActions.answer(entry.id, option);
    return;
  }
  const current = picked.value[entry.id] ?? [];
  picked.value = {
    ...picked.value,
    [entry.id]: current.includes(option)
      ? current.filter((value) => value !== option)
      : [...current, option],
  };
}

function chosen(entry: McpEntry, option: string): boolean {
  return (picked.value[entry.id] ?? []).includes(option);
}

function sendMulti(entry: McpEntry) {
  const answers = picked.value[entry.id] ?? [];
  if (answers.length === 0) return;
  void aiActions.answer(entry.id, answers.join(", "));
}

/** Tokens in and out, with the total, or a note that the model does not say. */
const tokenLabel = computed(() => {
  const usage = active.value?.usage;
  if (!usage || (!usage.input_tokens && !usage.output_tokens)) return "tokens: not reported";
  return `${format(usage.input_tokens)} in · ${format(usage.output_tokens)} out · ${format(
    usage.input_tokens + usage.output_tokens,
  )} total`;
});

/** What the session has cost so far, in dollars. */
const costLabel = computed(() => {
  const dollars = active.value?.cost_usd;
  if (dollars === null || dollars === undefined) return "cost: unknown model price";
  return `$${dollars.toFixed(4)}`;
});

const costTitle = computed(() =>
  active.value?.cost_usd === null
    ? "KLEAN has no price for this model, so only tokens are shown"
    : "Every turn of this session, at the model's published per-million-token rates",
);

/** What the prompt box says: it depends on the session, which may carry nothing. */
const placeholder = computed(() => {
  if (busy.value) return "the model is answering…";
  if (question.value) return "pick an answer above";
  if (!active.value?.nodes.length) {
    return "ask anything — this session has no context, so the model will look for what it needs";
  }
  return "ask about this theorem, or anything in the project";
});

/** 1234 → `1,234`. */
function format(value: number): string {
  return value.toLocaleString("en-US");
}

async function send() {
  const value = text.value.trim();
  if (!value || busy.value || !active.value) return;
  text.value = "";
  await aiActions.send(value);
}

/**
 * A new session with no context at all: nothing from the board, no Lean source.
 * The model has to look for itself, with `view_ast` and `expand`.
 */
async function newSession() {
  const used = aiState.sessions.filter((session) => session.title.startsWith("new session")).length;
  await aiActions.start("free", used === 0 ? "new session" : `new session ${used + 1}`, []);
}

// Keep the newest entry in view while text streams in.
watch(
  () => active.value?.entries.length ?? 0,
  async () => {
    await nextTick();
    const box = feed.value;
    if (box) box.scrollTop = box.scrollHeight;
  },
);
</script>

<template>
  <UiModal
    :open="aiState.open"
    title="AI"
    :subtitle="active?.title ?? 'no session yet'"
    :width="860"
    :height="640"
    :resizable="true"
    @close="aiActions.hide()"
  >
    <div class="ai">
      <!-- Tabs: one per session, newest last, and a way to start a fresh one. -->
      <div class="ai__tabs">
        <button
          v-for="session in aiState.sessions"
          :key="session.id"
          class="ai__tab"
          type="button"
          :data-active="session.id === aiState.active || undefined"
          :title="session.title"
          @click="aiActions.select(session.id)"
        >
          <UiSpinner v-if="session.running" :size="9" />
          <span class="ai__tab-label">{{ session.title }}</span>
          <span
            class="ai__tab-close"
            role="button"
            title="Close this session"
            @click.stop="aiActions.close(session.id)"
            >✕</span
          >
        </button>
        <button
          class="ai__tab ai__tab--new"
          type="button"
          title="Start a session with no context: the model looks for what it needs"
          @click="newSession"
        >
          +
        </button>
      </div>

      <div v-if="aiState.error" class="ai__error">
        <UiIcon name="alert" :size="11" />
        <span>{{ aiState.error }}</span>
      </div>

      <!-- The feed. -->
      <div ref="feed" class="ai__feed">
        <div v-if="!active" class="ai__empty">no session open</div>
        <div v-else-if="!active.entries.length" class="ai__empty ai__empty--session">
          <span
            >this session carries no context: nothing from the board and no Lean source.</span
          >
          <span class="k-mono">ask a question, or let the model look for itself</span>
        </div>

        <template v-for="entry in active?.entries ?? []" :key="entry.id">
          <!-- KLEAN's reduced Lean context: a graph, or the file that is sent. -->
          <div
            v-if="entry.type === 'context'"
            class="ai-entry ai-entry--context"
            :data-open="isOpen(entry) || undefined"
          >
            <button class="ai-entry__head" type="button" @click="toggle(entry)">
              <span class="ai-entry__who">context</span>
              <span class="ai-entry__preview k-mono">{{ preview(entry) }}</span>
              <span class="ai-entry__meta k-mono">{{ entry.label }}</span>
              <UiIcon :name="isOpen(entry) ? 'chevron-down' : 'chevron'" :size="10" />
            </button>
            <template v-if="isOpen(entry)">
              <div class="ai-entry__tabs">
                <button
                  type="button"
                  class="ai-entry__tab"
                  :data-active="tabOf(entry) === 'graph' || undefined"
                  @click="setTab(entry, 'graph')"
                >
                  graph
                  <span class="k-mono">{{ entry.graph?.nodes.length ?? 0 }}</span>
                </button>
                <button
                  type="button"
                  class="ai-entry__tab"
                  :data-active="tabOf(entry) === 'text' || undefined"
                  @click="setTab(entry, 'text')"
                >
                  lean
                </button>
              </div>
              <ContextGraph v-if="tabOf(entry) === 'graph'" :graph="entry.graph" />
              <pre v-else class="ai-code hljs" v-html="code(entry.text)" />
            </template>
          </div>

          <!-- What the user asked. -->
          <div
            v-else-if="entry.type === 'input'"
            class="ai-entry ai-entry--input"
            :data-open="isOpen(entry) || undefined"
          >
            <button class="ai-entry__head" type="button" @click="toggle(entry)">
              <span class="ai-entry__who">you</span>
              <span class="ai-entry__preview">{{ preview(entry) }}</span>
              <UiIcon :name="isOpen(entry) ? 'chevron-down' : 'chevron'" :size="10" />
            </button>
            <p v-if="isOpen(entry)" class="ai-entry__text">{{ entry.text }}</p>
          </div>

          <!-- Model output: never folded. -->
          <div
            v-else-if="entry.type === 'output'"
            class="ai-entry"
            :data-kind="(entry as OutputEntry).kind"
            :data-done="(entry as OutputEntry).done || undefined"
          >
            <div class="ai-entry__head">
              <span class="ai-entry__who">{{
                (entry as OutputEntry).kind === "chain_of_thought"
                  ? "thinking"
                  : (entry as OutputEntry).kind === "markdown"
                    ? "document"
                    : "model"
              }}</span>
              <UiSpinner v-if="!(entry as OutputEntry).done" :size="9" />
              <span class="k-spacer" />
              <button
                v-if="(entry as OutputEntry).kind !== 'text'"
                class="ai-entry__toggle"
                type="button"
                @click="toggle(entry)"
              >
                {{ isOpen(entry) ? "collapse" : "expand" }}
              </button>
            </div>
            <pre
              v-if="(entry as OutputEntry).kind === 'chain_of_thought'"
              class="ai-entry__cot k-mono"
              >{{ (entry as OutputEntry).text }}</pre
            >
            <div
              v-else-if="(entry as OutputEntry).kind === 'markdown'"
              class="ai-entry__doc"
              :data-collapsed="!isOpen(entry) || undefined"
            >
              <MarkdownView
                :text="(entry as OutputEntry).text"
                :done="Boolean((entry as OutputEntry).done)"
              />
            </div>
            <div v-else class="ai-entry__answer">
              <MarkdownView
                :text="(entry as OutputEntry).text"
                :done="Boolean((entry as OutputEntry).done)"
              />
            </div>
          </div>

          <!-- A tool the model called: what it asked for, and what came back. -->
          <div
            v-else-if="entry.type === 'mcp'"
            class="ai-entry ai-entry--mcp"
            :data-open="isOpen(entry) || undefined"
          >
            <button class="ai-entry__head" type="button" @click="toggle(entry)">
              <span class="ai-entry__tool k-mono">{{ entry.tool }}</span>
              <span class="ai-entry__preview k-mono">{{ preview(entry) }}</span>
              <span class="ai-entry__state" :data-state="entry.state">{{ entry.state }}</span>
              <UiSpinner v-if="entry.state === 'running'" :size="9" />
              <UiIcon :name="isOpen(entry) ? 'chevron-down' : 'chevron'" :size="10" />
            </button>

            <template v-if="isOpen(entry)">
              <!-- The arguments, spelled out: a call is only clear with them. -->
              <div v-if="inputRows(entry).length" class="ai-entry__input">
                <div v-for="row in inputRows(entry)" :key="row.key" class="ai-entry__input-row">
                  <span class="ai-entry__input-key k-mono">{{ row.key }}</span>
                  <pre class="ai-entry__input-value k-mono">{{ row.value }}</pre>
                </div>
              </div>

              <MermaidDiagram
                v-if="entry.tool === 'diagram' && diagramSource(entry)"
                :source="diagramSource(entry)"
                report
                @drawn="(error) => aiActions.diagram(entry.id, error)"
              />
              <pre v-else-if="entry.result" class="ai-entry__result k-mono">{{ entry.result }}</pre>

              <!-- A question the model asked, waiting for an answer. -->
              <div v-if="entry.state === 'waiting'" class="ai-entry__ask">
                <div class="ai-entry__options">
                  <UiButton
                    v-for="option in entry.options"
                    :key="option"
                    size="sm"
                    :variant="chosen(entry, option) ? 'solid' : 'subtle'"
                    @click="choose(entry, option)"
                  >
                    {{ option }}
                  </UiButton>
                  <UiButton
                    v-if="entry.multi"
                    size="sm"
                    variant="solid"
                    :disabled="(picked[entry.id] ?? []).length === 0"
                    @click="sendMulti(entry)"
                  >
                    send
                  </UiButton>
                </div>
              </div>
            </template>
          </div>
        </template>
      </div>

      <!-- What the session has used so far. -->
      <div v-if="active" class="ai__cost k-mono">
        <span>{{ tokenLabel }}</span>
        <span class="ai__cost-sep">·</span>
        <span :title="costTitle">{{ costLabel }}</span>
        <span class="k-spacer" />
        <span v-if="active.nodes.length" class="ai__cost-nodes"
          >{{ active.nodes.length }} node(s) in context</span
        >
      </div>

      <!-- The prompt belongs to the open session: without a tab there is none. -->
      <div v-if="!active" class="ai__start">
        <span class="ai__start-text"
          >no session open — <button class="ai__start-link" type="button" @click="newSession">
            start one
          </button>
          to ask about something in the project</span
        >
      </div>
      <div v-else class="ai__compose">
        <textarea
          v-model="text"
          class="ai__input k-mono"
          rows="2"
          spellcheck="false"
          :disabled="busy"
          :placeholder="placeholder"
          @keydown.enter.meta.prevent="send"
          @keydown.enter.ctrl.prevent="send"
        />
        <UiButton size="sm" variant="solid" :disabled="busy || !text.trim()" @click="send">
          <UiSpinner v-if="busy" :size="10" />
          <UiIcon v-else name="send" :size="11" />
          send
        </UiButton>
      </div>
    </div>
  </UiModal>
</template>

<style scoped>
.ai {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  gap: 6px;
  padding: 2px 2px 0;
}

.ai__tabs {
  display: flex;
  align-items: center;
  gap: 3px;
  overflow-x: auto;
  padding-bottom: 2px;
  border-bottom: 1px solid var(--line);
}

.ai__tab {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 200px;
  padding: 3px 7px;
  border: 1px solid transparent;
  border-bottom: none;
  border-radius: 2px 2px 0 0;
  background: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  cursor: pointer;
}

.ai__tab[data-active] {
  border-color: var(--line);
  background: var(--bg-1);
  color: var(--text);
}

.ai__tab--new {
  padding: 1px 8px;
  border-color: var(--line);
  color: var(--text-faint);
  font-size: var(--fs-sm);
  line-height: 1.1;
}

.ai__tab--new:hover {
  border-color: var(--accent-line);
  color: var(--accent-bright);
}

.ai__tab-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ai__tab-close {
  color: var(--text-ghost);
  font-size: 9px;
}

.ai__tab-close:hover {
  color: var(--danger);
}

.ai__error {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border-left: 2px solid var(--danger);
  background: var(--danger-soft);
  color: var(--danger);
  font-size: var(--fs-xs);
}

.ai__feed {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  gap: 6px;
  overflow: auto;
  padding: 2px;
}

.ai__empty {
  margin: auto;
  color: var(--text-ghost);
  font-size: var(--fs-xs);
  font-style: italic;
}

.ai__empty--session {
  display: flex;
  flex-direction: column;
  gap: 5px;
  max-width: 44ch;
  text-align: center;
  font-style: normal;
  color: var(--text-faint);
  line-height: 1.6;
}

.ai__empty--session .k-mono {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

/* The model's own words sit on the modal's surface; everything else — the prompt,
   a context, a tool call — is set back a step, so an answer reads as the reply to a
   quieter record. */
.ai-entry {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 5px 8px;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
  background: var(--bg-0);
}

.ai-entry[data-kind] {
  background: rgba(255, 255, 255, 0.012);
}

.ai-entry--input {
  border-left: 2px solid var(--accent-line);
}

.ai-entry--context {
  border-left: 2px solid var(--line-strong);
}

.ai-entry--mcp {
  border-left: 2px solid rgba(120, 200, 220, 0.35);
}

.ai-entry[data-kind="chain_of_thought"] {
  border-style: dashed;
  opacity: 0.75;
}

.ai-entry[data-kind="markdown"] {
  border-left: 2px solid var(--ok);
}

.ai-entry__head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 0;
  background: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.ai-entry[data-kind] .ai-entry__head {
  cursor: default;
}

.ai-entry__who {
  flex: 0 0 auto;
  color: var(--text-ghost);
  font-size: 8.5px;
  letter-spacing: 0.09em;
  text-transform: uppercase;
}

.ai-entry__tool {
  color: var(--accent-bright);
  font-size: var(--fs-xs);
}

.ai-entry__preview {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-faint);
  font-size: var(--fs-xs);
}

.ai-entry[data-open] .ai-entry__preview {
  color: var(--text-ghost);
}

.ai-entry__meta {
  color: var(--text-ghost);
  font-size: 9px;
}

.ai-entry__state {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.ai-entry__state[data-state="ok"] {
  color: var(--ok);
}

.ai-entry__state[data-state="failed"] {
  color: var(--danger);
}

.ai-entry__state[data-state="waiting"] {
  color: var(--warning);
}

.ai-entry__tabs {
  display: flex;
  align-items: center;
  gap: 3px;
}

.ai-entry__tab {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border: 1px solid transparent;
  border-radius: 2px;
  background: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  cursor: pointer;
}

.ai-entry__tab[data-active] {
  border-color: var(--line);
  background: var(--bg-1);
  color: var(--text);
}

.ai-entry__text {
  margin: 0;
  white-space: pre-wrap;
  color: var(--text);
  font-size: var(--fs-sm);
  line-height: 1.55;
}

.ai-entry__cot {
  margin: 0;
  padding: 5px 7px;
  border-radius: 2px;
  background: #0b0d0f;
  color: var(--text-ghost);
  font-size: var(--fs-xs);
  line-height: 1.55;
  white-space: pre-wrap;
}

.ai-entry__toggle {
  background: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  cursor: pointer;
}

.ai-entry__toggle:hover {
  color: var(--text-dim);
}

.ai-entry__input {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 6px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.02);
}

.ai-entry__input-row {
  display: flex;
  gap: 6px;
  min-width: 0;
}

.ai-entry__input-key {
  flex: 0 0 auto;
  color: var(--text-ghost);
  font-size: 9.5px;
}

.ai-entry__input-value {
  flex: 1;
  min-width: 0;
  margin: 0;
  max-height: 90px;
  overflow: auto;
  color: var(--text-faint);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
}

.ai-entry__result {
  margin: 0;
  max-height: 160px;
  overflow: auto;
  color: var(--text-faint);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
}

.ai-entry__ask {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding-top: 3px;
  border-top: 1px dashed var(--line);
}

.ai-entry__options {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

/* The model's answer is rendered markdown and is the point of the feed, so it is a
   step larger than the labels and tool output around it. */
.ai-entry__answer {
  --md-scale: 1.5;
}

.ai-entry__doc {
  max-height: 320px;
  overflow: auto;
}

.ai-entry__doc[data-collapsed] {
  max-height: 84px;
  overflow: hidden;
  mask-image: linear-gradient(180deg, #000 55%, transparent);
}

.ai-code {
  margin: 0;
  max-height: 260px;
  overflow: auto;
  padding: 6px 8px;
  border-radius: 2px;
  background: #0b0d0f;
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
}

.ai__cost {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 2px 0;
  color: var(--text-ghost);
  font-size: 9px;
}

.ai__cost-sep {
  color: var(--line-strong);
}

.ai__cost-nodes {
  color: var(--text-ghost);
}

/* No tab, no prompt: the row says how to get one instead. */
.ai__start {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 8px 0 2px;
  border-top: 1px solid var(--line);
  color: var(--text-ghost);
  font-size: var(--fs-xs);
}

.ai__start-link {
  padding: 0;
  border: none;
  background: none;
  color: var(--accent-bright);
  font-size: inherit;
  cursor: pointer;
}

.ai__start-link:hover {
  text-decoration: underline;
}

.ai__compose {
  display: flex;
  align-items: flex-end;
  gap: 6px;
  padding-top: 6px;
  border-top: 1px solid var(--line);
}

.ai__input {
  flex: 1;
  min-height: 38px;
  max-height: 120px;
  padding: 6px 8px;
  resize: none;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
  color: var(--text);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.ai__input:focus {
  border-color: var(--accent-line);
  background: var(--bg-2);
}
</style>
