// Keyboard shortcuts as the settings spell them, and what they mean to Monaco.
//
// Pure string work: `Ctrl+Shift+L`, or a bare modifier such as `Ctrl` for the
// gestures that hold one down (`Settings → Editor → Keyboard Shortcuts`). The
// Monaco conversion lives in `monaco.ts`, which is the only module that needs the
// editor's key table.

/** A shortcut, split into its parts. */
export interface Shortcut {
  control: boolean;
  alt: boolean;
  shift: boolean;
  /** Upper-case letter, a key name such as `Enter` or `ArrowUp`, or empty. */
  key: string;
}

/** Parses `Ctrl+Shift+L` into its parts. Anything unparsable is ignored. */
export function parseShortcut(value: string): Shortcut {
  const parts = value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  const shortcut: Shortcut = { control: false, alt: false, shift: false, key: "" };
  for (const part of parts) {
    const lower = part.toLowerCase();
    if (lower === "ctrl" || lower === "control" || lower === "cmd" || lower === "meta") {
      shortcut.control = true;
    } else if (lower === "alt" || lower === "option") {
      shortcut.alt = true;
    } else if (lower === "shift") {
      shortcut.shift = true;
    } else {
      shortcut.key = part.length === 1 ? part.toUpperCase() : part;
    }
  }
  return shortcut;
}

/** Writes a shortcut back out in the form `parseShortcut` reads. */
export function formatShortcut(shortcut: Shortcut): string {
  const parts: string[] = [];
  if (shortcut.control) parts.push("Ctrl");
  if (shortcut.alt) parts.push("Alt");
  if (shortcut.shift) parts.push("Shift");
  if (shortcut.key) parts.push(shortcut.key);
  return parts.join("+");
}

/** What a keyboard or mouse event has to look like for a shortcut to match. */
export interface KeyboardLike {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
}

/**
 * True when the event is exactly this shortcut. A bare modifier (`Ctrl`) matches
 * the modifier being held, with no other key involved — that is what the canvas
 * pan and the "open a new editor" gestures ask for.
 */
export function matchesShortcut(event: KeyboardLike, shortcut: Shortcut | string): boolean {
  const parsed = typeof shortcut === "string" ? parseShortcut(shortcut) : shortcut;
  if (!parsed.key && !parsed.control && !parsed.alt && !parsed.shift) return false;
  const control = Boolean(event.ctrlKey || event.metaKey);
  const key = event.key;
  const isModifierKey = key === "Control" || key === "Meta" || key === "Alt" || key === "Shift";
  if (!parsed.key) {
    // A held modifier: the event must be that modifier (or carry it with nothing else).
    if (parsed.control && !control) return false;
    if (parsed.alt && !event.altKey) return false;
    if (parsed.shift && !event.shiftKey) return false;
    if (!isModifierKey && !(parsed.control && !parsed.alt && !parsed.shift)) return false;
    return true;
  }
  if (control !== parsed.control && !(parsed.control && control)) return false;
  if (control && !parsed.control) return false;
  if (Boolean(event.altKey) !== parsed.alt) return false;
  if (Boolean(event.shiftKey) !== parsed.shift) return false;
  return key.length === 1 ? key.toUpperCase() === parsed.key : key === parsed.key;
}

/** Whether a mouse event carries the modifier of a bare-modifier shortcut. */
export function holdsModifier(event: MouseEvent | PointerEvent, shortcut: string): boolean {
  return matchesShortcut(
    {
      key: "Control",
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      shiftKey: event.shiftKey,
    },
    shortcut,
  );
}
