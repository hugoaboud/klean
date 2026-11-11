// Geometry for the graph.
//
// Libraries, folders and files are drawn as squares. Each group also gets a
// square *frame* fitted to its content — the union of everything it holds — so a
// file's AST sits visibly inside that file's square and a library's files sit
// inside the library's square. Frames are drawn from the settled positions and
// never feed back into the simulation, which keeps the layout compact: the force
// layout only has to pack nodes, not nested boxes.
//
// Nothing here depends on Vue or D3, so the numbers can be reasoned about (and
// checked) on their own.

/** Kinds drawn as squares: groups that contain other nodes. */
export const GROUP_KINDS = new Set(["lib", "folder", "file", "astgroup"]);

/** Half edge of a group's own footprint (its label anchor area). */
export const SQUARE_R = 11;
/** Height of an AST node pill. */
export const PILL_H = 21;
/** Smallest edge a group square can have. */
export const MIN_GROUP = 30;
/** Space kept between a group square and the content it holds. */
export const GROUP_PADDING = 24;

export interface LayoutNode {
  id: string;
  parent: string | null;
  kind: string;
  label: string;
  child_count: number;
  x?: number;
  y?: number;
}

/** Axis-aligned rectangle, centre + size. */
export interface Box {
  cx: number;
  cy: number;
  w: number;
  h: number;
}

export function isGroup(node: LayoutNode, kinds: Set<string> = GROUP_KINDS): boolean {
  return kinds.has(node.kind);
}

/** Half width of an AST node pill, estimated from its label. */
export function pillHalfWidth(node: LayoutNode): number {
  const toggle = node.child_count > 0 ? 15 : 0;
  // The trailing constant is slack on both sides, so a label never touches the
  // end of the pill.
  return Math.min(190, 36 + node.label.length * 5.9 + toggle) / 2;
}

/** Footprint of a node itself, ignoring its children. */
export function ownBox(node: LayoutNode, kinds: Set<string> = GROUP_KINDS): Box {
  if (isGroup(node, kinds)) {
    return { cx: node.x ?? 0, cy: node.y ?? 0, w: SQUARE_R * 2, h: SQUARE_R * 2 };
  }
  return { cx: node.x ?? 0, cy: node.y ?? 0, w: pillHalfWidth(node) * 2, h: PILL_H };
}

function union(a: Box, b: Box): Box {
  const left = Math.min(a.cx - a.w / 2, b.cx - b.w / 2);
  const right = Math.max(a.cx + a.w / 2, b.cx + b.w / 2);
  const top = Math.min(a.cy - a.h / 2, b.cy - b.h / 2);
  const bottom = Math.max(a.cy + a.h / 2, b.cy + b.h / 2);
  return {
    cx: (left + right) / 2,
    cy: (top + bottom) / 2,
    w: right - left,
    h: bottom - top,
  };
}

/** Index of visible children by parent id. */
export function indexChildren<T extends LayoutNode>(nodes: T[]): Map<string, T[]> {
  const index = new Map<string, T[]>();
  for (const node of nodes) {
    if (!node.parent) continue;
    const bucket = index.get(node.parent);
    if (bucket) bucket.push(node);
    else index.set(node.parent, [node]);
  }
  return index;
}

/** Depth of every node, derived from the parent links. */
function depths(nodes: LayoutNode[]): Map<string, number> {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const depth = new Map<string, number>();
  const resolve = (node: LayoutNode): number => {
    const known = depth.get(node.id);
    if (known !== undefined) return known;
    const parent = node.parent ? byId.get(node.parent) : undefined;
    const value = parent ? resolve(parent) + 1 : 0;
    depth.set(node.id, value);
    return value;
  };
  for (const node of nodes) resolve(node);
  return depth;
}

/** Height of a group's title strip, drawn above its frame. */
export const GROUP_HEADER = 19;

/**
 * Box for every node: everything it draws, children included. A group's box is
 * its padded content plus the header strip above it, so a parent's frame always
 * clears its children's titles instead of letting them poke over the edge.
 * Deepest nodes first.
 */
export function measureBoxes(
  nodes: LayoutNode[],
  children: Map<string, LayoutNode[]>,
  options: { padding?: number; groupKinds?: Set<string> } = {},
): Map<string, Box> {
  const padding = options.padding ?? GROUP_PADDING;
  const kinds = options.groupKinds ?? GROUP_KINDS;
  const depth = depths(nodes);
  const ordered = [...nodes].sort((a, b) => (depth.get(b.id) ?? 0) - (depth.get(a.id) ?? 0));
  const boxes = new Map<string, Box>();

  for (const node of ordered) {
    const kids = children.get(node.id) ?? [];
    let content: Box | null = null;
    for (const kid of kids) {
      const kidBox = boxes.get(kid.id);
      if (!kidBox) continue;
      content = content ? union(content, kidBox) : kidBox;
    }

    let box: Box;
    if (isGroup(node, kinds) && content) {
      // A group with visible contents is exactly its contents plus a margin and
      // the title strip. Its own anchor is not drawn — counting it would inflate
      // the frame and leave the empty space that made groups look oversized.
      box = {
        cx: content.cx,
        cy: content.cy - GROUP_HEADER / 2,
        w: content.w + padding * 2,
        h: content.h + padding * 2 + GROUP_HEADER,
      };
    } else if (content) {
      box = union(ownBox(node, kinds), content);
    } else {
      box = ownBox(node, kinds);
    }
    boxes.set(node.id, box);
  }
  return boxes;
}

