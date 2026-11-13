<script setup lang="ts">
// `#check`, with the whole project in scope.
//
// The field is the term; the button is what asks. The answer is Lean's own — the
// very same call the AI's `lean_check` tool makes, so `lake env lean` runs as a
// separate process outside the project lock and can take seconds. That is why it is
// a button and not a keystroke: nothing is started until the user asks for it.
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import * as api from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiCard from "./ui/UiCard.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiModal from "./ui/UiModal.vue";
import UiSpinner from "./ui/UiSpinner.vue";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const term = ref("");
/** The field: it takes the caret as the modal opens, so typing can start at once. */
const field = ref<HTMLInputElement | null>(null);
/** What Lean answered, split into lines for the card. */
const answer = ref<string[]>([]);
/** Set when the check could not run at all (no build output, no toolchain). */
const error = ref<string | null>(null);
const busy = ref(false);
/** The term the answer belongs to, so the card can say what was checked. */
const checked = ref("");
/** Bumped per run: a slower answer never replaces a newer one. */
let seq = 0;

async function run() {
  const expression = term.value.trim();
  if (expression === "" || busy.value) return;
  const mine = (seq += 1);
  busy.value = true;
  error.value = null;
  try {
    const text = await api.checkProject(expression);
    if (mine !== seq) return;
    checked.value = expression;
    answer.value = text.split("\n").filter((line) => line.trim() !== "");
  } catch (problem) {
    if (mine !== seq) return;
    checked.value = expression;
    answer.value = [];
    error.value = problem instanceof Error ? problem.message : String(problem);
  } finally {
    if (mine === seq) busy.value = false;
  }
}

/** A line of Lean's answer is a complaint when `check.rs` said so. */
function failed(line: string): boolean {
  return line.startsWith("error");
}

watch(
  () => props.open,
  (open) => {
    if (open) void nextTick(() => field.value?.focus());
  },
);

onBeforeUnmount(() => {
  // A run in flight is left to finish: it holds nothing the app needs back.
  seq += 1;
});
</script>

<template>
  <UiModal
    :open="open"
    title="Check"
    subtitle="#check, with the whole project in scope"
    :width="620"
    :height="420"
    :resizable="true"
    @close="emit('close')"
  >
    <div class="check">
      <div class="check__bar">
        <div class="check__field">
          <span class="check__glyph" aria-hidden="true">∀</span>
          <input
            ref="field"
            v-model="term"
            class="check__input k-mono"
            type="text"
            spellcheck="false"
            placeholder="a term to #check, e.g. Nat.add"
            @keydown.enter.prevent="run()"
          />
        </div>
        <UiButton
          size="sm"
          variant="solid"
          :disabled="busy || term.trim() === ''"
          @click="run()"
        >
          <UiSpinner v-if="busy" :size="10" />
          <UiIcon v-else name="check" :size="11" />
          Check
        </UiButton>
      </div>

      <UiCard class="check__card" padding="none">
        <div v-if="error" class="check__error">
          <UiIcon name="alert" :size="11" />
          <span>{{ error }}</span>
        </div>
        <template v-else-if="answer.length > 0">
          <div class="check__head k-mono">#check {{ checked }}</div>
          <ul class="check__list">
            <li
              v-for="(line, index) in answer"
              :key="index"
              class="check__row"
              :data-tone="failed(line) ? 'error' : 'info'"
            >
              <span class="check__dot" aria-hidden="true" />
              <span class="check__answer k-mono">{{ line }}</span>
            </li>
          </ul>
        </template>
        <p v-else-if="busy" class="check__empty">asking Lean…</p>
        <p v-else class="check__empty">
          nothing checked yet — type a term, or a name from the project
        </p>
      </UiCard>
    </div>
  </UiModal>
</template>

<style scoped>
.check {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
  min-height: 0;
  padding: 12px 14px 14px;
}

.check__bar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.check__field {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 6px 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
}

.check__field:focus-within {
  border-color: var(--line-strong);
}

.check__glyph {
  color: var(--fg-3);
  font-size: var(--fs-md);
  line-height: 1;
}

.check__input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--fg-1);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.check__card {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.check__head {
  padding: 8px 12px;
  border-bottom: 1px solid var(--line);
  color: var(--fg-3);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
  word-break: break-word;
}

.check__list {
  display: flex;
  flex-direction: column;
  margin: 0;
  padding: 4px 0;
  list-style: none;
}

.check__row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--line);
}

.check__row:last-child {
  border-bottom: 0;
}

.check__dot {
  width: 5px;
  height: 5px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--accent);
}

.check__row[data-tone="error"] .check__dot {
  background: var(--danger);
}

.check__answer {
  color: var(--fg-1);
  font-size: var(--fs-sm);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}

.check__row[data-tone="error"] .check__answer {
  color: var(--danger);
}

.check__empty,
.check__error {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  padding: 12px;
  color: var(--fg-3);
  font-size: var(--fs-sm);
}

.check__error {
  color: var(--danger);
}
</style>
