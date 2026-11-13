<script setup lang="ts">
// The logic board.
//
// A stack of graphs. The root graph holds the theory nodes; opening one (double
// click, or a breadcrumb) shows that theory's graph: its terms, the theories
// nested inside it and the horizontal layer dividers. Nothing is laid out here:
// a node sits where the user put it, the rectangle around a theory graph is
// derived from what it holds, and a link is drawn only between neighbour layers.
//
// Mouse: drag a node or a divider to move it, drag the background for a rubber
// band, ctrl/cmd + drag to pan, wheel to zoom, double click a theory to open it,
// right click for the tools (on empty canvas) or Delete (on a theory/divider).
import * as d3 from "d3";
import { matchesShortcut } from "../editor/keys";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Camera, FixedNode, GraphLink, GraphNode, NodeDetail } from "../api/lean";
import UiIcon from "./ui/UiIcon.vue";

interface SimNode extends GraphNode {
  x: number;
  y: number;
}

interface Layer {
  id: string;
  name: string;
  y: number;
}

const props = withDefaults(
  defineProps<{
    nodes: GraphNode[];
    links: GraphLink[];
    layers: Layer[];
    external: {
      source: string;
      side: string;
      targets: { label: string; kind: string; namespace: string }[];
    }[];
    trail: { key: string; name: string }[];
    /** Key of the open theory; empty at the root. */
    theory: string;
    selectedIds: string[];
    camera?: Camera | null;
    hoverInfo?: NodeDetail | null;
    /** Held while dragging to look around, from `Settings → Editor`. */
    panKey?: string;
  }>(),
  { camera: null, hoverInfo: null, panKey: "Ctrl" },
);

const emit = defineEmits<{
  pick: [string, boolean];
  select: [string[]];
  open: [string, PointerEvent];
  "open-theory": [string];
  crumb: [string];
  move: [FixedNode[]];
  camera: [Camera];
  hover: [string | null];
  settings: [];
  "create-theory": [];
  "add-axiom": [];
  "rename-theory": [string];
  "add-layer": [];
  "delete-theory": [string];
  "delete-layer": [string];
  "delete-axiom": [string];
  "rename-layer": [string];
  "move-layer": [id: string, y: number];
  assign: [ids: string[], theory: string | null];
}>();

const host = ref<HTMLDivElement | null>(null);
const graphError = ref<string | null>(null);
const toolsOpen = ref(false);
const toolsAt = ref<{ x: number; y: number } | null>(null);
/** Context menu of a theory node or a divider. */
const target = ref<{
  kind: "theory" | "layer" | "axiom";
  id: string;
  name: string;
  x: number;
  y: number;
} | null>(null);
/** Axiom waiting for the confirmation that removes it from its file. */
const confirmAxiom = ref<{ id: string; label: string; path: string | null } | null>(null);
const dropTheory = ref<string | null>(null);
/** True while the board fades between two theory graphs. */
const fading = ref(false);
const card = ref<{
  id: string;
  label: string;
  kind: string;
  status: string | null;
  detail: string | null;
  path: string | null;
  namespace: string;
  comments: string;
  x: number;
  y: number;
} | null>(null);

/** Hover over a diamond at the edge: everything that node refers to outside. */
const edgeCard = ref<{
  side: string;
  targets: { label: string; kind: string; namespace: string }[];
  x: number;
  y: number;
} | null>(null);

const insideTheory = computed(() => props.theory !== "");
const selected = computed(() => new Set(props.selectedIds));

/** Pill height, and the theory box size, in board units. */
const PILL_H = 21;
const THEORY_H = 42;
const RECT_PAD = 28;
/** Height of the band above the rect that carries the theory name. */
const HEADER_H = 22;
const LAYER_GAP = 8;

function pillHalf(label: string): number {
  return Math.min(190, 36 + label.length * 5.9) / 2;
}

function theoryBox(node: GraphNode): { w: number; h: number } {
  return { w: Math.min(260, Math.max(128, 40 + node.label.length * 7.4)), h: THEORY_H };
}

function halfHeight(node: GraphNode): number {
  return node.kind === "theory" ? THEORY_H / 2 : PILL_H / 2;
}

function halfWidth(node: GraphNode): number {
  return node.kind === "theory" ? theoryBox(node).w / 2 : pillHalf(node.label);
}

function truncate(text: string, limit: number): string {
  return text.length <= limit ? text : text.slice(0, limit - 1) + "…";
}

/**
 * True when this event carries the modifier the user set for looking around the
 * board by dragging it (`Settings → Editor → Pan the Canvas`).
 */
function pans(event: { ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }): boolean {
  return matchesShortcut(
    {
      key: "Control",
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      shiftKey: event.shiftKey,
    },
    props.panKey ?? "Ctrl",
  );
}

let svg: d3.Selection<SVGSVGElement, unknown, null, undefined> | null = null;
let root: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let rectLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let layerLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let linkLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let nodeLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let nodeGroups: d3.Selection<SVGGElement, SimNode, SVGGElement, unknown> | null = null;

let zoom: d3.ZoomBehavior<SVGSVGElement, unknown> | null = null;
let observer: ResizeObserver | null = null;
let currentNodes: SimNode[] = [];
let rubber: d3.Selection<SVGRectElement, unknown, null, undefined> | null = null;

/** Namespace a node lives in, read back from its key (`kind:name@file`). */
function namespaceOf(node: GraphNode): string {
  const head = node.key.split("@")[0];
  const colon = head.indexOf(":");
  const qual = colon >= 0 ? head.slice(colon + 1) : head;
  const dot = qual.lastIndexOf(".");
  if (node.kind === "theory") return "";
  return dot > 0 ? qual.slice(0, dot) : "root";
}

function showEdgeCard(event: PointerEvent, stub: (typeof props.external)[number]) {
  const bounds = host.value?.getBoundingClientRect();
  if (!bounds) return;
  edgeCard.value = {
    side: stub.side,
    targets: stub.targets,
    x: Math.min(Math.max(8, event.clientX - bounds.left + 14), bounds.width - 250),
    y: Math.min(Math.max(8, event.clientY - bounds.top + 16), bounds.height - 120),
  };
}

/** The rectangle wrapping the open theory graph: the box of everything in it. */
function contentRect(): { left: number; right: number; top: number; bottom: number } | null {
  // Sub-theory nodes are not part of the theory's own content: they can be
  // dragged outside of the rectangle.
  const content = currentNodes.filter((node) => node.kind !== "theory");
  if (content.length === 0) return null;
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  for (const node of content) {
    left = Math.min(left, node.x - halfWidth(node));
    right = Math.max(right, node.x + halfWidth(node));
    top = Math.min(top, node.y - halfHeight(node));
    bottom = Math.max(bottom, node.y + halfHeight(node));
  }
  return {
    left: left - RECT_PAD,
    right: right + RECT_PAD,
    top: top - RECT_PAD - HEADER_H,
    bottom: bottom + RECT_PAD,
  };
}

/**
 * What the camera frames: the theory rect plus any sub-theory node, which is
 * allowed to sit outside of it.
 */
