// LaTeX-style input, as the Lean VS Code extension offers: type `\` + a name and
// press space (or tab) to get the unicode symbol.

export const TEX_SYMBOLS: Record<string, string> = {
  forall: "∀", all: "∀", exists: "∃", any: "∃", lam: "λ", lambda: "λ", fun: "λ",
  Pi: "Π", Sigma: "Σ",
  to: "→", rarr: "→", get: "←", larr: "←", iff: "↔", lr: "↔",
  and: "∧", or: "∨", not: "¬", neg: "¬", ne: "≠", le: "≤", ge: "≥", lt: "<", gt: ">",
  in: "∈", notin: "∉", subset: "⊆", supset: "⊇", cup: "∪", cap: "∩",
  emptyset: "∅", inter: "⋂", union: "⋃", times: "×", cdot: "·", circ: "∘",
  mapsto: "↦", implies: "⇒", impliedby: "⇐", iff_arrow: "⇔",
  alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ε", zeta: "ζ",
  eta: "η", theta: "θ", iota: "ι", kappa: "κ", mu: "μ", nu: "ν", xi: "ξ",
  pi: "π", rho: "ρ", sigma: "σ", tau: "τ", phi: "φ", chi: "χ", psi: "ψ",
  omega: "ω", Gamma: "Γ", Delta: "Δ", Theta: "Θ", Lambda: "Λ", Xi: "Ξ",
  Pi_up: "Π", Sigma_up: "Σ", Phi: "Φ", Psi: "Ψ", Omega: "Ω",
  sum: "∑", prod: "∏", int: "∫", sqrt: "√", infin: "∞", partial: "∂",
  nat: "ℕ", int_ty: "ℤ", rat: "ℚ", real: "ℝ", complex: "ℂ",
  lbrace: "{", rbrace: "}", langle: "⟨", rangle: "⟩", lc: "⟨", rc: "⟩",
  sqcap: "⊓", sqcup: "⊔", le_ty: "≤", me: "≥", ne_ty: "≠",
  "<->": "↔", "->": "→", "<-": "←", "=>": "⇒", ">=": "≥", "<=": "≤",
  bottom: "⊥", top: "⊤", turnstile: "⊢", vdash: "⊢", models: "⊨",
  qed: "∎", square: "□", diamond: "◇", degree: "°", prime: "′",
  subseteq: "⊆", supseteq: "⊇", equiv: "≡", sim: "∼", approx: "≈",
  angle: "∠", triangle: "△", star: "⋆", bullet: "•", dots: "…", cdots: "⋯",
};

const PATTERN = /\\([A-Za-z<>=\-_]{1,12})$/;

/** Returns the symbol completing the `\name` right before the caret. */
export function texCompletion(before: string): { name: string; symbol: string } | null {
  const match = PATTERN.exec(before);
  if (!match) return null;
  const symbol = TEX_SYMBOLS[match[1]];
  return symbol ? { name: match[1], symbol } : null;
}

/**
 * The `\name` being typed right before the caret — possibly empty right after the
 * backslash — so the editor can offer the symbols as a list.
 */
export function texPrefix(before: string): { name: string; start: number } | null {
  const match = /\\([A-Za-z<>=\-_]{0,24})$/.exec(before);
  if (!match) return null;
  return { name: match[1], start: before.length - match[0].length };
}

/** The symbols whose name starts with what has been typed, for the suggestion list. */
export function texSuggestions(name: string): { name: string; symbol: string }[] {
  const query = name.toLowerCase();
  return Object.keys(TEX_SYMBOLS)
    .filter((key) => key.toLowerCase().startsWith(query))
    .sort((left, right) => left.localeCompare(right))
    .map((key) => ({ name: key, symbol: TEX_SYMBOLS[key] }));
}
