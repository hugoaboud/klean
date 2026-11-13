// The shortcut settings, checked without a browser.
import assert from "node:assert";
import { describe, it } from "node:test";
import { formatShortcut, holdsModifier, matchesShortcut, parseShortcut } from "./keys.ts";

describe("shortcuts", () => {
  it("parses and writes combinations", () => {
    assert.deepEqual(parseShortcut("Ctrl+Shift+L"), {
      control: true,
      alt: false,
      shift: true,
      key: "L",
    });
    assert.equal(formatShortcut(parseShortcut("ctrl+z")), "Ctrl+Z");
    assert.deepEqual(parseShortcut("Ctrl"), { control: true, alt: false, shift: false, key: "" });
  });

  it("matches a combination exactly", () => {
    const find = "Ctrl+F";
    assert.ok(matchesShortcut({ key: "f", ctrlKey: true }, find));
    assert.ok(!matchesShortcut({ key: "f", ctrlKey: true, shiftKey: true }, find));
    assert.ok(!matchesShortcut({ key: "f" }, find));
    // Cmd counts as Ctrl, so the same settings work on macOS.
    assert.ok(matchesShortcut({ key: "f", metaKey: true }, find));
  });

  it("matches arrows and the keys Monaco names", () => {
    const duplicate = "Ctrl+Alt+Shift+ArrowUp";
    assert.ok(matchesShortcut({ key: "ArrowUp", ctrlKey: true, altKey: true, shiftKey: true }, duplicate));
    assert.ok(!matchesShortcut({ key: "ArrowUp", ctrlKey: true, shiftKey: true }, duplicate));
  });

  it("matches a held modifier", () => {
    assert.ok(matchesShortcut({ key: "Control", ctrlKey: true }, "Ctrl"));
    assert.ok(matchesShortcut({ key: "ArrowLeft", ctrlKey: true }, "Ctrl"));
    assert.ok(!matchesShortcut({ key: "ArrowLeft" }, "Ctrl"));
  });

  it("reads the modifier off a mouse event", () => {
    const click = { ctrlKey: true, metaKey: false, altKey: false, shiftKey: false } as MouseEvent;
    assert.ok(holdsModifier(click, "Ctrl"));
    assert.ok(!holdsModifier({ ...click, ctrlKey: false } as MouseEvent, "Ctrl"));
  });
});
