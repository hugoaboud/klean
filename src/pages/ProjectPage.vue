<script setup lang="ts">
// Project shell: AST board (graph + tree) or Logic board, with the shared editor
// modal and the bottom view switcher.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import GraphCanvas from "../components/GraphCanvas.vue";
import LeanTreePanel from "../components/LeanTreePanel.vue";
import NodeEditor from "../components/NodeEditor.vue";
import PrintedViewer from "../components/PrintedViewer.vue";
import UiButton from "../components/ui/UiButton.vue";
import UiIcon from "../components/ui/UiIcon.vue";
import UiModal from "../components/ui/UiModal.vue";
import UiPrompt from "../components/ui/UiPrompt.vue";
import UiSnackbar from "../components/ui/UiSnackbar.vue";
import UiSpinner from "../components/ui/UiSpinner.vue";
import UiSplitPane from "../components/ui/UiSplitPane.vue";
import LogicPage from "./LogicPage.vue";
import SettingsModal from "../components/SettingsModal.vue";
import AiModal from "../components/AiModal.vue";
import { matchesShortcut } from "../editor/keys";
import { aiActions, aiState } from "../store/ai";
import {
  actions,
  activePane,
  aiEnabled,
  buildPrompt,
  canBuild,
  checkDebounce,
  closeKey,
  editorKeys,
  isLocked,
  newEditorKey,
  panKey,
  state,
  type EditorPane,
} from "../store/workspace";

const split = ref(0.44);
const aiOn = computed(() => aiEnabled());
const logicSplit = ref(0.6);
const createOpen = ref(false);
const deleteOpen = ref(false);
const deleteTarget = ref("");

const isLogic = computed(() => state.boardView === "logic");

// The question belongs to a project, not to the app: ask when one appears.
watch(
  () => state.view?.label,
  (label) => {
    if (label) void nextTick(askToBuild);
  },
);
const createInitial = computed(() => {
  const path = activePane()?.detail?.path ?? "";
  const slash = path.lastIndexOf("/");
  return slash >= 0 ? path.slice(0, slash + 1) : "";
});
const deleteLabel = computed(() => deleteTarget.value || "this file");

function pickFromTree(id: string, additive: boolean) {
  if (additive) {
    void actions.toggleSelected(id);
    return;
  }
  void actions.select(id, { focus: true });
}

function pickFromGraph(id: string, additive: boolean) {
  if (additive) void actions.toggleSelected(id);
  else void actions.select(id);
}

/** The editor's prompt: an AI session seeded with the node being edited. */
async function askAboutNode(text: string) {
  const detail = activePane()?.detail ?? null;
  if (!detail) return;
  const title = detail.name ?? detail.label;
  const id = await aiActions.start("editor", title, [detail.id]);
  if (id) await aiActions.send(text, id);
}

async function onSaveSettings(values: [string, string][]) {
  if (await actions.saveSettings(values)) actions.setSettingsOpen(false);
}

/** `action` rows in settings: only the Lean installer has one. */
function onSettingAction(id: string) {
  if (id === "lean.action.install") void actions.installLean();
}

/** The build question, asked once per project unless the answer is remembered. */
const buildQuestion = ref(false);
const buildRemember = ref(false);

/** Shows the build question when a project opens, if it can be built. */
function askToBuild() {
  if (!canBuild()) return;
  const prompt = buildPrompt();
  if (prompt === "always") {
    void actions.buildLean();
    return;
  }
  if (prompt === "never") return;
  buildQuestion.value = true;
}

async function answerBuild(build: boolean) {
  buildQuestion.value = false;
  if (buildRemember.value) {
    await actions.setBuildPrompt(build ? "always" : "never");
  }
  if (build) await actions.buildLean();
}

const emptyConsole = computed(() =>
  canBuild()
    ? "press build to run `lake build`; its output streams here"
    : "this project has no lakefile, or Lean is not set up — see Settings → Lean",
);

