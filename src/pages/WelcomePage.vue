<script setup lang="ts">
// Welcome screen: one centred card asking for a Lean project root.
import { ref } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import UiButton from "../components/ui/UiButton.vue";
import UiCard from "../components/ui/UiCard.vue";
import UiIcon from "../components/ui/UiIcon.vue";
import UiSpinner from "../components/ui/UiSpinner.vue";
import type { RecentProject } from "../api/lean";
import { actions, state } from "../store/workspace";

const busy = ref<"folder" | "zip" | "memory" | "recent" | null>(null);

/** Opens a project from the recent list, with the same feedback as the buttons. */
async function openRecent(entry: RecentProject) {
  if (busy.value !== null) return;
  busy.value = "recent";
  try {
    await actions.openRecent(entry);
  } finally {
    busy.value = null;
  }
}

async function pickFolder() {
  busy.value = "folder";
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Select the root of a Lean project",
    });
    if (typeof selected === "string") await actions.openFolder(selected);
  } finally {
    busy.value = null;
  }
}

async function pickZip() {
  busy.value = "zip";
  try {
    const selected = await openDialog({
      multiple: false,
      title: "Import a project archive",
      filters: [{ name: "Zip archive", extensions: ["zip"] }],
    });
    if (typeof selected === "string") await actions.openZip(selected);
  } finally {
    busy.value = null;
  }
}

async function newMemory() {
  busy.value = "memory";
  try {
    await actions.newMemory("workspace");
  } finally {
    busy.value = null;
  }
}
</script>

<template>
  <div class="welcome">
    <UiCard class="welcome__card" padding="lg" tone="raised">
      <div class="welcome__mark">
        <span class="welcome__glyph">∀</span>
        <h1 class="welcome__title">KLEAN</h1>
      </div>
      <p class="welcome__tagline">
        A visual editor for Lean projects.<br />
        Parse the syntax tree of a library, browse it as a board, rewrite it in place.
      </p>

      <div class="welcome__actions">
        <UiButton variant="solid" :disabled="busy !== null" @click="pickFolder">
          <UiSpinner v-if="busy === 'folder'" :size="11" />
          <UiIcon v-else name="folder" :size="12" />
          choose a project folder
        </UiButton>
        <UiButton variant="outline" :disabled="busy !== null" @click="pickZip">
          <UiSpinner v-if="busy === 'zip'" :size="11" />
          <UiIcon v-else name="download" :size="12" />
          import zip
        </UiButton>
        <UiButton variant="ghost" :disabled="busy !== null" @click="newMemory">
          <UiSpinner v-if="busy === 'memory'" :size="11" />
          <UiIcon v-else name="layers" :size="12" />
          try an in-memory workspace
        </UiButton>
      </div>

      <div v-if="state.recents.length" class="welcome__recents">
        <div class="welcome__recents-head">
          <span class="k-label">recent</span>
          <span class="welcome__recents-hint">last {{ state.recents.length }} opened</span>
        </div>
        <button
          v-for="entry in state.recents"
          :key="entry.path"
          class="welcome__recent"
          type="button"
          :title="entry.path"
          :disabled="busy !== null"
          @click="openRecent(entry)"
        >
          <UiSpinner v-if="busy === 'recent'" :size="11" />
          <UiIcon v-else :name="entry.kind === 'zip' ? 'archive' : 'folder'" :size="11" />
          <span class="welcome__recent-label">{{ entry.label }}</span>
          <span class="welcome__recent-path k-mono">{{ entry.path }}</span>
        </button>
      </div>

      <div class="welcome__hint">
        <span class="k-label">root</span>
        <span class="welcome__hint-text">
          a folder with a <code>lakefile.toml</code>, or any folder of <code>.lean</code> files
        </span>
      </div>

      <p v-if="state.error" class="welcome__error">{{ state.error }}</p>
    </UiCard>

    <footer class="welcome__foot">
      <span>every file operation goes through rust-vfs</span>
      <span class="welcome__dot">·</span>
      <span>disk on desktop, memory on the web</span>
    </footer>
  </div>
</template>

<style scoped>
.welcome {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: 22px;
  background:
    radial-gradient(900px 420px at 50% 12%, rgba(106, 169, 196, 0.06), transparent 70%),
    radial-gradient(700px 380px at 50% 100%, rgba(255, 255, 255, 0.014), transparent 70%),
    var(--bg-0);
}

.welcome__card {
  width: min(520px, calc(100vw - 64px));
  display: flex;
  flex-direction: column;
  gap: 18px;
  animation: welcome-in 320ms var(--ease) both;
}

.welcome__mark {
  display: flex;
  align-items: center;
  gap: 11px;
}

.welcome__glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: 1px solid var(--accent-line);
  border-radius: var(--radius);
  background: var(--grad-accent);
  color: var(--accent-bright);
  font-size: 15px;
  font-family: var(--mono);
}

.welcome__title {
  margin: 0;
  font-size: 19px;
  font-weight: 500;
  letter-spacing: 0.34em;
  color: var(--text);
}

.welcome__tagline {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-md);
  line-height: 1.6;
  max-width: 52ch;
}

.welcome__actions {
  display: flex;
  flex-direction: column;
  gap: 7px;
  align-items: stretch;
  margin-top: 2px;
}

.welcome__actions :deep(.ui-button) {
  height: 30px;
  justify-content: flex-start;
  padding: 0 12px;
}

.welcome__recents {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-top: 12px;
  border-top: 1px solid var(--line-soft);
}

.welcome__recents-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  padding-bottom: 3px;
}

.welcome__recents-hint {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.welcome__recent {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 24px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: 2px;
  color: var(--text-dim);
  text-align: left;
  transition:
    background-color var(--t) var(--ease),
    border-color var(--t) var(--ease),
    color var(--t) var(--ease);
}

.welcome__recent:hover {
  background: var(--bg-hover);
  border-color: var(--line);
  color: var(--text);
}

.welcome__recent-label {
  font-size: var(--fs-sm);
  white-space: nowrap;
}

.welcome__recent-path {
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-ghost);
  font-size: 10px;
  max-width: 60%;
}

.welcome__hint {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--line-soft);
}

.welcome__hint-text {
  color: var(--text-faint);
  font-size: var(--fs-xs);
}

.welcome__hint code {
  font-family: var(--mono);
  font-size: 10px;
  color: var(--text-faint);
}

.welcome__error {
  margin: 0;
  padding: 7px 9px;
  border: 1px solid rgba(201, 115, 107, 0.28);
  border-radius: 2px;
  background: var(--danger-soft);
  color: var(--danger);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
}

.welcome__foot {
  display: flex;
  gap: 8px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.welcome__dot {
  color: var(--line-strong);
}

@keyframes welcome-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}
</style>
