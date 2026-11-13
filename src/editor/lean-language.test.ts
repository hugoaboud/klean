// The Lean lexer, checked without Monaco.
import assert from "node:assert";
import { describe, it } from "node:test";
import { leanLanguage } from "./lean-language.ts";

type Action = string | { cases: Record<string, string> };
type Rule = [RegExp, Action];

interface Token {
  type: string;
  text: string;
}

/**
 * A Monarch-subset driver: the first matching rule wins, and the three actions the
 * language uses (`@comment`, `@push`, `@pop`) are the only state changes there are.
 *
 * It checks the *rules*, not Monaco. What it catches is a rule sitting behind
 * another that eats its first character — the bug that turned every file grey,
 * because a comment's closing `-/` was consumed by the `-` rule before the rule
 * that ends the comment could see it.
 */
function tokenize(lines: string[]): Token[] {
  const language = leanLanguage() as unknown as {
    keywords: string[];
    tactics: string[];
    defaultToken: string;
    tokenizer: Record<string, Rule[]>;
  };
  const stack: string[] = [];
  let state = "root";
  const tokens: Token[] = [];
  // A rule is `[pattern, token]` or `[pattern, token, nextState]`, and either half
  // can be the state action (`"@comment"`, `"@pop"`, `"@push"`).
  const step = (action: string) => {
    if (action === "@pop") state = stack.pop() ?? "root";
    else if (action === "@push") stack.push(state);
    else if (action.startsWith("@")) {
      stack.push(state);
      state = action.slice(1);
    }
  };
  for (const line of lines) {
    let column = 0;
    while (column < line.length) {
      const rules = language.tokenizer[state];
      assert.ok(rules, `Monarch state \`${state}\` exists`);
      let matched = false;
      for (const rule of rules) {
        const [pattern, token, next] = rule as [RegExp, Action, string?];
        const sticky = new RegExp(pattern.source, "y");
        sticky.lastIndex = column;
        const found = sticky.exec(line);
        if (!found) continue;
        matched = true;
        column += found[0].length;
        const type = typeof token === "string" ? token : resolve(token, found[0], language);
        if (type.startsWith("@")) step(type);
        else tokens.push({ type, text: found[0] });
        if (next) step(next);
        break;
      }
      if (!matched) {
        // Monarch's fallback: an unmatched character is one token of `defaultToken`.
        tokens.push({ type: language.defaultToken, text: line[column] });
        column += 1;
      }
    }
    tokens.push({ type: "newline", text: "" });
  }
  return tokens;
}

/** The `cases` action: a word is a keyword, a tactic, or an identifier. */
function resolve(
  action: { cases: Record<string, string> },
  text: string,
  language: { keywords: string[]; tactics: string[] },
): string {
  if (language.keywords.includes(text)) return action.cases["@keywords"] ?? "";
  if (language.tactics.includes(text)) return action.cases["@tactics"] ?? "";
  return action.cases["@default"] ?? "";
}

/**
 * The tokens with a colour, in order, as `type:text`; whitespace is not one, and
 * neighbours of the same type are joined — Monaco paints them as one run.
 */
function painted(lines: string[]): string[] {
  const tokens = tokenize(lines).filter(
    (token) => token.type !== "" && token.type !== "newline",
  );
  const merged: Token[] = [];
  for (const token of tokens) {
    const last = merged[merged.length - 1];
    if (last && last.type === token.type) last.text += token.text;
    else merged.push({ ...token });
  }
  return merged
    .filter((token) => token.text.trim() !== "")
    .map((token) => `${token.type}:${token.text}`);
}

describe("the Lean lexer", () => {
  it("ends a module doc comment, which is how every file starts", () => {
    // The regression: `-/` was eaten by the single `-` rule, the comment state never
    // popped, and the rest of the file came out as one grey token.
    const tokens = painted(["/-!", "Copyright (c) 2024", "-/", "import Mathlib"]);
    assert.deepEqual(tokens.slice(-2), ["keyword:import", "type:Mathlib"]);
    for (const token of tokens.slice(0, -2)) {
      assert.ok(token.startsWith("comment:"), `${token} is inside the module doc`);
    }
  });

  it("keeps the lines after a comment out of it", () => {
    const tokens = painted(["/-- doc: a note -/", "theorem t : True := trivial"]);
    const line = tokens.slice(tokens.indexOf("keyword:theorem"));
    assert.ok(line.length > 0, "the theorem is code, not comment");
    assert.ok(
      !line.some((token) => token.startsWith("comment:")),
      `nothing after the comment is one: ${line.join(" ")}`,
    );
  });

  it("closes a block comment where it ends, nested ones included", () => {
    // Lean nests: the inner `-/` ends the inner comment, not the outer one.
    const tokens = painted(["/- a /- b -/ c -/ def x := 1"]);
    assert.deepEqual(tokens, [
      "comment:/- a /- b -/ c -/",
      "keyword:def",
      "identifier:x",
      "operator::=",
      "number:1",
    ]);
  });

  it("stops a line comment, a string and a character literal at their end", () => {
    assert.ok(painted(["-- note", "def x := 1"]).includes("keyword:def"));
    const quoted = painted(['example : "hi" = "hi" := rfl']);
    assert.equal(quoted.filter((token) => token === 'string:"hi"').length, 2, quoted.join(" "));
    assert.ok(quoted.includes("keyword.tactic:rfl"), quoted.join(" "));
    assert.ok(painted(["'a'", "def x := 1"]).includes("keyword:def"));
  });

  it("separates keywords, tactics, types and identifiers", () => {
    const tokens = painted(["def two : Nat := 2"]);
    assert.deepEqual(tokens, [
      "keyword:def",
      "identifier:two",
      "operator::",
      "type:Nat",
      "operator::=",
      "number:2",
    ]);
    assert.ok(painted(["by simp"]).includes("keyword.tactic:simp"));
  });

  it("does not colour Lean's symbols as identifiers", () => {
    assert.ok(painted(["∀ x, x = x"]).includes("operator:∀"));
    assert.ok(painted(["x ⟨ y ⟩"]).includes("operator:⟨"));
  });
});