/** Keeps the console scrolled to the newest line. */
const consoleBox = ref<HTMLElement | null>(null);
watch(
  () => state.build?.lines.length ?? 0,
  async () => {
    await nextTick();
    const box = consoleBox.value;
    if (box) box.scrollTop = box.scrollHeight;
  },
);

/** One line describing the toolchain, for the header badge. */
const leanBadge = computed(() => {
  const lean = state.lean;
  if (!lean) return null;
  if (!lean.enabled) return { tone: "muted", text: "lean off" };
  if (!lean.available) return { tone: "error", text: "no lean" };
  if (lean.error) return { tone: "error", text: "lean error" };
  const version = lean.version.match(/version ([\w.+-]+)/)?.[1] ?? "";
  return { tone: "ok", text: `lean ${version || lean.origin}` };
});

/** True while any AI session is waiting on a model. */
const aiRunning = computed(() => aiState.sessions.some((session) => session.running));

/** What the header says while Lean is still working through the project. */
const processingLabel = computed(() => {
  const files = state.processingFiles;
  if (!state.processing) return null;
  if (files.length === 0) return "checking…";
  if (files.length === 1) return `checking ${files[0]}`;
  return `checking ${files.length} files`;
});

/** True when the file open in the modal is one Lean is still working on. */
/** True while Lean is still checking the file a pane shows. */
function paneProcessing(pane: EditorPane) {
  const path = pane.detail?.path;
  return Boolean(path && state.processingFiles.includes(path));
}

/** The editor's settings, as the settings page spells them. */
const debounce = computed(() => checkDebounce());
const keys = computed(() => editorKeys());

/** Double-click on a node: another modal when the modifier is held. */
function openFromBoard(id: string, event?: Event) {
  void actions.openNode(id, { fresh: freshRequested(event) });
}

/**
 * True when the click carries the "open another editor" modifier
 * (`Settings → Editor → Keyboard Shortcuts`).
 */
function freshRequested(event?: Event): boolean {
  if (!event) return false;
  const mouse = event as MouseEvent;
  return matchesShortcut(
    {
      key: "Control",
      ctrlKey: Boolean(mouse.ctrlKey),
      metaKey: Boolean(mouse.metaKey),
      altKey: Boolean(mouse.altKey),
      shiftKey: Boolean(mouse.shiftKey),
    },
    newEditorKey(),
  );
}

/** Held while dragging a board to look around, from settings. */
const pan = computed(() => panKey());

/**
 * Ctrl+W closes the editor the user is working in — or asks first, when it holds
 * an unsaved edit. Escape deliberately does nothing: it is a key pressed while
 * thinking, not a decision to close anything.
 */
function onWindowKey(event: KeyboardEvent) {
  if (!state.editors.length) return;
  if (!matchesShortcut(event, closeKey())) return;
  event.preventDefault();
  event.stopPropagation();
  const pane = activePane() ?? state.editors[state.editors.length - 1];
  if (pane) actions.requestClosePane(pane.id);
}

onMounted(() => window.addEventListener("keydown", onWindowKey, { capture: true }));
onBeforeUnmount(() => window.removeEventListener("keydown", onWindowKey, { capture: true }));

/**
 * Work that has been running long enough to be worth a spinner.
 *
 * Some of it cannot be made short — re-reading the project after a save, or Lean
 * checking a file — so the editor says it is happening instead of looking stuck.
 */
const slowBusy = ref<string | null>(null);
let busyTimer: number | undefined;
watch(
  () => state.busy,
  (busy) => {
    window.clearTimeout(busyTimer);
    if (!busy) {
      slowBusy.value = null;
      return;
    }
    // Anything quicker than this would only flicker.
    busyTimer = window.setTimeout(() => {
      slowBusy.value = busy;
    }, 250);
  },
);
onBeforeUnmount(() => window.clearTimeout(busyTimer));

function toggleGraph(id: string) {
  void actions.toggleGraph(id);
}

function askCreate() {
  createOpen.value = true;
}

function askDelete(path: string) {
  if (!path) return;
  deleteTarget.value = path;
  deleteOpen.value = true;
}

