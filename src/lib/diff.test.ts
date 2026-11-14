// The side-by-side diff the AI feed shows after a `modify_ast` prompt.
import assert from "node:assert";
import { describe, it } from "node:test";
import { changedLines, sideBySide } from "./diff.ts";

describe("side-by-side diff", () => {
  it("lines up an edited line and leaves the rest alone", () => {
    const rows = sideBySide("def two : Nat := 2", "def two : Nat := 3");
    assert.deepEqual(rows, [
      { left: "def two : Nat := 2", right: null, kind: "remove" },
      { left: null, right: "def two : Nat := 3", kind: "add" },
    ]);
    assert.deepEqual(changedLines(rows), { added: 1, removed: 1 });
  });

  it("keeps the lines around a change as rows of their own", () => {
    const rows = sideBySide(
      "theorem t : P := by\n  simp\n  rfl",
      "theorem t : P := by\n  simp\n  omega\n  rfl",
    );
    assert.deepEqual(rows.map((row) => row.kind), [
      "same",
      "same",
      "add",
      "same",
    ]);
    assert.equal(rows[2].right, "  omega");
    assert.equal(rows[2].left, null);
  });

  it("shows a new declaration as additions only", () => {
    const rows = sideBySide("", "def three : Nat := 3\ndef four : Nat := 4");
    assert.ok(rows.every((row) => row.kind === "add"));
    assert.equal(rows.length, 2);
  });

  it("shows a removed declaration as removals only", () => {
    const rows = sideBySide("def two : Nat := 2", "");
    assert.deepEqual(rows, [{ left: "def two : Nat := 2", right: null, kind: "remove" }]);
  });

  it("ignores the empty line a final newline leaves behind", () => {
    assert.deepEqual(sideBySide("a\nb\n", "a\nb\n"), [
      { left: "a", right: "a", kind: "same" },
      { left: "b", right: "b", kind: "same" },
    ]);
  });

  it("keeps every line of both sides, in order", () => {
    const rows = sideBySide("a\nb\nc", "a\nc\nd\ne");
    assert.equal(rows.length >= 5, true, JSON.stringify(rows));
    // Every left-side line appears once, in order; same for the right.
    assert.deepEqual(rows.filter((row) => row.left !== null).map((row) => row.left), [
      "a",
      "b",
      "c",
    ]);
    assert.deepEqual(rows.filter((row) => row.right !== null).map((row) => row.right), [
      "a",
      "c",
      "d",
      "e",
    ]);
  });
});