function fitBounds(): { left: number; right: number; top: number; bottom: number } | null {
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  const add = (box: { left: number; right: number; top: number; bottom: number }) => {
    left = Math.min(left, box.left);
    right = Math.max(right, box.right);
    top = Math.min(top, box.top);
    bottom = Math.max(bottom, box.bottom);
  };
  const rect = contentRect();
  if (rect) add(rect);
  for (const node of currentNodes) {
    if (node.kind !== "theory") continue;
    add({
      left: node.x - halfWidth(node),
      right: node.x + halfWidth(node),
      top: node.y - halfHeight(node),
      bottom: node.y + halfHeight(node),
    });
  }
  return left === Infinity ? null : { left, right, top, bottom };
}

function guard(action: () => void) {
  try {
    action();
    graphError.value = null;
  } catch (error) {
    graphError.value = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  }
}

// ---------------------------------------------------------------- lifecycle

function mount() {
  const el = host.value;
  if (!el) return;

  svg = d3
    .select(el)
    .append("svg")
    .attr("class", "logic__svg")
    .attr("width", el.clientWidth)
    .attr("height", el.clientHeight);

  root = svg.append("g").attr("class", "logic__root");
  rectLayer = root.append("g").attr("class", "logic__rect");
  layerLayer = root.append("g").attr("class", "logic__layers");
  linkLayer = root.append("g").attr("class", "logic__links");
  nodeLayer = root.append("g").attr("class", "logic__nodes");

  zoom = d3
    .zoom<SVGSVGElement, unknown>()
    .scaleExtent([0.2, 2.4])
    .filter((event: Event) => {
      if (event.type === "wheel") return true;
      const mouse = event as MouseEvent;
      return mouse.type === "mousedown" && pans(mouse);
    })
    .on("zoom", (event) => {
      applyTransform(event.transform);
      scheduleCamera();
    });
  svg.call(zoom).on("dblclick.zoom", null);
  svg.on("pointerdown", onBackgroundDown);
  svg.on("contextmenu", onContextMenu);
  el.addEventListener("dragover", onDragOver);
  el.addEventListener("dragleave", onDragLeave);
  el.addEventListener("drop", onDrop);

  observer = new ResizeObserver(() => {
    if (!el || !svg) return;
    svg.attr("width", el.clientWidth).attr("height", el.clientHeight);
  });
  observer.observe(el);
}

// ------------------------------------------------------------------ render

function render() {
  currentNodes = props.nodes.map((node) => ({ ...node }));
  draw();
}

function draw() {
  paintRect();
  paintLayers();
  paintLinks();
  paintNodes();
  applyCameraOnce();
}

/** The wrapping rectangle of the open theory graph. */
function paintRect() {
  if (!rectLayer) return;
  const rect = insideTheory.value ? contentRect() : null;
  const data = rect ? [rect] : [];
  rectLayer
    .selectAll<SVGRectElement, { left: number; right: number; top: number; bottom: number }>(
      "rect.logic-rect",
    )
    .data(data)
    .join("rect")
    .attr("class", "logic-rect")
    .attr("x", (box) => box.left)
    .attr("y", (box) => box.top)
    .attr("width", (box) => box.right - box.left)
    .attr("height", (box) => box.bottom - box.top);

  rectLayer
    .selectAll<SVGTextElement, { left: number; right: number; top: number; bottom: number }>(
      "text.logic-rect__title",
    )
    .data(data)
    .join("text")
    .attr("class", "logic-rect__title")
    .attr("x", (box) => box.left)
    .attr("y", (box) => box.top + 14)
    .text(() => props.trail[props.trail.length - 1]?.name ?? "");
}

/** Layer dividers: a line across the rect, its name at the right end. */
function paintLayers() {
  if (!layerLayer) return;
  const rect = contentRect();
  const layers = rect && insideTheory.value ? props.layers : [];
  const span = rect ?? { left: -80, right: 80, top: 0, bottom: 0 };

  const groups = layerLayer
    .selectAll<SVGGElement, Layer>("g.logic-layer")
    .data(layers, (layer) => layer.id)
    .join((enter) => {
      const group = enter.append("g").attr("class", "logic-layer");
      group.append("line").attr("class", "logic-layer__line");
      group.append("rect").attr("class", "logic-layer__grip");
      group.append("text").attr("class", "logic-layer__label");
      return group;
    });

  groups
    .attr("data-selected", (layer) => (target.value?.id === layer.id ? "true" : null))
    .attr("transform", (layer) => `translate(0,${layer.y})`);

  groups
    .select(".logic-layer__line")
    .attr("x1", span.left)
    .attr("x2", span.right)
    .attr("y1", 0)
    .attr("y2", 0);

  // A thin strip that is easy to grab without covering the nodes.
  groups
    .select(".logic-layer__grip")
    .attr("x", span.left)
    .attr("y", -LAYER_GAP)
    .attr("width", Math.max(span.right - span.left, 1))
    .attr("height", LAYER_GAP * 2);

  groups
    .select(".logic-layer__label")
    .attr("x", span.right - 6)
    .attr("y", -6)
    .attr("text-anchor", "end")
    .text((layer) => layer.name);

  groups
    .on("pointerdown", (event: PointerEvent, layer: Layer) => {
      if (event.button !== 0) return;
      startLayerDrag(event, layer);
    })
    .on("contextmenu", (event: PointerEvent, layer: Layer) => {
      event.preventDefault();
      event.stopPropagation();
      openTargetMenu(event, "layer", layer.id, layer.name);
    });
}

/**
 * Layer a node falls in: the number of dividers above it. Read from the live
 * coordinates, so a node dragged across a divider changes layer right away.
 */
function bandOf(y: number): number {
  if (!insideTheory.value) return 0;
  return props.layers.filter((layer) => layer.y <= y).length;
}

/**
 * A node belongs to exactly one layer: the number of dividers above it. Links
 * are drawn between nodes of the same layer and of neighbouring layers; only a
 * link that skips a layer stays hidden, and even then it shows up (dashed,
 * quieter) while a node it touches is selected.
 */
function isHiddenLink(link: GraphLink, byId: Map<string, SimNode>): boolean {
  if (link.structural || !insideTheory.value || props.layers.length === 0) return false;
  const source = byId.get(link.source);
  const target = byId.get(link.target);
  if (!source || !target) return false;
  return Math.abs(bandOf(source.y) - bandOf(target.y)) > 1;
}