async function confirmCreate(path: string) {
  createOpen.value = false;
  await actions.createFile(path);
}

async function confirmDelete() {
  deleteOpen.value = false;
  await actions.deleteFile(deleteTarget.value);
}

async function exportZip() {
  const path = await saveDialog({
    title: "Export project as zip",
    defaultPath: `${state.view?.label ?? "project"}.zip`,
    filters: [{ name: "Zip archive", extensions: ["zip"] }],
  });
  if (path) await actions.exportZip(path);
}

async function savePrinted() {
  const path = await saveDialog({
    title: "Save printed Lean",
    defaultPath: `${state.view?.label ?? "project"}-outline.lean`,
    filters: [{ name: "Lean source", extensions: ["lean"] }],
  });
  if (path) await actions.saveOutline(path);
}

async function copyPrinted() {
  if (state.printed) await navigator.clipboard.writeText(state.printed.text);
}
</script>

<template>
  <div class="board">
    <SettingsModal
      :open="state.settingsOpen"
      :tree="state.settings"
      root-id="editor"
      :lean="state.lean"
      :install="state.install"
      @close="actions.setSettingsOpen(false)"
      @save="onSaveSettings"
      @refresh="actions.refreshLean()"
      @action="onSettingAction"
    />

    <header class="board__bar">
      <div class="board__brand">
        <span class="board__glyph">∀</span>
        <span class="board__name">KLEAN</span>
      </div>

      <span class="board__divider" />

      <div class="board__project">
        <span class="board__label">{{ state.view?.label ?? "—" }}</span>
        <span class="board__backend">{{ state.view?.backend ?? "" }}</span>
      </div>

      <div class="board__stats k-mono">
        <span>{{ state.view?.stats.libs ?? 0 }} libs</span>
        <span class="board__sep">·</span>
        <span>{{ state.view?.stats.files ?? 0 }} files</span>
        <span class="board__sep">·</span>
        <span>{{ state.view?.stats.nodes ?? 0 }} nodes</span>
        <template v-if="isLogic">
          <span class="board__sep">·</span>
          <span>{{ state.logic?.links.length ?? 0 }} uses</span>
        </template>
      </div>

      <span class="k-spacer" />

      <div class="board__actions">
        <UiButton
          size="sm"
          variant="ghost"
          class="board__ai"
          title="AI sessions"
          @click="aiActions.show()"
        >
          <UiIcon name="ai" :size="12" />
          AI
          <span v-if="aiRunning" class="board__ai-dot" />
        </UiButton>
        <button
          v-if="leanBadge"
          class="board__lean k-mono"
          type="button"
          :data-tone="leanBadge.tone"
          :title="state.lean?.error ?? state.lean?.lean ?? 'Lean toolchain'"
          @click="actions.setSettingsOpen(true)"
        >
          <UiSpinner v-if="state.lean?.building" :size="9" />
          {{ leanBadge.text }}
        </button>
        <button
          v-if="processingLabel"
          class="board__processing k-mono"
          type="button"
          :title="`${processingLabel} — open the console to follow it`"
          @click="actions.showBuild()"
        >
          <UiSpinner :size="9" />
          {{ processingLabel }}
        </button>
        <UiButton
          v-if="canBuild()"
          size="sm"
          variant="ghost"
          title="Build the project (`lake build`), with the output in a console"
          @click="state.build ? actions.showBuild() : actions.buildLean()"
        >
          <UiSpinner v-if="state.busy === 'lean'" :size="10" />
          <UiIcon v-else name="refresh" :size="11" />
          build
        </UiButton>
        <UiButton
          size="sm"
          variant="ghost"
          title="Re-read the project from storage"
          @click="actions.refresh()"
        >
          <UiSpinner v-if="state.busy === 'refresh'" :size="10" />
          <UiIcon v-else name="refresh" :size="11" />
          refresh
        </UiButton>
        <UiButton
          size="sm"
          variant="ghost"
          title="Print the project as human-readable Lean"
          @click="actions.showOutline()"
        >
          <UiIcon name="outline" :size="11" />
          outline
        </UiButton>
        <UiButton
          size="sm"
          variant="ghost"
          title="Export the workspace as a zip archive"
          @click="exportZip"
        >
          <UiIcon name="upload" :size="11" />
          export
        </UiButton>
        <UiButton size="sm" variant="ghost" title="Close the project" @click="actions.close()">
          <UiIcon name="close" :size="11" />
        </UiButton>
      </div>
    </header>

    <div
      v-if="state.error || state.notice"
      class="board__ticker"
      :data-tone="state.error ? 'error' : 'notice'"
    >
      <span>{{ state.error ?? state.notice }}</span>
      <button type="button" @click="state.error ? actions.dismissError() : actions.dismissNotice()">
        <UiIcon name="close" :size="10" />
      </button>
    </div>

    <div class="board__body">
      <UiSplitPane v-if="!isLogic" v-model:left="split" class="board__split">
        <template #left>
          <GraphCanvas
            :nodes="state.graph?.nodes ?? []"
            :links="state.graph?.links ?? []"
            :selected-ids="state.selectedIds"
            :status-kinds="['lib', 'folder', 'file']"
            :camera="state.graph?.camera ?? null"
            :hover-info="state.hover"
            @pick="pickFromGraph"
            :pan-key="pan"
            @camera="actions.saveCamera('ast', $event)"
            @settings="actions.setSettingsOpen(true)"
            @hover="actions.loadHover($event)"
            @bend="(key, value) => actions.saveLinkBend('ast', key, value)"
            @move="actions.savePositions('ast', $event)"
            @open="(id: string, event: Event) => openFromBoard(id, event)"
            @select="actions.selectMany($event)"
            @toggle="toggleGraph"
          />
        </template>
        <template #right>
          <LeanTreePanel
            :tree="state.view?.tree ?? []"
            :selected-ids="state.selectedIds"
            :files="state.view?.stats.files ?? 0"
            :nodes="state.view?.stats.nodes ?? 0"
            @pick="pickFromTree"
            @open="(id: string, event: Event) => openFromBoard(id, event)"
            @select-many="actions.selectMany($event)"
            @create="askCreate"
            @delete="askDelete"
          />
        </template>
      </UiSplitPane>

      <LogicPage
        v-else
        v-model:split="logicSplit"
        :logic="state.logic"
        :selected-ids="state.selectedIds"
        @open="(id: string, event: Event) => openFromBoard(id, event)"
      />
    </div>

    <UiSnackbar :mode="state.boardView" @change="actions.setBoardView($event)" />

    <AiModal />

    <!-- One modal per open node: Ctrl+double-click opens another, a plain
         double-click retargets the one the user last touched. -->
    <UiModal
      v-for="(pane, index) in state.editors"
      :key="pane.id"
      :open="true"
      :title="pane.detail?.name ?? pane.detail?.label ?? ''"
      :badge="pane.detail?.kind ?? ''"
      :subtitle="pane.detail?.path ?? ''"
      :width="640"
      :height="470"
      :resizable="true"
      :layer="index"
      :active="pane.id === state.activeEditor"
      :geometry="pane.geometry"
      :close-on-escape="false"
      @close="actions.requestClosePane(pane.id)"
      @focus="actions.focusPane(pane.id)"
      @geometry="(geometry: { x: number; y: number; w: number; h: number }) =>
        actions.setPaneGeometry(pane.id, geometry)"
    >
      <div v-if="pane.confirming" class="board__warn">
        <UiIcon name="alert" :size="12" />
        <span>discard the unsaved edit?</span>
        <UiButton size="sm" variant="ghost" @click="actions.keepEditing(pane.id)">keep editing</UiButton>
        <UiButton size="sm" variant="solid" @click="actions.discardAndClose(pane.id)">discard</UiButton>
      </div>
      <NodeEditor
        :detail="pane.detail"
        :ai-enabled="aiOn"
        :saving="pane.saving"
        :error="pane.error"
        :goals="pane.goals"
        :goals-busy="pane.goalsBusy"
        :term="pane.term"
        :term-busy="pane.termBusy"
        :multi="pane.multi"
        :node-goals="pane.nodeGoals"
        :locked="isLocked(pane)"
        :busy="slowBusy"
        :processing="paneProcessing(pane)"
        :debounce="debounce"
        :loading="pane.loading"
        :keys="keys"
        @preview="(text: string) => actions.previewText(pane.id, text)"
        @save="(text: string) => actions.applyText(pane.id, text)"
        @dirty="(value: boolean) => actions.setPaneDirty(pane.id, value)"
        @selections="(count: number) => actions.setPaneSelections(pane.id, count)"
        @revert="actions.discardPane(pane.id)"
        @comments="(text: string) => actions.applyComments(pane.id, text)"
        @prompt="askAboutNode"
        @refresh-goals="void actions.loadPaneGoals(pane.id)"
        @goals="(line: number, character: number) => {
          void actions.loadPaneGoals(pane.id, line, character);
          void actions.loadPaneTerm(pane.id, line, character);
        }"
        @file="actions.openFilePane(pane.id)"
      />
    </UiModal>

    <UiModal
      :open="state.printedOpen"
      :title="state.printed?.title ?? 'file'"
      :subtitle="state.printed?.subtitle ?? ''"
      :width="900"
      :height="640"
      :resizable="true"
      @close="actions.closePrinted()"
    >
      <PrintedViewer
        :printed="state.printed"
        @copy="copyPrinted"
        @save="savePrinted"
        @close="actions.closePrinted()"
      />
    </UiModal>

    <UiModal
      :open="buildQuestion"
      title="build this project?"
      :subtitle="state.view?.label ?? ''"
      :width="440"
      :height="180"
      @close="answerBuild(false)"
    >
      <div class="build-ask">
        <p class="build-ask__text">
          KLEAN can run <code>lake build</code> so imports resolve for the parser and the
          language server. It runs in the background with a memory ceiling, and its output
          streams into a console.
        </p>
        <label class="build-ask__remember">
          <input v-model="buildRemember" type="checkbox" />
          don't ask again for this project
        </label>
        <div class="build-ask__actions">
          <UiButton size="sm" variant="ghost" @click="answerBuild(false)">skip</UiButton>
          <UiButton size="sm" variant="solid" @click="answerBuild(true)">build now</UiButton>
        </div>
      </div>
    </UiModal>

    <UiModal
      :open="state.buildOpen"
      title="build console"
      :subtitle="state.build?.summary ?? state.build?.command ?? 'lake build'"
      :width="820"
      :height="520"
      :resizable="true"
      @close="actions.setBuildOpen(false)"
    >
      <div class="console">
        <div class="console__bar">
          <span class="k-mono console__state" :data-running="state.build?.running || undefined">
            {{ state.build?.running ? `running · ${state.build.seconds}s` : state.build?.summary ?? "no build yet" }}
          </span>
          <span class="k-spacer" />
          <UiButton
            v-if="state.build?.running"
            size="sm"
            variant="ghost"
            @click="actions.cancelBuild()"
          >
            <UiIcon name="close" :size="11" /> cancel
          </UiButton>
          <UiButton
            v-else
            size="sm"
            variant="solid"
            title="Run `lake build`"
            @click="actions.buildLean()"
          >
            <UiIcon name="refresh" :size="11" /> build
          </UiButton>
        </div>
        <pre ref="consoleBox" class="console__output k-mono">{{
          (state.build?.lines ?? []).join("\n") || emptyConsole
        }}</pre>
      </div>
    </UiModal>

    <UiPrompt
      :open="createOpen"
      title="New Lean file"
      label="path (relative to the project root)"
      :initial="createInitial"
      placeholder="Core/NewFile.lean"
      confirm-label="create"
      @cancel="createOpen = false"
      @confirm="confirmCreate"
    />

    <UiPrompt
      :open="deleteOpen"
      title="Delete file"
      :message="`Delete ${deleteLabel}? This removes the file from disk.`"
      confirm-label="delete"
      :danger="true"
      @cancel="deleteOpen = false"
      @confirm="confirmDelete"
    />
  </div>
