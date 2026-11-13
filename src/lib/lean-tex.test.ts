// Lean's LaTeX-style input, checked without a browser.
import assert from "node:assert";
import { describe, it } from "node:test";
import { TEX_SYMBOLS, texCompletion, texPrefix, texSuggestions } from "./lean-tex.ts";

describe("\\name completion", () => {
  it("turns a name before the caret into its symbol", () => {
    assert.deepEqual(texCompletion("\\all"), { name: "all", symbol: "∀" });
    assert.deepEqual(texCompletion("theorem x := \\forall"), { name: "forall", symbol: "∀" });
  });

  it("takes the arrow spellings, which are names too", () => {
    assert.deepEqual(texCompletion("\\->"), { name: "->", symbol: "→" });
    assert.deepEqual(texCompletion("\\<="), { name: "<=", symbol: "≤" });
    assert.deepEqual(texCompletion("\\=>"), { name: "=>", symbol: "⇒" });
  });

  it("ignores a name it does not know, and a backslash that is not one", () => {
    assert.equal(texCompletion("\\nosuch"), null);
    assert.equal(texCompletion("x \\ y"), null);
    assert.equal(texCompletion("all"), null);
  });

  it("reports the prefix being typed, even before its first letter", () => {
    assert.deepEqual(texPrefix("f \\fo"), { name: "fo", start: 2 });
    assert.deepEqual(texPrefix("f \\"), { name: "", start: 2 });
    assert.equal(texPrefix("no backslash"), null);
  });

  it("offers the symbols whose name starts with what was typed", () => {
    const names = texSuggestions("for").map((entry) => entry.name);
    assert.ok(names.includes("forall"), "{names:?}");
    const arrows = texSuggestions("->").map((entry) => entry.symbol);
    assert.deepEqual(arrows, ["→"]);
    assert.ok(texSuggestions("").length > 50, "a bare backslash lists everything");
  });

  it("knows every symbol it offers", () => {
    for (const [name, symbol] of Object.entries(TEX_SYMBOLS)) {
      assert.ok(symbol.length > 0, `${name} has a symbol`);
      assert.deepEqual(texCompletion(`\\${name}`), { name, symbol });
    }
  });
});