/** Links inside the graph, plus the dashed stubs that leave it. */
function paintLinks() {
  if (!linkLayer) return;
  const byId = new Map(currentNodes.map((node) => [node.id, node]));
  const rect = contentRect();

  const links = props.links
    .filter((link) => byId.has(link.source) && byId.has(link.target))
    .map((link) => {
      const hidden = isHiddenLink(link, byId);
      const touched = selected.value.has(link.source) || selected.value.has(link.target);
      return { link, hidden, touched };
    })
    // A hidden link is only worth drawing while it is being traced from a node.
    .filter((entry) => !entry.hidden || entry.touched);

  linkLayer
    .selectAll<SVGPathElement, (typeof links)[number]>("path.logic-link")
    .data(links, (entry) => `${entry.link.source}->${entry.link.target}`)
    .join("path")
    .attr("class", "logic-link")
    .attr("data-relation", (entry) => entry.link.relation ?? null)
    .attr("data-hidden", (entry) => (entry.hidden ? "true" : null))
    .attr("data-active", (entry) =>
      entry.touched && !entry.hidden ? "true" : null,
    )
    .attr("d", (entry) => {
      const source = byId.get(entry.link.source) as SimNode;
      const target = byId.get(entry.link.target) as SimNode;
      return curve(source, target);
    });

  const stubs = rect
    ? props.external
        .filter((stub) => byId.has(stub.source) && stub.targets.length > 0)
        .map((stub) => {
          const node = byId.get(stub.source) as SimNode;
          const x = stub.side === "left" ? rect.left : rect.right;
          return { stub, node, x, y: Math.min(Math.max(node.y, rect.top + 10), rect.bottom - 10) };
        })
    : [];

  const stubJoin = linkLayer
    .selectAll<SVGGElement, (typeof stubs)[number]>("g.logic-stub")
    .data(stubs, (entry) => entry.stub.source)
    .join((enter) => {
      const group = enter.append("g").attr("class", "logic-stub");
      group.append("path").attr("class", "logic-stub__line");
      group.append("rect").attr("class", "logic-stub__diamond");
      group.append("text").attr("class", "logic-stub__label");
      return group;
    })
    .attr("data-side", (entry) => entry.stub.side);

  stubJoin.each(function (entry) {
    const group = d3.select(this);
    const exitX = entry.x;
    group
      .select(".logic-stub__line")
      .attr("d", `M${entry.node.x},${entry.node.y} L${exitX},${entry.y}`);
    const size = 4.5;
    group
      .select(".logic-stub__diamond")
      .attr("x", exitX - size)
      .attr("y", entry.y - size)
      .attr("width", size * 2)
      .attr("height", size * 2)
      .attr("transform", `rotate(45 ${exitX} ${entry.y})`);
    // One reference is named; several are counted, so the names never collide.
    const count = entry.stub.targets.length;
    const text = count === 1 ? truncate(entry.stub.targets[0].label, 16) : `${count}+`;
    group
      .select(".logic-stub__label")
      .attr("x", entry.stub.side === "left" ? exitX - 9 : exitX + 9)
      .attr("y", entry.y + 3)
      .attr("text-anchor", entry.stub.side === "left" ? "end" : "start")
      .text(text);

    group
      .on("pointerenter", (event: PointerEvent) => showEdgeCard(event, entry.stub))
      .on("pointerleave", () => {
        edgeCard.value = null;
      });
  });
}

/** Smooth connection, bottom to top when the target sits below. */
function curve(source: SimNode, target: SimNode): string {
  const down = target.y >= source.y;
  const sy = source.y + (down ? halfHeight(source) : -halfHeight(source));
  const ty = target.y + (down ? -halfHeight(target) : halfHeight(target));
  const distance = Math.max(Math.abs(ty - sy) * 0.55, 24);
  const c1 = down ? sy + distance : sy - distance;
  const c2 = down ? ty - distance : ty + distance;
  return `M${source.x},${sy} C${source.x},${c1} ${target.x},${c2} ${target.x},${ty}`;
}

function paintNodes() {
  if (!nodeLayer) return;

  nodeGroups = nodeLayer
    .selectAll<SVGGElement, SimNode>("g.logic-node")
    .data(currentNodes, (node) => node.id)
    .join((enter) => {
      const group = enter.append("g").attr("class", "logic-node");

      const theory = group.filter((node) => node.kind === "theory");
      theory.append("rect").attr("class", "logic-theory__box");
      theory.append("text").attr("class", "logic-theory__label");
      theory.append("text").attr("class", "logic-theory__detail");

      const pill = group.filter((node) => node.kind !== "theory");
      pill.append("rect").attr("class", "logic-pill__box");
      pill.append("rect").attr("class", "logic-pill__bar");
      pill.append("text").attr("class", "logic-pill__label");
      pill.append("text").attr("class", "logic-pill__status");
      return group;
    })
    .attr("data-kind", (node) => node.kind)
    .attr("data-status", (node) => node.status)
    .attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null))
    .attr("data-drop", (node) =>
      dropTheory.value !== null && node.key === dropTheory.value ? "true" : null,
    )
    .attr("transform", (node) => `translate(${node.x},${node.y})`);

  nodeGroups
    .select<SVGRectElement>(".logic-theory__box")
    .attr("x", (node) => -theoryBox(node).w / 2)
    .attr("y", -THEORY_H / 2)
    .attr("width", (node) => theoryBox(node).w)
    .attr("height", THEORY_H);

  nodeGroups
    .select<SVGTextElement>(".logic-theory__label")
    .attr("x", 0)
    .attr("y", -2)
    .attr("text-anchor", "middle")
    .text((node) => truncate(node.label, 26));

  nodeGroups
    .select<SVGTextElement>(".logic-theory__detail")
    .attr("x", 0)
    .attr("y", 12)
    .attr("text-anchor", "middle")
    .text((node) => node.detail ?? "");

  nodeGroups
    .select<SVGRectElement>(".logic-pill__box")
    .attr("x", (node) => -halfWidth(node))
    .attr("y", -PILL_H / 2)
    .attr("width", (node) => halfWidth(node) * 2)
    .attr("height", PILL_H);

  nodeGroups
    .select<SVGRectElement>(".logic-pill__bar")
    .attr("x", (node) => -halfWidth(node) + 0.5)
    .attr("y", -PILL_H / 2 + 0.5)
    .attr("width", 2)
    .attr("height", PILL_H - 1);

  nodeGroups
    .select<SVGTextElement>(".logic-pill__label")
    .attr("x", (node) => -halfWidth(node) + 10)
    .attr("y", 3.5)
    .text((node) => truncate(node.label, 26));

  nodeGroups
    .select<SVGTextElement>(".logic-pill__status")
    .attr("x", (node) => halfWidth(node) - 9)
    .attr("y", 4)
    .attr("text-anchor", "middle")
    .text((node) => (node.status === "proven" ? "✓" : node.status === "sorry" ? "!" : node.status ? "✕" : ""));

  nodeGroups
    .on("pointerdown", (event: PointerEvent, node: SimNode) => {
      if (event.button !== 0) return;
      event.stopPropagation();
      press(event, node);
      startDrag(node, event);
    })
    .on("pointerup", (event: PointerEvent, node: SimNode) => release(event, node))
    .on("pointerenter", (event: PointerEvent, node: SimNode) => showCard(event, node))
    .on("pointerleave", () => hideCard())
    .on("contextmenu", (event: PointerEvent, node: SimNode) => {
      if (node.kind !== "theory" && node.kind !== "axiom") return;
      event.preventDefault();
      event.stopPropagation();
      openTargetMenu(event, node.kind, node.kind === "theory" ? node.key : node.id, node.label);
    });
}

// ----------------------------------------------------------------- dragging

let dragNodes: { node: SimNode; x: number; y: number }[] = [];
let beforeDrag = new Map<string, readonly [number, number]>();
let dragStart = { x: 0, y: 0 };
let layerDrag: { id: string; offset: number } | null = null;

