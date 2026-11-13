// Monaco, loaded once and dressed as KLEAN.
//
// The editor is the heaviest module in the app, and the welcome page must stay
// idle, so it is imported on demand: the first editor modal opens it, and every
// editor after that shares the same instance.
//
// Three things are registered here:
//
// - the **Lean language**, as a Monarch tokenizer: highlighting is a lexer, and it
//   has to know Lean's comments (`--`, `/- -/`, `/-- -/`), strings, characters,
//   numbers, attributes, the keywords of the language and the tactics of its
//   proofs. Diagnostics and completions are not the lexer's business — they come
//   from Lean's own language server, which is what `CodeEditor.vue` feeds in.
// - the **theme**, so the surface, the gutter and the tokens match the rest of the
//   app instead of Monaco's dark blue.
// - the **key table**, so a shortcut written in `Settings → Editor` becomes a
//   Monaco keybinding.
import type * as Monaco from "monaco-editor";
import { parseShortcut, type Shortcut } from "./keys";
import { leanLanguage } from "./lean-language";

export type MonacoApi = typeof Monaco;

let loading: Promise<MonacoApi> | null = null;

/** Loads Monaco (once), registers Lean's language and theme, and returns it. */
export function loadMonaco(): Promise<MonacoApi> {
  if (!loading) {
    loading = (async () => {
      // The editor worker handles the model bookkeeping Monaco does off-thread.
      // Vite bundles it as its own chunk; the app links it through this hook.
      // The package's `exports` map is `./*` → `./esm/vs/*.js`, so this is the
      // worker module with Vite's `?worker` suffix.
      const worker = (await import("monaco-editor/editor/editor.worker?worker")).default;
      const environment = self as unknown as {
        MonacoEnvironment?: { getWorker: () => Worker };
      };
      environment.MonacoEnvironment = { getWorker: () => new worker() };

      const monaco = await import("monaco-editor");
      registerLean(monaco);
      registerTheme(monaco);
      // The first `editor.create` of a session is the slow one: it lays out the
      // editor's DOM, measures the font and starts the worker. Building a throwaway
      // one here — on the splash screen — is what makes the first modal appear with
      // its text already in it instead of empty for a second.
      warmEditor(monaco);
      return monaco;
    })();
  }
  return loading;
}

/** Builds and throws away one editor, so the first real one is cheap. */
function warmEditor(monaco: MonacoApi): void {
  const host = document.createElement("div");
  host.style.cssText = "position:absolute;left:-9999px;top:0;width:320px;height:120px;";
  document.body.appendChild(host);
  try {
    const model = monaco.editor.createModel("def klean_warm_up : Nat := 0\n", "lean");
    const editor = monaco.editor.create(host, {
      model,
      theme: "klean-dark",
      minimap: { enabled: false },
      automaticLayout: false,
    });
    editor.dispose();
    model.dispose();
  } catch {
    // A warm-up that fails changes nothing: the real editor reports its own problems.
  } finally {
    host.remove();
  }
}

/** Registers Lean with Monaco: a lexer, the comments and brackets it pairs. */
function registerLean(monaco: MonacoApi): void {
  if (monaco.languages.getLanguages().some((language) => language.id === "lean")) return;
  monaco.languages.register({ id: "lean", extensions: [".lean"] });
  monaco.languages.setMonarchTokensProvider("lean", leanLanguage());
  monaco.languages.setLanguageConfiguration("lean", {
    comments: { lineComment: "--", blockComment: ["/-", "-/"] },
    brackets: [
      ["{", "}"],
      ["[", "]"],
      ["(", ")"],
      ["⟨", "⟩"],
      ["⦃", "⦄"],
    ],
    autoClosingPairs: [
      { open: "{", close: "}" },
      { open: "[", close: "]" },
      { open: "(", close: ")" },
      { open: "⟨", close: "⟩" },
      { open: "⦃", close: "⦄" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    surroundingPairs: [
      { open: "(", close: ")" },
      { open: "{", close: "}" },
      { open: "[", close: "]" },
      { open: "⟨", close: "⟩" },
    ],
    wordPattern: /[A-Za-z_α-ωΑ-Ω][A-Za-z0-9_'.!?₀-₉α-ωΑ-Ω]*/,
    indentationRules: {
      increaseIndentPattern: /\b(by|where|do|then|else|match)\s*$/,
      decreaseIndentPattern: /^\s*(end\b|else\b|[}\])])/,
    },
  });
}

