<script setup lang="ts">
// Project shell: AST board (graph + tree) or Logic board, with the shared editor
// modal and the bottom view switcher.
import { computed, ref } from "vue";
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
import { actions, aiEnabled, state } from "../store/workspace";

const split = ref(0.44);
const aiOn = computed(() => aiEnabled());
const logicSplit = ref(0.6);
const createOpen = ref(false);
const deleteOpen = ref(false);
const deleteTarget = ref("");

const isLogic = computed(() => state.boardView === "logic");
const createInitial = computed(() => {
  const path = state.detail?.path ?? "";
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

async function onSaveSettings(values: [string, string][]) {
  if (await actions.saveSettings(values)) actions.setSettingsOpen(false);
}

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
      :root-id="state.boardView === 'logic' ? 'logic' : 'ast'"
      @close="actions.setSettingsOpen(false)"
      @save="onSaveSettings"
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
            @camera="actions.saveCamera('ast', $event)"
            @settings="actions.setSettingsOpen(true)"
            @hover="actions.loadHover($event)"
            @bend="(key, value) => actions.saveLinkBend('ast', key, value)"
            @move="actions.savePositions('ast', $event)"
            @open="actions.openNode($event)"
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
            @open="actions.openNode($event)"
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
        @open="actions.openNode($event)"
      />
    </div>

    <UiSnackbar :mode="state.boardView" @change="actions.setBoardView($event)" />

    <UiModal
      :open="state.editorOpen"
      :title="state.detail?.name ?? state.detail?.label ?? ''"
      :badge="state.detail?.kind ?? ''"
      :subtitle="state.detail?.path ?? ''"
      :width="640"
      :height="470"
      :resizable="true"
      @close="actions.closeEditor()"
    >
      <NodeEditor
        :detail="state.detail"
        :ai-enabled="aiOn"
        :saving="state.saving"
        :save-error="state.saveError"
        @apply="actions.applyText"
        @comments="actions.applyComments"
        @rename="actions.rename"
        @file="actions.showFile()"
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
