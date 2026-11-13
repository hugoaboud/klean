<script setup lang="ts">
// The splash screen: the two things that cost seconds, paid before the first
// click instead of after it.
//
// Lean's parser environment takes ~20 s to build the first time (Lean imports its
// own `Init` and `Lean` modules), and Monaco is a large module to fetch and parse.
// Both are started the moment the app opens and reported here, so the wait is
// explained rather than looking like a hang — and opening a project afterwards is
// instantaneous.
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import * as api from "../api/lean";
import { loadMonaco } from "../editor/monaco";
import { actions } from "../store/workspace";
import UiSpinner from "../components/ui/UiSpinner.vue";

/** The backend's two steps plus the editor's own. */
const TOTAL = 3;

const startedAt = Date.now();

const lean = ref<api.StartupState>({
  label: "starting",
  done: 0,
  total: TOTAL,
  finished: false,
  error: null,
  seconds: 0,
});
const editorReady = ref(false);
const editorLabel = ref("loading the editor");

let timer: number | undefined;

/** Steps finished: Lean's toolchain, Lean's parser, the editor. */
const done = computed(() => lean.value.done + (editorReady.value ? 1 : 0));

const percent = computed(() => {
  const steps = Math.min(TOTAL, done.value);
  // Never 0 and never 100 before the app is actually ready: a bar that sits at
  // either end reads as broken.
  return Math.max(4, Math.min(100, Math.round((steps / TOTAL) * 100)));
});

const label = computed(() => {
  if (lean.value.error) return "Lean is not available — KLEAN will start without it";
  if (!lean.value.finished) return lean.value.label;
  if (!editorReady.value) return editorLabel.value;
  return "ready";
});

const seconds = computed(() => lean.value.seconds);

/** Polls the backend's warm-up; the editor loads in parallel. */
async function watchLean() {
  const state = await api.startupState().catch(() => null);
  if (state) lean.value = state;
  // The bar has to keep moving even if the backend never reports again.
  if (!lean.value.finished && Date.now() - startedAt < 90_000) {
    timer = window.setTimeout(() => void watchLean(), 220);
  }
}

onMounted(() => {
  void watchLean();
  void (async () => {
    const started = Date.now();
    try {
      await loadMonaco();
    } catch {
      // Without Monaco the editor falls back to nothing rather than blocking the
      // whole app: the splash still has to end.
    }
    // The chunk is cached after this, but a first paint under 120 ms would only
    // flash the screen.
    const spent = Date.now() - started;
    if (spent < 120) await new Promise((resolve) => window.setTimeout(resolve, 120 - spent));
    editorReady.value = true;
    await waitForLean();
    await actions.finishStartup();
  })();
});

/**
 * Holds the splash until Lean's parser is warm, so the first project is instant.
 *
 * It gives up after a minute and a half: a warm-up that never finishes is a
 * problem the project itself will report, and the app is no use stuck here.
 */
async function waitForLean() {
  const deadline = Date.now() + 90_000;
  while (!lean.value.finished && Date.now() < deadline) {
    await new Promise((resolve) => window.setTimeout(resolve, 120));
    const state = await api.startupState().catch(() => null);
    if (state) lean.value = state;
  }
}

onBeforeUnmount(() => window.clearTimeout(timer));
</script>

<template>
  <div class="splash">
    <div class="splash__mark">
      <span class="splash__glyph">∀</span>
      <span class="splash__name">KLEAN</span>
    </div>

    <div class="splash__bar" role="progressbar" :aria-valuenow="percent" aria-valuemin="0" aria-valuemax="100">
      <div class="splash__fill" :data-error="lean.error ? true : undefined" :style="{ width: `${percent}%` }" />
    </div>

    <div class="splash__line">
      <UiSpinner :size="10" />
      <span class="splash__label">{{ label }}</span>
      <span class="k-spacer" />
      <span class="splash__seconds k-mono">{{ seconds }}s</span>
    </div>

    <p v-if="lean.error" class="splash__error">{{ lean.error }}</p>
    <p v-else class="splash__hint">
      Lean's parser is being prepared once, now, so opening a project is instant afterwards.
    </p>
  </div>
</template>

<style scoped>
.splash {
  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  padding: 0 32px;
  background: radial-gradient(120% 90% at 50% 0%, #14181c 0%, #0a0c0e 60%, #08090b 100%);
}

.splash__mark {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 6px;
}

.splash__glyph {
  color: var(--accent);
  font-size: 34px;
  line-height: 1;
}

.splash__name {
  color: var(--text);
  font-size: 20px;
  letter-spacing: 0.22em;
}

.splash__bar {
  width: min(420px, 80vw);
  height: 4px;
  border-radius: 2px;
  background: var(--bg-0);
  border: 1px solid var(--line);
  overflow: hidden;
}

.splash__fill {
  height: 100%;
  border-radius: 2px;
  background: var(--accent);
  transition: width 220ms ease-out;
}

.splash__fill[data-error] {
  background: var(--warning);
}

.splash__line {
  display: flex;
  align-items: center;
  gap: 8px;
  width: min(420px, 80vw);
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.splash__label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.splash__seconds {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  font-variant-numeric: tabular-nums;
}

.splash__hint,
.splash__error {
  margin: 0;
  max-width: min(460px, 84vw);
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  line-height: 1.6;
  text-align: center;
}

.splash__error {
  color: var(--warning);
}
</style>