</template>

<style scoped>
.console {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  gap: 6px;
}

.console__bar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.console__state {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.console__state[data-running] {
  color: var(--accent);
}

/* The editor's unsaved-changes row: the settings modal's, in the same colours. */
.board__warn {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 12px 8px;
  padding: 6px 8px;
  border: 1px solid rgba(194, 166, 112, 0.36);
  border-radius: var(--radius);
  background: var(--warning-soft);
  color: var(--warning);
  font-size: var(--fs-xs);
}

.board__warn span:first-of-type {
  flex: 1;
}

.console__output {
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 8px 10px;
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #0b0d0f;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
}

.build-ask {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 2px 2px 0;
}

.build-ask__text {
  margin: 0;
  color: var(--text-faint);
  font-size: var(--fs-xs);
  line-height: 1.6;
}

.build-ask__text code {
  color: var(--text-dim);
  font-family: var(--mono);
}

.build-ask__remember {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.build-ask__actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.board {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg-0);
}

.board__bar {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 38px;
  padding: 0 10px;
  border-bottom: 1px solid var(--line);
  background: var(--grad-header);
}

.board__brand {
  display: flex;
  align-items: center;
  gap: 7px;
}

.board__glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 1px solid var(--accent-line);
  border-radius: 2px;
  background: var(--grad-accent);
  color: var(--accent-bright);
  font-family: var(--mono);
  font-size: 10px;
}

