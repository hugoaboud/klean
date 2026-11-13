// Lean's syntax, as a Monarch tokenizer for Monaco.
//
// Highlighting is a lexer, so this is where the language's *shape* lives: `--` and
// `/- -/` comments (which nest), doc comments, strings, character literals, numbers,
// the keywords of the language and the tactics of its proofs. Diagnostics and
// completions are not the lexer's business — they come from Lean's own language
// server, through `CodeEditor.vue`.
//
// It is kept apart from `monaco.ts` so that it is data, not a Monaco dependency:
// `npm test` drives it directly, which is how the comment bug below stayed fixed.
import type * as Monaco from "monaco-editor";

const KEYWORDS = [
  "def", "theorem", "lemma", "abbrev", "opaque", "axiom", "constant", "instance",
  "example", "structure", "class", "inductive", "coinductive", "namespace", "section",
  "end", "mutual", "import", "prelude", "open", "export", "variable", "universe",
  "universes", "set_option", "attribute", "notation", "infix", "infixl", "infixr",
  "prefix", "postfix", "syntax", "macro", "macro_rules", "elab", "deriving", "extends",
  "where", "with", "fun", "let", "in", "if", "then", "else", "match", "do", "have",
  "show", "from", "at", "by", "private", "protected", "noncomputable", "partial",
  "unsafe", "local", "scoped",
];

const TACTICS = [
  "intro", "intros", "exact", "apply", "refine", "rw", "simp", "simp_all", "dsimp",
  "unfold", "cases", "induction", "constructor", "contradiction", "exfalso",
  "obtain", "rcases", "rintro", "specialize", "subst", "trans", "calc", "ring",
  "omega", "linarith", "nlinarith", "positivity", "norm_num", "norm_cast", "push_cast",
  "field_simp", "gcongr", "assumption", "rfl", "trivial", "decide", "native_decide",
  "aesop", "tauto", "contrapose", "by_cases", "by_contra", "conv", "skip", "repeat",
  "try", "first", "all_goals", "any_goals", "focus", "case", "next", "trace_state",
  "sorry", "admit", "done",
];

/** A name Lean allows: unicode letters included, `'` and `!`/`?` at the end. */
const NAME = /[A-Za-z_α-ωΑ-Ω][A-Za-z0-9_'.!?₀-₉α-ωΑ-Ω]*/;

/** The lexer, built once so Monarch can see the keyword lists. */
export function leanLanguage(): Monaco.languages.IMonarchLanguage {
  return {
    defaultToken: "",
    keywords: KEYWORDS,
    tactics: TACTICS,
    tokenizer: {
      root: [
        [/\s+/, ""],
        [/--.*$/, "comment"],
        // `/-` opens a block comment; the `comment` state below closes it.
        [/\/-/, "comment", "@comment"],
        [/"/, "string", "@string"],
        [/'(\\.|[^'\\])'/, "string"],
        [/\b\d+(\.[0-9]+)?\b/, "number"],
        // A capitalised name is a type, and it has to be matched before `NAME`:
        // after it, the rule is unreachable and `Nat` reads like any identifier.
        [/[A-Z][A-Za-z0-9_'.]*/, "type"],
        [
          NAME,
          {
            cases: {
              "@keywords": "keyword",
              "@tactics": "keyword.tactic",
              "@default": "identifier",
            },
          },
        ],
        [/[∀∃λΠΣ⟨⟩⦃⦄↦→←↔≤≥≠∧∨¬⊢∈∉⊆⊂∪∩∑∏√∞∅∘±×÷·≡≃≈]/, "operator"],
        [/[=<>:+\-*/%|&^!?~.@#$]+/, "operator"],
        [/[{}()[\]]/, "@brackets"],
      ],
      // A block comment, which Lean *nests*: `/- a /- b -/ c -/` is one comment.
      //
      // Order matters more than it looks. A `-` on its own is a comment character,
      // so the rule that ends the comment has to come first: with `[-/]` ahead of
      // `-\/`, the closing `-/` was eaten one character at a time and the state
      // never popped. Every Lean file starts with a `/-! … -/` module doc comment,
      // so the whole file came out as one grey comment — while a node's text, which
      // usually starts after it, looked fine.
      comment: [
        [/-\//, "comment", "@pop"],
        [/\/-/, "comment", "@push"],
        [/[^/-]+/, "comment"],
        [/[-/]/, "comment"],
      ],
      string: [
        [/[^\\"]+/, "string"],
        [/\\./, "string.escape"],
        [/"/, "string", "@pop"],
      ],
    },
  };
}