function startDrag(node: SimNode, event: PointerEvent) {
  const origin = pointerInGraph(event);
  if (!origin) return;
  dragStart = origin;
  beforeDrag = new Map(currentNodes.map((entry) => [entry.id, [entry.x, entry.y] as const]));
  // Dragging a node that is part of a selection drags the whole selection, which
  // is also what a drop on a theory moves.
  const selected = new Set(props.selectedIds);
  const group = selected.has(node.id) && selected.size > 1
    ? currentNodes.filter((entry) => selected.has(entry.id))
    : [node];
  dragNodes = group.map((entry) => ({ node: entry, x: entry.x, y: entry.y }));
  nodeLayer?.selectAll("g.logic-node").raise();
  // A term can be dragged onto a theory node to join it.
  const movable = node.kind !== "theory";
  const assignable = group.filter((entry) => entry.kind !== "theory").map((entry) => entry.id);

  const move = (moveEvent: PointerEvent) => {
    const point = pointerInGraph(moveEvent);
    if (!point) return;
    const dx = point.x - dragStart.x;
    const dy = point.y - dragStart.y;
    for (const entry of dragNodes) {
      entry.node.x = entry.x + dx;
      entry.node.y = entry.y + dy;
    }
    if (movable && assignable.length > 0) {
      const hit = theoryAt(point, node.id);
      const key = hit?.key ?? null;
      if (key !== dropTheory.value) {
        dropTheory.value = key;
        draw();
      }
    }
    draw();
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    const target = dropTheory.value;
    dropTheory.value = null;
    if (movable && target) {
      // The term is about to belong to another theory: put it back where it was
      // and let Rust re-place it in its new home, instead of saving this drag.
      for (const entry of dragNodes) {
        entry.node.x = entry.x;
        entry.node.y = entry.y;
      }
      beforeDrag = new Map();
      dragNodes = [];
      emit("assign", assignable, target);
      draw();
      return;
    }
    endDrag();
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

/** The theory node under a point in graph coordinates, ignoring `except`. */
function theoryAt(
  point: { x: number; y: number },
  except?: string,
): SimNode | null {
  for (const node of [...currentNodes].reverse()) {
    if (node.kind !== "theory") continue;
    if (except && node.id === except) continue;
    if (
      Math.abs(point.x - node.x) <= halfWidth(node) &&
      Math.abs(point.y - node.y) <= halfHeight(node)
    ) {
      return node;
    }
  }
  return null;
}

function endDrag() {
  const moved: FixedNode[] = [];
  for (const node of currentNodes) {
    const before = beforeDrag.get(node.id);
    if (!before || before[0] !== node.x || before[1] !== node.y) {
      moved.push({ id: node.key, x: node.x, y: node.y });
    }
  }
  beforeDrag = new Map();
  dragNodes = [];
  if (moved.length > 0) emit("move", moved);
}

function startLayerDrag(event: PointerEvent, layer: Layer) {
  const origin = pointerInGraph(event);
  if (!origin) return;
  layerDrag = { id: layer.id, offset: origin.y - layer.y };
  const move = (moveEvent: PointerEvent) => {
    const point = pointerInGraph(moveEvent);
    if (!point || !layerDrag) return;
    const value = point.y - layerDrag.offset;
    layer.y = value;
    paintLayers();
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    const id = layerDrag?.id;
    layerDrag = null;
    if (id) emit("move-layer", id, layer.y);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  event.stopPropagation();
}

/** Click versus double click, without relying on the browser's `click`. */
let pressed: { id: string; x: number; y: number; moved: boolean } | null = null;
let lastPick = { id: "", time: 0 };
/** When the canvas background was last clicked, for stepping out of a theory. */
let lastBackgroundClick = 0;

function press(event: PointerEvent, node: SimNode) {
  pressed = { id: node.id, x: event.clientX, y: event.clientY, moved: false };
  const track = (move: PointerEvent) => {
    if (!pressed) return;
    if (Math.hypot(move.clientX - pressed.x, move.clientY - pressed.y) > 4) pressed.moved = true;
  };
  window.addEventListener("pointermove", track);
  window.addEventListener("pointerup", () => window.removeEventListener("pointermove", track), {
    once: true,
  });
}

function release(event: PointerEvent, node: SimNode) {
  const state = pressed;
  pressed = null;
  if (!state || state.id !== node.id || state.moved) return;
  const additive = event.ctrlKey || event.metaKey || event.shiftKey;
  const now = Date.now();
  // A double-click opens, modifier or not: the page reads the modifier from the
  // event to decide between another editor and the pane the user last touched.
  if (lastPick.id === node.id && now - lastPick.time < 380) {
    lastPick = { id: "", time: 0 };
    if (node.kind === "theory") {
      // The board is about to change underneath: no lingering hover.
      hideCard();
      edgeCard.value = null;
      emit("open-theory", node.key);
    } else {
      emit("open", node.id, event);
    }
    return;
  }
  lastPick = { id: node.id, time: now };
  // Clicking inside a multi-selection keeps it: the drag that follows moves it.
  if (!additive && props.selectedIds.length > 1 && props.selectedIds.includes(node.id)) return;
  emit("pick", node.id, additive);
}

/**
 * Where a pointer event lands in graph coordinates.
 *
 * Read from the zoom transform rather than through the SVG's screen matrix: the view
 * is moved with a CSS transform (see `applyTransform`), and this is both independent
 * of how it was applied and cheaper than asking the browser for a matrix on every
 * pointer move.
 */
function pointerInGraph(event: PointerEvent | MouseEvent): { x: number; y: number } | null {
  const element = svg?.node();
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  const transform = d3.zoomTransform(element);
  return {
    x: (event.clientX - rect.left - transform.x) / transform.k,
    y: (event.clientY - rect.top - transform.y) / transform.k,
  };
}

function onBackgroundDown(event: PointerEvent) {
  if (event.button !== 0 || pans(event)) return;
  const origin = pointerInGraph(event);
  if (!origin || !root) return;
  // Shift keeps what is selected: the rectangle then adds to it.
  const additive = event.shiftKey;
  toolsOpen.value = false;
  target.value = null;

  rubber = root
    .append("rect")
    .attr("class", "logic__rubber")
    .attr("x", origin.x)
    .attr("y", origin.y)
    .attr("width", 0)
    .attr("height", 0);

  const move = (moveEvent: PointerEvent) => {
    const point = pointerInGraph(moveEvent);
    if (!rubber || !point) return;
    rubber
      .attr("x", Math.min(origin.x, point.x))
      .attr("y", Math.min(origin.y, point.y))
      .attr("width", Math.abs(point.x - origin.x))
      .attr("height", Math.abs(point.y - origin.y));
  };

  const up = (upEvent: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    const point = pointerInGraph(upEvent) ?? origin;
    rubber?.remove();
    rubber = null;
    const left = Math.min(origin.x, point.x);
    const right = Math.max(origin.x, point.x);
    const top = Math.min(origin.y, point.y);
    const bottom = Math.max(origin.y, point.y);
    if (right - left < 4 && bottom - top < 4) {
      // A click, not a drag: a second one in quick succession steps out of the
      // theory, otherwise the click clears the selection (unless shift asked to
      // keep it).
      const now = Date.now();
      const isDouble = now - lastBackgroundClick < 380;
      lastBackgroundClick = isDouble ? 0 : now;
      if (isDouble && onBackgroundDouble(origin)) return;
      if (!additive) emit("select", []);
      return;
    }
    const hits = currentNodes
      .filter((node) => node.x >= left && node.x <= right && node.y >= top && node.y <= bottom)
      .map((node) => node.id);
    emit("select", additive ? [...new Set([...props.selectedIds, ...hits])] : hits);
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  event.preventDefault();
}

// ------------------------------------------------------------------- menus

function openTools(at?: { x: number; y: number }) {
  toolsAt.value = at ?? null;
  toolsOpen.value = true;
  target.value = null;
}

/**
 * Double click on the canvas outside the theory's rectangle: step back to the
 * parent graph. Inside the rectangle nothing happens — that is the theory's own
 * space.
 *
 * The browser's `dblclick` never arrives here: `pointerdown` on the canvas calls
 * `preventDefault` for the rubber band, so clicks are counted by hand, the same
 * way a node's clicks are.
 */
function onBackgroundDouble(point: { x: number; y: number }): boolean {
  if (!props.theory || props.trail.length < 2) return false;
  const rect = contentRect();
  const inside =
    rect !== null &&
    point.x >= rect.left &&
    point.x <= rect.right &&
    point.y >= rect.top &&
    point.y <= rect.bottom;
  if (inside) return false;
  const parent = props.trail[props.trail.length - 2];
  hideCard();
  edgeCard.value = null;
  emit("crumb", parent?.key ?? "");
  return true;
}

function onContextMenu(event: MouseEvent) {
  // A node or divider stops propagation, so this is the empty canvas.
  event.preventDefault();
  if (!host.value) return;
  const bounds = host.value.getBoundingClientRect();
  openTools({ x: event.clientX - bounds.left, y: event.clientY - bounds.top });
}

function openTargetMenu(
  event: PointerEvent,
  kind: "theory" | "layer" | "axiom",
  id: string,
  name: string,
) {
  if (!host.value) return;
  const bounds = host.value.getBoundingClientRect();
  target.value = {
    kind,
    id,
    name,
    x: event.clientX - bounds.left,
    y: event.clientY - bounds.top,
  };
  toolsOpen.value = false;
  card.value = null;
  edgeCard.value = null;
}

function runTool(action: "theory" | "layer" | "axiom" | "settings") {
  toolsOpen.value = false;
  if (action === "theory") emit("create-theory");
  else if (action === "layer") emit("add-layer");
  else if (action === "axiom") emit("add-axiom");
  else emit("settings");
}

function askRename() {
  const current = target.value;
  target.value = null;
  if (!current) return;
  if (current.kind === "theory") emit("rename-theory", current.id);
  else emit("rename-layer", current.id);
}

function confirmDelete() {
  const current = target.value;
  target.value = null;
  if (!current) return;
  if (current.kind === "theory") emit("delete-theory", current.id);
  else if (current.kind === "axiom") {
    // Removing a declaration rewrites a file: ask first, and say so.
    const node = currentNodes.find((entry) => entry.id === current.id);
    confirmAxiom.value = { id: current.id, label: current.name, path: node?.path ?? null };
  } else emit("delete-layer", current.id);
}

function dropAxiom() {
  const current = confirmAxiom.value;
  confirmAxiom.value = null;
  if (current) emit("delete-axiom", current.id);
}

// --------------------------------------------------------- drag and drop

function dropTargetAt(event: DragEvent): SimNode | null {
  const point = pointerInGraph(event);
  if (!point) return null;
  return theoryAt(point);
}

function onDragOver(event: DragEvent) {
  if (!event.dataTransfer) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "move";
  const node = dropTargetAt(event);
  dropTheory.value = node ? node.key : null;
  draw();
}

function onDragLeave() {
  if (dropTheory.value === null) return;
  dropTheory.value = null;
  draw();
}

function onDrop(event: DragEvent) {
  event.preventDefault();
  const node = dropTargetAt(event);
  dropTheory.value = null;
  const payload = event.dataTransfer?.getData("text/plain") ?? "";
  const ids = payload.split("\n").filter(Boolean);
  if (!node || ids.length === 0) {
    draw();
    return;
  }
  emit("assign", ids, node.key);
  draw();
}

// --------------------------------------------------------------- hover card

let hoverTimer: number | null = null;

function showCard(event: PointerEvent, node: SimNode) {
  if (dragNodes.length > 0) return;
  if (hoverTimer !== null) window.clearTimeout(hoverTimer);
  if (card.value && card.value.id !== node.id) card.value = null;
  const bounds = host.value?.getBoundingClientRect();
  if (!bounds) return;
  hoverTimer = window.setTimeout(() => {
    hoverTimer = null;
    const info = props.hoverInfo;
    card.value = {
      id: node.id,
      label: node.label,
      kind: node.kind,
      status: node.status,
      detail: node.detail,
      path: node.path,
      namespace: namespaceOf(node),
      comments: info && info.id === node.id ? (info.comments ?? "").trim() : "",
      x: Math.min(Math.max(8, event.clientX - bounds.left + 14), bounds.width - 210),
      y: Math.min(Math.max(8, event.clientY - bounds.top + 16), bounds.height - 96),
    };
    emit("hover", node.id);
  }, 160);
}

function hideCard() {
  if (hoverTimer !== null) {
    window.clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  if (!card.value) return;
  card.value = null;
  emit("hover", null);
}

// ---------------------------------------------------------------- viewport

let cameraTimer: number | null = null;
let cameraApplied = false;

/**
 * The view transform, applied as a CSS transform once per animation frame.
 *
 * Writing the SVG `transform` attribute per zoom event makes the browser lay the
 * whole board out again; a CSS transform is composited, so the picture is moved
 * instead of rebuilt. One per frame keeps a fast wheel from queueing dozens.
 */
let zoomFrame = 0;
let zoomPending: d3.ZoomTransform | null = null;

function applyTransform(transform: d3.ZoomTransform) {
  zoomPending = transform;
  if (zoomFrame !== 0) return;
  zoomFrame = window.requestAnimationFrame(() => {
    zoomFrame = 0;
    const next = zoomPending;
    zoomPending = null;
    if (!next || !root) return;
    root.attr("transform", null);
    root.style("transform-origin", "0 0");
    root.style("transform", `translate(${next.x}px, ${next.y}px) scale(${next.k})`);
  });
}

function scheduleCamera() {
  if (cameraTimer !== null) return;
  cameraTimer = window.setTimeout(() => {
    cameraTimer = null;
    const element = svg?.node();
    if (!element) return;
    const transform = d3.zoomTransform(element);
    emit("camera", { x: transform.x, y: transform.y, k: transform.k });
  }, 260);
}

function applyCameraOnce() {
  if (cameraApplied || !svg || !zoom) return;
  cameraApplied = true;
  if (props.camera) {
    svg.call(zoom.transform, d3.zoomIdentity.translate(props.camera.x, props.camera.y).scale(props.camera.k));
    return;
  }
  const rect = fitBounds();
  if (!rect || !host.value) return;
  const width = host.value.clientWidth;
  const height = host.value.clientHeight;
  const raw = Math.min(width / (rect.right - rect.left + 120), height / (rect.bottom - rect.top + 120));
  const scale = Math.max(0.35, Math.min(1, raw));
  const centreX = (rect.left + rect.right) / 2;
  const centreY = (rect.top + rect.bottom) / 2;
  svg.call(
    zoom.transform,
    d3.zoomIdentity.translate(width / 2, height / 2).scale(scale).translate(-centreX, -centreY),
  );
}

onMounted(() => {
  guard(() => {
    mount();
    render();
  });
});

/**
 * Entering or leaving a theory swaps every node at once. A short fade keeps that
 * from reading as a flash: the old picture dims, the new one comes back in.
 */
watch(
  () => props.theory,
  (next, previous) => {
    if (next === previous) return;
    fading.value = true;
    window.setTimeout(() => {
      fading.value = false;
    }, 90);
  },
);

onBeforeUnmount(() => {
  observer?.disconnect();
  if (zoomFrame !== 0) window.cancelAnimationFrame(zoomFrame);
  zoomFrame = 0;
  svg?.remove();
  svg = null;
  root = null;
});

watch(
  () => [props.nodes, props.links, props.layers, props.external],
  () =>
    guard(() => {
      // A different graph opens with a different framing.
      cameraApplied = false;
      render();
    }),
);

watch(
  () => props.selectedIds,
  () => draw(),
);

watch(
  () => props.theory,
  () => {
    cameraApplied = false;
  },
);
</script>

<template>
  <div class="logic">
    <div class="logic__crumbs">
      <button
        v-for="(step, index) in trail"
        :key="step.key || 'root'"
        type="button"
        class="logic__crumb"
        :data-current="index === trail.length - 1 || undefined"
        :title="index === trail.length - 1 ? step.name : `back to ${step.name}`"
        @click="emit('crumb', step.key)"
      >
        <UiIcon v-if="index > 0" name="chevron" :size="8" class="logic__crumb-sep" />
        <span>{{ step.name }}</span>
      </button>
      <span class="k-spacer" />
      <span class="logic__count k-mono">{{ nodes.length }} nodes · {{ links.length }} links</span>
    </div>

    <div
      ref="host"
      class="logic__host"
      :data-fading="fading || undefined"
      @pointerdown.self="hideCard()"
    />

    <div class="logic__tools">
      <button class="logic__tool" type="button" title="Settings" @click="emit('settings')">
        <UiIcon name="settings" :size="12" />
      </button>
    </div>

    <div
      v-if="toolsOpen"
      class="logic__menu"
      :style="toolsAt ? { left: `${toolsAt.x}px`, top: `${toolsAt.y}px`, right: 'auto' } : undefined"
      @pointerdown.stop
    >
      <button class="logic__menu-item" type="button" @click="runTool('theory')">
        <UiIcon name="plus" :size="11" />
        <span>Create Theory</span>
      </button>
      <button class="logic__menu-item" type="button" @click="runTool('axiom')">
        <UiIcon name="file-plus" :size="11" />
        <span>Add Axiom</span>
      </button>
      <button
        class="logic__menu-item"
        type="button"
        :disabled="!insideTheory"
        :title="insideTheory ? 'Add a layer divider' : 'Open a theory first'"
        @click="runTool('layer')"
      >
        <UiIcon name="layers" :size="11" />
        <span>Add Layer</span>
      </button>
      <button class="logic__menu-item" type="button" @click="runTool('settings')">
        <UiIcon name="settings" :size="11" />
        <span>Settings</span>
      </button>
      <p class="logic__menu-note">
        Drag a term from the logic tree onto a theory to move it. Double click a theory to open it.
      </p>
    </div>

    <div
      v-if="target"
      class="logic__menu logic__menu--small"
      :style="{ left: `${target.x}px`, top: `${target.y}px` }"
      @pointerdown.stop
    >
      <button
        v-if="target.kind !== 'axiom'"
        class="logic__menu-item"
        type="button"
        @click="askRename"
      >
        <UiIcon name="code" :size="11" />
        <span>Rename {{ target.kind }}</span>
      </button>
      <button class="logic__menu-item" type="button" @click="confirmDelete">
        <UiIcon name="trash" :size="11" />
        <span>Delete {{ target.kind }}</span>
      </button>
    </div>

    <div
      v-if="card"
      class="logic__card"
      :style="{ left: `${card.x}px`, top: `${card.y}px` }"
    >
      <div class="logic__card-head">
        <span class="logic__card-title">{{ card.label }}</span>
        <span class="logic__card-kind">{{ card.kind === "theory" ? "theory" : card.kind }}</span>
      </div>
      <p v-if="card.comments" class="logic__card-comments">{{ card.comments }}</p>
      <dl class="logic__card-rows">
        <dt>kind</dt>
        <dd>{{ card.kind }}</dd>
        <template v-if="card.namespace">
          <dt>ns</dt>
          <dd>{{ card.namespace }}</dd>
        </template>
        <template v-if="card.detail && card.kind === 'theory'">
          <dt>holds</dt>
          <dd>{{ card.detail }}</dd>
        </template>
        <template v-if="card.path">
          <dt>file</dt>
          <dd>{{ card.path }}</dd>
        </template>
        <template v-if="card.status">
          <dt>state</dt>
          <dd :data-status="card.status">{{ card.status }}</dd>
        </template>
      </dl>
    </div>

    <div
      v-if="edgeCard"
      class="logic__card logic__card--edge"
      :style="{ left: `${edgeCard.x}px`, top: `${edgeCard.y}px` }"
    >
      <div class="logic__card-head">
        <span class="logic__card-title">{{ edgeCard.targets.length }} external reference{{ edgeCard.targets.length === 1 ? "" : "s" }}</span>

      </div>
      <ul class="logic__refs">
        <li v-for="entry in edgeCard.targets" :key="entry.label" class="logic__ref">
          <span class="logic__ref-kind">{{ entry.kind }}</span>
          <span class="logic__ref-label">{{ entry.namespace ? `${entry.namespace}.` : "" }}{{ entry.label }}</span>
        </li>
      </ul>
    </div>

    <div v-if="confirmAxiom" class="logic__confirm-backdrop" @pointerdown.stop="confirmAxiom = null">
      <div class="logic__confirm" @pointerdown.stop>
        <p class="logic__confirm-title">Delete axiom {{ confirmAxiom.label }}?</p>
        <p class="logic__confirm-text">
          It will be removed from
          <span class="k-mono">{{ confirmAxiom.path ?? "its file" }}</span
          >. Nothing else in the file changes.
        </p>
        <div class="logic__confirm-bar">
          <span class="k-spacer" />
          <button class="logic__menu-item logic__confirm-cancel" type="button" @click="confirmAxiom = null">
            cancel
          </button>
          <button class="logic__menu-item logic__confirm-ok" type="button" @click="dropAxiom">
            <UiIcon name="trash" :size="11" />
            <span>delete</span>
          </button>
        </div>
      </div>
    </div>

    <div v-if="nodes.length === 0" class="logic__empty">
      <span class="k-mono">
        {{ insideTheory ? "this theory has no terms yet" : "no theories yet — create one" }}
      </span>
    </div>

    <div v-if="graphError" class="logic__error">
      <span class="logic__error-text">{{ graphError }}</span>
    </div>
  </div>
</template>

<style scoped>
/* The zoom layer keeps its own compositing layer: moving it is then a transform of
   a picture the browser already has, not a re-layout of every node and link. */
.logic :deep(.logic__root) {
  will-change: transform;
}

.logic {
  position: relative;
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background:
    radial-gradient(1200px 500px at 50% -10%, rgba(106, 169, 196, 0.045), transparent 70%),
    var(--bg-1);
}

.logic__crumbs {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
  height: 27px;
  padding: 0 10px;
  border-bottom: 1px solid var(--line);
  background: var(--grad-header);
}

.logic__crumb {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 5px;
  border: 0;
  border-radius: 3px;
  background: transparent;
  color: var(--text-faint);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: color var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
}

.logic__crumb:hover {
  color: var(--text);
  background: rgba(255, 255, 255, 0.04);
}

.logic__crumb[data-current] {
  color: var(--text);
  cursor: default;
}

.logic__crumb-sep {
  color: var(--text-ghost);
  transform: rotate(0deg);
}

.logic__count {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.logic__host {
  position: absolute;
  inset: 27px 0 0;
  transition: opacity 150ms var(--ease);
}

.logic__host[data-fading] {
  opacity: 0.15;
}

.logic__host :deep(svg) {
  display: block;
}

.logic__tools {
  position: absolute;
  top: 33px;
  right: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.logic__tool {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
  color: var(--text-faint);
  cursor: pointer;
  transition: color var(--t) var(--ease), border-color var(--t) var(--ease),
    background var(--t) var(--ease);
}

.logic__tool:hover {
  color: var(--text);
  border-color: var(--line-strong);
  background: var(--bg-2);
}

.logic__menu {
  position: absolute;
  top: 33px;
  right: 32px;
  width: 214px;
  padding: 4px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  background: linear-gradient(180deg, rgba(24, 28, 31, 0.98), rgba(16, 19, 21, 0.98));
  box-shadow: 0 10px 24px rgba(0, 0, 0, 0.5);
  z-index: 4;
}

.logic__menu--small {
  top: auto;
  right: auto;
  width: 150px;
}

.logic__menu-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 7px;
  border: 0;
  border-radius: 3px;
  background: transparent;
  color: var(--text-dim);
  font: inherit;
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
  transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease);
}

.logic__menu-item:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.05);
  color: var(--text);
}

.logic__menu-item:disabled {
  color: var(--text-ghost);
  cursor: default;
}

.logic__menu-note {
  margin: 4px 7px 5px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  line-height: 1.45;
}

.logic__card {
  position: absolute;
  z-index: 3;
  width: 196px;
  padding: 7px 9px 8px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  background: linear-gradient(180deg, rgba(24, 28, 31, 0.98), rgba(15, 18, 20, 0.98));
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.5);
  pointer-events: none;
}

.logic__card-head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding-bottom: 5px;
  border-bottom: 1px solid var(--line-soft);
}

.logic__card-title {
  flex: 1;
  overflow: hidden;
  color: var(--text);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logic__card-kind {
  flex: none;
  color: var(--accent);
  font-family: var(--mono);
  font-size: var(--fs-micro);
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.logic__card-rows {
  display: grid;
  grid-template-columns: 46px 1fr;
  gap: 2px 6px;
  margin: 6px 0 0;
  font-size: var(--fs-micro);
}

.logic__card-rows dt {
  color: var(--text-ghost);
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.logic__card-rows dd {
  margin: 0;
  overflow: hidden;
  color: var(--text-dim);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logic__card-rows dd[data-status="proven"] {
  color: var(--ok);
}

.logic__card-rows dd[data-status="sorry"] {
  color: var(--warning);
}

.logic__card-rows dd[data-status="error"] {
  color: var(--danger);
}

.logic__card-comments {
  margin: 6px 0 0;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--line-soft);
  display: -webkit-box;
  overflow: hidden;
  color: var(--text-faint);
  font-family: var(--mono);
  font-size: var(--fs-micro);
  line-height: 1.5;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 4;
  white-space: pre-wrap;
}

/* Hover of a diamond: what a node refers to outside its own graph. */
.logic__card--edge {
  width: 232px;
}

.logic__refs {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin: 6px 0 0;
  padding: 0;
  list-style: none;
  max-height: 148px;
  overflow: hidden;
}

.logic__ref {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: var(--fs-micro);
}

.logic__ref-kind {
  flex: none;
  width: 54px;
  overflow: hidden;
  color: var(--text-ghost);
  font-family: var(--mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logic__ref-label {
  overflow: hidden;
  color: var(--text-dim);
  font-family: var(--mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logic__empty,
.logic__error {
  position: absolute;
  inset: 27px 0 0;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  letter-spacing: 0.04em;
}

.logic__error {
  align-items: flex-end;
  justify-content: flex-start;
  padding: 10px;
}

.logic__error-text {
  color: var(--danger);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  letter-spacing: 0;
}

/* D3-created elements need :deep selectors. */
.logic__host :deep(.logic-rect) {
  fill: rgba(106, 169, 196, 0.022);
  stroke: #2b3339;
  stroke-width: 1;
  pointer-events: none;
}

.logic__host :deep(.logic-rect__title) {
  fill: #6f7a83;
  font-family: var(--mono);
  font-size: 10px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  pointer-events: none;
}

.logic__host :deep(.logic-layer) {
  cursor: ns-resize;
}

.logic__host :deep(.logic-layer__line) {
  stroke: #3b454c;
  stroke-width: 1;
  stroke-dasharray: 1 4;
}

.logic__host :deep(.logic-layer:hover .logic-layer__line),
.logic__host :deep(.logic-layer[data-selected="true"] .logic-layer__line) {
  stroke: var(--accent-line);
}

.logic__host :deep(.logic-layer__grip) {
  fill: transparent;
  pointer-events: all;
}

.logic__host :deep(.logic-layer__label) {
  fill: #7b858d;
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  pointer-events: none;
}

.logic__host :deep(.logic-node) {
  cursor: pointer;
}

.logic__host :deep(.logic-theory__box) {
  fill: rgba(106, 169, 196, 0.1);
  stroke: rgba(106, 169, 196, 0.42);
  stroke-width: 1;
  rx: 2;
  transition: fill var(--t) var(--ease), stroke var(--t) var(--ease);
}

.logic__host :deep(.logic-node[data-kind="theory"]:hover .logic-theory__box) {
  fill: rgba(106, 169, 196, 0.17);
  stroke: var(--accent-line);
}

.logic__host :deep(.logic-node[data-selected="true"] .logic-theory__box) {
  fill: rgba(106, 169, 196, 0.2);
  stroke: var(--accent-bright);
}

/* A theory a term is about to be dropped into: deliberately not the selection
   colour, so "this is selected" and "this is the drop target" never read the same. */
.logic__host :deep(.logic-node[data-drop="true"] .logic-theory__box) {
  fill: rgba(140, 214, 165, 0.3);
  stroke: #9be6b4;
  stroke-width: 2;
  stroke-dasharray: 5 3;
  filter: drop-shadow(0 0 7px rgba(140, 220, 165, 0.55));
}

.logic__host :deep(.logic-node[data-drop="true"] .logic-theory__label) {
  fill: #d8f5e4;
}

.logic__host :deep(.logic-node[data-drop="true"] .logic-theory__detail) {
  fill: #a9d8bd;
}

.logic__host :deep(.logic-theory__label) {
  fill: #c3d3da;
  font-family: var(--mono);
  font-size: 10.5px;
  pointer-events: none;
}

.logic__host :deep(.logic-theory__detail) {
  fill: #71818a;
  font-family: var(--mono);
  font-size: 8.5px;
  pointer-events: none;
}

.logic__host :deep(.logic-pill__box) {
  fill: #161a1d;
  stroke: #262c30;
  stroke-width: 1;
  rx: 2;
  transition: fill var(--t) var(--ease), stroke var(--t) var(--ease);
}

.logic__host :deep(.logic-node[data-kind="theory"]) .logic-theory__box {
  rx: 3;
}

.logic__host :deep(.logic-node:hover .logic-pill__box) {
  fill: #1b2024;
  stroke: #333a3f;
}

.logic__host :deep(.logic-node[data-selected="true"] .logic-pill__box) {
  fill: #1a2328;
  stroke: var(--accent-bright);
}

.logic__host :deep(.logic-node[data-selected="true"] .logic-pill__label) {
  fill: var(--accent-bright);
}

.logic__host :deep(.logic-pill__bar) {
  fill: #6b757c;
}

.logic__host :deep(.logic-node[data-kind="def"] .logic-pill__bar) {
  fill: #8fb8a4;
}

.logic__host :deep(.logic-node[data-kind="theorem"] .logic-pill__bar),
.logic__host :deep(.logic-node[data-kind="lemma"] .logic-pill__bar) {
  fill: #a8a0cf;
}

.logic__host :deep(.logic-node[data-kind="axiom"] .logic-pill__bar) {
  fill: #c9a86b;
}

/* A theorem says how it stands at a glance: the bar takes the proof state. */
.logic__host :deep(.logic-node[data-kind="theorem"][data-status="proven"] .logic-pill__bar),
.logic__host :deep(.logic-node[data-kind="lemma"][data-status="proven"] .logic-pill__bar) {
  fill: var(--ok);
}

.logic__host :deep(.logic-node[data-kind="theorem"][data-status="sorry"] .logic-pill__bar),
.logic__host :deep(.logic-node[data-kind="lemma"][data-status="sorry"] .logic-pill__bar) {
  fill: var(--warning);
}

.logic__host :deep(.logic-node[data-kind="theorem"][data-status="error"] .logic-pill__bar),
.logic__host :deep(.logic-node[data-kind="lemma"][data-status="error"] .logic-pill__bar) {
  fill: var(--danger);
}

.logic__host :deep(.logic-node[data-kind="structure"] .logic-pill__bar) {
  fill: #c9a96b;
}

.logic__host :deep(.logic-pill__label) {
  fill: #b9c2c9;
  font-family: var(--mono);
  font-size: 10px;
  pointer-events: none;
}

.logic__host :deep(.logic-pill__status) {
  fill: var(--ok);
  font-family: var(--mono);
  font-size: 11px;
  pointer-events: none;
}

.logic__host :deep(.logic-node[data-status="sorry"] .logic-pill__status) {
  fill: var(--warning);
}

.logic__host :deep(.logic-node[data-status="error"] .logic-pill__status) {
  fill: var(--danger);
}

.logic__host :deep(.logic-pill__box) {
  filter: none;
}

.logic__host :deep(.logic-node[data-status="proven"] .logic-pill__box) {
  fill: rgba(124, 171, 136, 0.14);
  stroke: rgba(124, 171, 136, 0.42);
}

.logic__host :deep(.logic-node[data-status="sorry"] .logic-pill__box) {
  fill: rgba(194, 166, 112, 0.14);
  stroke: rgba(194, 166, 112, 0.42);
}

.logic__host :deep(.logic-node[data-status="error"] .logic-pill__box) {
  fill: rgba(201, 115, 107, 0.16);
  stroke: rgba(201, 115, 107, 0.48);
}

.logic__host :deep(.logic-link) {
  fill: none;
  stroke: #262e33;
  stroke-width: 1;
  transition: stroke var(--t) var(--ease);
}

.logic__host :deep(.logic-link[data-relation="axiom"]),
.logic__host :deep(.logic-link[data-relation="structure"]) {
  stroke: rgba(201, 168, 107, 0.45);
}

.logic__host :deep(.logic-link[data-relation="def"]) {
  stroke: rgba(143, 184, 164, 0.4);
}

.logic__host :deep(.logic-link[data-relation="theorem"]) {
  stroke: rgba(168, 160, 207, 0.42);
}

/* The selection pulls its links forward, in the same blue as a picked node. */
.logic__host :deep(path.logic-link[data-active="true"]) {
  stroke: var(--accent-bright);
  stroke-width: 1.7;
}

/* Out-of-layer links: shown only while a node they touch is selected, dashed
   and quieter, so the flow that matters keeps the eye. */
.logic__host :deep(path.logic-link[data-hidden="true"]) {
  stroke: rgba(140, 152, 160, 0.4);
  stroke-width: 1;
  stroke-dasharray: 3 4;
}

.logic__host :deep(.logic-stub__line) {
  fill: none;
  stroke: rgba(140, 152, 160, 0.42);
  stroke-width: 1;
  stroke-dasharray: 4 3;
}

.logic__host :deep(.logic-stub__diamond) {
  fill: #12171a;
  stroke: rgba(150, 163, 171, 0.6);
  stroke-width: 1;
  pointer-events: all;
  cursor: help;
}

.logic__host :deep(.logic-stub:hover .logic-stub__diamond) {
  stroke: var(--accent-bright);
}

.logic__host :deep(.logic-stub__label) {
  pointer-events: none;
}

.logic__host :deep(.logic-stub__label) {
  fill: #7d878e;
  font-family: var(--mono);
  font-size: 8.5px;
}

.logic__confirm-backdrop {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(8, 10, 12, 0.55);
  z-index: 5;
}

.logic__confirm {
  width: 320px;
  padding: 12px 13px 10px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  background: linear-gradient(180deg, rgba(24, 28, 31, 0.99), rgba(15, 18, 20, 0.99));
  box-shadow: 0 16px 34px rgba(0, 0, 0, 0.55);
}

.logic__confirm-title {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-sm);
}

.logic__confirm-text {
  margin: 6px 0 10px;
  color: var(--text-faint);
  font-size: var(--fs-micro);
  line-height: 1.55;
}

.logic__confirm-bar {
  display: flex;
  align-items: center;
  gap: 4px;
}

.logic__confirm-cancel,
.logic__confirm-ok {
  width: auto;
  color: var(--text-dim);
}

.logic__confirm-ok {
  color: var(--danger);
}

.logic__confirm-ok:hover {
  background: rgba(201, 115, 107, 0.14);
  color: var(--danger);
}

.logic__host :deep(.logic__rubber) {
  fill: rgba(106, 169, 196, 0.08);
  stroke: var(--accent-line);
  stroke-width: 1;
  stroke-dasharray: 3 3;
  pointer-events: none;
}
</style>
