<script setup lang="ts">
// The read of a project, while it happens.
//
// Opening a folder walks the file tree and stops there: parsing every file is what
// this modal reports, and it is the only way onto the project page. There is no
// board to show before it finishes, so closing the modal closes the project — the
// read is stopped and nothing is opened.
import { computed } from "vue";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiModal from "./ui/UiModal.vue";
import { actions, state } from "../store/workspace";

/** How far the read got, as a percentage of the files. */
const percent = computed(() => {
  const scan = state.scan;
  if (!scan || scan.total === 0) return scan && !scan.running ? 100 : 0;
  return Math.min(100, Math.round((scan.done / scan.total) * 100));
});

const counts = computed(() => {
  const scan = state.scan;
  if (!scan) return "starting…";
  if (scan.total === 0) return "no Lean files";
  return `${scan.done} / ${scan.total} files`;
});

/** The file being read, or what happened when the read is over. */
const line = computed(() => {
  const scan = state.scan;
  if (!scan) return "";
  if (scan.running) return scan.current;
  return scan.summary ?? "";
});

const elapsed = computed(() => {
  const seconds = state.scan?.seconds ?? 0;
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
});

/** True once nothing more will be read, so the bar stops pretending to move. */
const stopped = computed(() => Boolean(state.parseBlocked) || state.scan?.running === false);
</script>

<template>
  <UiModal
    :open="state.parseOpen"
    title="reading the project"
    :subtitle="state.parseLabel"
    :width="420"
    :height="230"
    @close="actions.cancelParse()"
  >
    <div class="parse">
      <div class="parse__head">
        <span class="parse__name">{{ state.parseLabel || "project" }}</span>
        <span class="k-spacer" />
        <span class="parse__count k-mono">{{ counts }}</span>
      </div>

      <!-- A linear bar: one step per file, which is what the read is made of. -->
      <div
        class="parse__bar"
        role="progressbar"
        :aria-valuenow="percent"
        aria-valuemin="0"
        aria-valuemax="100"
        :data-stalled="state.parseBlocked ? true : undefined"
      >
        <div class="parse__fill" :style="{ width: `${percent}%` }" />
      </div>

      <div class="parse__line">
        <UiIcon v-if="!stopped" name="dot" :size="9" class="parse__dot" />
        <span class="parse__file k-mono">{{ line }}</span>
        <span class="k-spacer" />
        <span class="parse__time k-mono">{{ elapsed }}</span>
      </div>

      <p v-if="state.parseBlocked" class="parse__note">
        <UiIcon name="alert" :size="11" />
        <span>{{ state.parseBlocked }}</span>
      </p>
      <p v-else class="parse__note parse__note--quiet">
        Lean's own parser reads each file once. The board is built when it is done.
      </p>
    </div>

    <template #footer>
      <template v-if="state.parseBlocked">
        <UiButton size="sm" variant="ghost" @click="actions.cancelParse()">cancel</UiButton>
        <UiButton size="sm" variant="solid" @click="actions.parseAnyway()">open anyway</UiButton>
      </template>
      <UiButton v-else size="sm" variant="ghost" @click="actions.cancelParse()">
        <UiIcon name="close" :size="11" /> stop
      </UiButton>
    </template>
  </UiModal>
</template>

<style scoped>
.parse {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
}

.parse__head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.parse__name {
  font-family: var(--mono);
  font-size: var(--fs-sm);
  color: var(--text);
}

.parse__count {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.parse__bar {
  position: relative;
  height: 6px;
  border: 1px solid var(--line);
  border-radius: 3px;
  background: var(--bg-0);
  overflow: hidden;
}

.parse__fill {
  height: 100%;
  border-radius: 2px;
  background: var(--accent);
  /* A file is a small step: the transition exists so it reads as one. */
  transition: width 160ms linear;
}

.parse__bar[data-stalled] .parse__fill {
  background: var(--warning);
}

.parse__line {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 14px;
}

.parse__dot {
  color: var(--accent);
}

.parse__file {
  overflow: hidden;
  color: var(--text-faint);
  font-size: var(--fs-micro);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.parse__time {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.parse__note {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  color: var(--warning);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.parse__note--quiet {
  color: var(--text-ghost);
  font-style: italic;
}
</style>