.board__name {
  font-size: var(--fs-sm);
  letter-spacing: 0.26em;
  color: var(--text-dim);
}

.board__divider {
  width: 1px;
  height: 16px;
  background: var(--line);
}

.board__project {
  display: flex;
  align-items: baseline;
  gap: 7px;
  min-width: 0;
}

.board__label {
  font-size: var(--fs-md);
  color: var(--text);
  max-width: 34ch;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.board__backend {
  font-family: var(--mono);
  font-size: var(--fs-micro);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-ghost);
  border: 1px solid var(--line);
  border-radius: 2px;
  padding: 1px 4px;
}

.board__stats {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.board__sep {
  color: var(--line-strong);
}

.board__actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.board__ai {
  gap: 4px;
}

.board__ai-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent);
  animation: ai-pulse 1.1s ease-in-out infinite;
}

@keyframes ai-pulse {
  0%,
  100% {
    opacity: 0.35;
  }
  50% {
    opacity: 1;
  }
}

.board__processing {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 22ch;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding: 0;
  border: none;
  background: none;
  color: var(--accent);
  font-size: var(--fs-micro);
  cursor: pointer;
}

.board__processing:hover {
  color: var(--accent-bright);
  text-decoration: underline;
}

.board__lean {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 20px;
  padding: 0 6px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  transition: border-color var(--t) var(--ease), color var(--t) var(--ease);
}

.board__lean:hover {
  border-color: var(--line-strong);
  color: var(--text-faint);
}

.board__lean[data-tone="ok"] {
  color: var(--ok);
  border-color: rgba(120, 170, 130, 0.25);
}

.board__lean[data-tone="error"] {
  color: var(--danger);
  border-color: rgba(194, 112, 112, 0.3);
}

.board__ticker {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  font-size: var(--fs-xs);
  border-bottom: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.015);
  color: var(--text-faint);
}

.board__ticker[data-tone="error"] {
  background: var(--danger-soft);
  color: var(--danger);
}

.board__ticker button {
  margin-left: auto;
  display: flex;
  color: inherit;
  opacity: 0.6;
  transition: opacity var(--t) var(--ease);
}

.board__ticker button:hover {
  opacity: 1;
}

.board__body {
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
}

.board__split {
  flex: 1;
  min-height: 0;
}
</style>