/** A dark theme in KLEAN's palette, so the editor does not look foreign. */
function registerTheme(monaco: MonacoApi): void {
  monaco.editor.defineTheme("klean-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "", foreground: "c8ccd2" },
      { token: "comment", foreground: "6a7078", fontStyle: "italic" },
      { token: "keyword", foreground: "9aa7d8" },
      { token: "keyword.tactic", foreground: "b8a2d6" },
      { token: "string", foreground: "9dc08b" },
      { token: "string.escape", foreground: "c9a26a" },
      { token: "number", foreground: "d0a26a" },
      { token: "type", foreground: "7fb3c9" },
      { token: "operator", foreground: "9aa3ad" },
      { token: "identifier", foreground: "c8ccd2" },
    ],
    colors: {
      "editor.background": "#0c0e10",
      "editor.foreground": "#c8ccd2",
      "editorLineNumber.foreground": "#3f454c",
      "editorLineNumber.activeForeground": "#7c848c",
      "editorGutter.background": "#0c0e10",
      "editor.lineHighlightBackground": "#ffffff08",
      "editor.selectionBackground": "#6aa9c438",
      "editor.inactiveSelectionBackground": "#6aa9c420",
      "editor.selectionHighlightBackground": "#6aa9c420",
      "editor.wordHighlightBackground": "#6aa9c426",
      "editor.wordHighlightStrongBackground": "#d6b84a26",
      "editor.findMatchBackground": "#d6b84a59",
      "editor.findMatchHighlightBackground": "#c4a83c33",
      "editorCursor.foreground": "#8fc7e0",
      "editorIndentGuide.background1": "#ffffff10",
      "editorBracketMatch.background": "#6aa9c41f",
      "editorBracketMatch.border": "#6aa9c455",
      "editorError.foreground": "#c9736b",
      "editorWarning.foreground": "#c2a670",
      "editorInfo.foreground": "#6aa9c4",
      "editorHoverWidget.background": "#14171a",
      "editorHoverWidget.border": "#2a2f35",
      "editorSuggestWidget.background": "#14171a",
      "editorSuggestWidget.border": "#2a2f35",
      "editorSuggestWidget.selectedBackground": "#6aa9c433",
      "editorWidget.background": "#14171a",
      "editorWidget.border": "#2a2f35",
      "input.background": "#0c0e10",
      "input.border": "#2a2f35",
      "scrollbarSlider.background": "#ffffff12",
      "scrollbarSlider.hoverBackground": "#ffffff1f",
    },
  });
}

/** The Monaco key codes for the key names a shortcut can hold. */
const KEY_CODES: Record<string, string> = {
  arrowup: "ArrowUp",
  arrowdown: "ArrowDown",
  arrowleft: "ArrowLeft",
  arrowright: "ArrowRight",
  enter: "Enter",
  escape: "Escape",
  tab: "Tab",
  space: "Space",
  backspace: "Backspace",
  delete: "Delete",
  home: "Home",
  end: "End",
  pageup: "PageUp",
  pagedown: "PageDown",
};

/**
 * Turns `Ctrl+Shift+ArrowUp` into the number Monaco uses, or `null` when the
 * shortcut names a key Monaco has no code for.
 */
export function monacoKeybinding(monaco: MonacoApi, shortcut: string): number | null {
  const parsed: Shortcut = parseShortcut(shortcut);
  if (!parsed.key) return null;
  const name = KEY_CODES[parsed.key.toLowerCase()] ?? parsed.key.toUpperCase();
  const code = (monaco.KeyCode as unknown as Record<string, number>)[name];
  if (code === undefined) return null;
  let binding = code;
  if (parsed.control) binding |= monaco.KeyMod.CtrlCmd;
  if (parsed.alt) binding |= monaco.KeyMod.Alt;
  if (parsed.shift) binding |= monaco.KeyMod.Shift;
  return binding;
}
