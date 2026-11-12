// Splitting model output into markdown and mermaid blocks.
//
// This lives apart from `markdown.ts` because it is pure: the renderer needs a DOM
// and highlight.js, while this is just fence accounting, and keeping it separate
// means it can be checked on its own.

/** One piece of model output: markdown to render, or a diagram to draw. */
export interface Block {
  kind: "markdown" | "mermaid";
  text: string;
}

/**
 * Splits `text` into markdown and mermaid blocks.
 *
 * An unterminated fence still counts as a diagram: a truncated answer should show
 * what it managed to draw rather than nothing.
 */
export function splitBlocks(text: string): Block[] {
  const blocks: Block[] = [];
  const open = /^\s*```mermaid\s*$/im;
  const close = /^\s*```\s*$/m;
  let rest = text;
  while (rest.length > 0) {
    const match = rest.match(open);
    if (!match || match.index === undefined) {
        if (rest.trim()) push(blocks, { kind: "markdown", text: rest });
      break;
    }
    const before = rest.slice(0, match.index);
    if (before.trim()) push(blocks, { kind: "markdown", text: before });
    const after = rest.slice(match.index + match[0].length);
    const end = after.search(close);
    const diagram = end >= 0 ? after.slice(0, end) : after;
    blocks.push({ kind: "mermaid", text: diagram.trim() });
    if (end < 0) break;
    rest = after.slice(end).replace(close, "");
  }
  return blocks;
}

/** Appends a block, joining it with the previous one when both are markdown. */
function push(blocks: Block[], block: Block) {
  const last = blocks[blocks.length - 1];
  if (block.kind === "markdown" && last?.kind === "markdown") {
    last.text += block.text;
    return;
  }
  blocks.push(block);
}

/** True when the text holds a mermaid diagram. */
export function hasDiagram(text: string): boolean {
  return /^\s*```mermaid\s*$/im.test(text);
}
