<script setup lang="ts">
// Fixed-layout graph used by both boards.
//
// Every node has a position stored in `.klean.json` (Rust places new nodes next
// to their namespace without overlapping). There is no force simulation: the
// picture is stable, nodes move only when dragged, and dragging a group moves
// its whole contents with it. Links are routed orthogonally, like draw.io.
//
// Mouse: drag a node or a group header to move it, drag the background for a
// rubber-band selection, ctrl/cmd + drag to pan, wheel to zoom, click to select,
// double click to open the editor.
import * as d3 from "d3";
import { matchesShortcut } from "../editor/keys";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Camera, FixedNode, GraphLink, GraphNode, NodeDetail } from "../api/lean";
import {
  GROUP_HEADER,
  GROUP_PADDING,
  indexChildren,
  measureBoxes,
  ownBox,
  pillHalfWidth,
  type Box,
} from "../lib/graph-layout";
import UiIcon from "./ui/UiIcon.vue";

interface SimNode extends GraphNode {
  x: number;
  y: number;
}

const props = withDefaults(
  defineProps<{
    nodes: GraphNode[];
    links: GraphLink[];
    selectedIds: string[];
    /** Kinds drawn as rectangles; everything else is a pill. */
    groupKinds?: string[];
    /** Groups get a collapse control. */
    expandable?: boolean;
    /** Remembered view; when absent the content is fitted instead. */
    camera?: Camera | null;
    /** Loaded details of the hovered node (comments come from here). */
    hoverInfo?: NodeDetail | null;
    /** Kinds that show their proof state. Empty means "any node that has one". */
    statusKinds?: string[];
    /** Draw the status glyph. Off when the background already carries it. */
    statusIcon?: boolean;
    /** Held while dragging to look around, from `Settings → Editor`. */
    panKey?: string;
  }>(),
  {
    groupKinds: () => ["lib", "folder", "file", "astgroup"],
    expandable: true,
    statusIcon: true,
    camera: null,
    hoverInfo: null,
    statusKinds: () => [],
    panKey: "Ctrl",
  },
);

const emit = defineEmits<{
  pick: [string, boolean];
  select: [string[]];
  open: [string, PointerEvent];
  toggle: [string];
  move: [FixedNode[]];
  camera: [Camera];
  settings: [];
  hover: [string | null];
  bend: [key: string, value: number | null];
}>();

const host = ref<HTMLDivElement | null>(null);
const graphError = ref<string | null>(null);
const HEADER_H = GROUP_HEADER;

const kinds = computed(() => new Set(props.groupKinds));
const isGroup = (node: GraphNode) => kinds.value.has(node.kind);
const selected = computed(() => new Set(props.selectedIds));
const statusKinds = computed(() => new Set(props.statusKinds));

/** True when the node carries a proof state at all (it tints the background). */
function tinted(node: GraphNode): boolean {
  if (!node.status) return false;
  return statusKinds.value.size === 0 || statusKinds.value.has(node.kind);
}

/** True when that state is drawn as a glyph too. */
function showsStatus(node: GraphNode): boolean {
  return props.statusIcon && tinted(node);
}

function statusGlyph(status: string): string {
  if (status === "proven") return "✓";
  if (status === "sorry") return "!";
  return "✕";
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
let frameLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let linkLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let nodeLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;

let frameRects: d3.Selection<SVGRectElement, SimNode, SVGGElement, unknown> | null = null;
type Link = { source: SimNode; target: SimNode; relation: string | null; bend: number | null };

let linkPaths: d3.Selection<SVGPathElement, Link, SVGGElement, unknown> | null = null;
let bendLayer: d3.Selection<SVGGElement, unknown, null, undefined> | null = null;
let groupNodes: d3.Selection<SVGGElement, SimNode, SVGGElement, unknown> | null = null;
let pillNodes: d3.Selection<SVGGElement, SimNode, SVGGElement, unknown> | null = null;

let zoom: d3.ZoomBehavior<SVGSVGElement, unknown> | null = null;
let observer: ResizeObserver | null = null;
let fitted = false;

let currentNodes: SimNode[] = [];

/** Node under the pointer, with the position of the little info card. */
const card = ref<{
  id: string;
  key: string;
  label: string;
  kind: string;
  status: string | null;
  x: number;
  y: number;
} | null>(null);
let hoverTimer: number | null = null;
const HOVER_DELAY = 180;

/** Qualified name of a node, taken from its key. */
function qualifiedName(kind: string, key: string): string {
  if (kind === "namespace") {
    return key.split(":").slice(2).join(":");
  }
  const head = key.split("@")[0];
  const colon = head.indexOf(":");
  return colon >= 0 ? head.slice(colon + 1) : head;
}

const CONTEXT_KINDS = new Set(["file", "folder", "lib", "astgroup"]);

/** Full name shown at the top of the card. */
const cardTitle = computed(() => {
  const node = card.value;
  if (!node) return "";
  if (node.kind === "lib") return node.label;
  if (CONTEXT_KINDS.has(node.kind)) return node.key.split(":").slice(1).join(":");
  return qualifiedName(node.kind, node.key);
});

/** Namespace: everything before the node's own name. */
const cardNamespace = computed(() => {
  const node = card.value;
  if (!node) return "";
  if (node.kind === "lib") return "library";
  if (CONTEXT_KINDS.has(node.kind)) return "—";
  const qualified = qualifiedName(node.kind, node.key);
  const dot = qualified.lastIndexOf(".");
  return dot > 0 ? qualified.slice(0, dot) : "root";
});

const cardFile = computed(() => {
  const node = card.value;
  if (!node) return "";
  if (node.kind === "folder") return node.key.slice("dir:".length);
  if (node.kind === "astgroup") return node.key.slice("astgrp:".length);
  if (node.kind === "file") return node.key.slice("file:".length);
  const parts = node.key.split("@");
  return parts.length > 1 ? parts[1] : "";
});

const cardComments = computed(() => {
  const node = card.value;
  const info = props.hoverInfo;
  if (!node || !info || info.id !== node.id) return "";
  return (info.comments ?? "").trim();
});

const cardStatus = computed(() => {
  const status = card.value?.status;
  if (status === "proven") return "proven";
  if (status === "sorry") return "sorry";
  if (status === "error") return "error";
  return "—";
});

function showCard(event: PointerEvent, node: SimNode) {
  // Nothing pops up while something is being moved.
  if (pressed || dragNodes.length > 0) return;
  if (hoverTimer !== null) window.clearTimeout(hoverTimer);
  if (card.value && card.value.id !== node.id) card.value = null;
  const host = hostEl();
  hoverTimer = window.setTimeout(() => {
    hoverTimer = null;
    if (!host) return;
    const bounds = host.getBoundingClientRect();
    card.value = {
      id: node.id,
      key: node.key,
      label: node.label,
      kind: node.kind,
      status: node.status,
      x: Math.min(Math.max(8, event.clientX - bounds.left + 14), bounds.width - 210),
      y: Math.min(Math.max(8, event.clientY - bounds.top + 16), bounds.height - 96),
    };
    emit("hover", node.id);
  }, HOVER_DELAY);
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

function hostEl(): HTMLDivElement | null {
  return host.value;
}
let childrenOf = new Map<string, SimNode[]>();
const boxes = new Map<string, Box>();

function truncate(text: string, limit: number): string {
  return text.length <= limit ? text : text.slice(0, limit - 1) + "…";
}

function guard(action: () => void) {
  try {
    action();
    graphError.value = null;
  } catch (error) {
    graphError.value = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  }
}

// ------------------------------------------------------------------ lifecycle

function mount() {
  const el = host.value;
  if (!el) return;

  svg = d3
    .select(el)
    .append("svg")
    .attr("class", "graph__svg")
    .attr("width", el.clientWidth)
    .attr("height", el.clientHeight);

  root = svg.append("g").attr("class", "graph__root");
  frameLayer = root.append("g").attr("class", "graph__frames");
  linkLayer = root.append("g").attr("class", "graph__links");
  nodeLayer = root.append("g").attr("class", "graph__nodes");
  bendLayer = root.append("g").attr("class", "graph__bends");

  zoom = d3
    .zoom<SVGSVGElement, unknown>()
    .scaleExtent([0.2, 2.4])
    // Wheel zooms; panning is ctrl/cmd + drag so plain dragging can select.
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
  svg.on("pointermove", onHostPointerMove);
  svg.on("pointerleave", () => {
    if (!bendHover.value) return;
    bendHover.value = null;
    renderBend();
  });

  observer = new ResizeObserver(() => {
    if (!el || !svg) return;
    svg.attr("width", el.clientWidth).attr("height", el.clientHeight);
  });
  observer.observe(el);
}

// --------------------------------------------------------------- arrangement

// ---------------------------------------------------------------- rendering

function refreshBoxes() {
  boxes.clear();
  for (const [id, box] of measureBoxes(currentNodes, childrenOf, {
    groupKinds: kinds.value,
    padding: GROUP_PADDING,
  })) {
    boxes.set(id, box);
  }
}

/** Geometry of one link: its two bends and the handle between them. */
interface LinkShape {
  points: [number, number][];
  /** Midpoint of the middle segment, where the drag handle sits. */
  handle: [number, number];
  /** `x` when the route runs sideways, `y` when it runs up and down. */
  axis: "x" | "y";
}

function linkShape(source: SimNode, target: SimNode, bend: number | null): LinkShape {
  const halfSource = pillHalfWidth(source) / 2;
  const halfTarget = pillHalfWidth(target) / 2;
  const sx = source.x;
  const sy = source.y;
  const tx = target.x;
  const ty = target.y;
  const dx = tx - sx;
  const dy = ty - sy;

  if (Math.abs(dx) >= Math.abs(dy)) {
    const startX = sx + (dx >= 0 ? halfSource : -halfSource);
    const endX = tx + (dx >= 0 ? -halfTarget : halfTarget);
    const midX = bend ?? (startX + endX) / 2;
    return {
      points: [
        [startX, sy],
        [midX, sy],
        [midX, ty],
        [endX, ty],
      ],
      handle: [midX, (sy + ty) / 2],
      axis: "x",
    };
  }

  const startY = sy + (dy >= 0 ? 11 : -11);
  const endY = ty + (dy >= 0 ? -11 : 11);
  const midY = bend ?? (startY + endY) / 2;
  return {
    points: [
      [sx, startY],
      [sx, midY],
      [tx, midY],
      [tx, endY],
    ],
    handle: [(sx + tx) / 2, midY],
    axis: "y",
  };
}

// --------------------------------------------------------------- link bends

/** Pointer position in graph units. */
function pointerGraph(event: PointerEvent): [number, number] {
  const element = svg?.node();
  const bounds = host.value?.getBoundingClientRect();
  if (!element || !bounds) return [0, 0];
  const transform = d3.zoomTransform(element);
  return [
    (event.clientX - bounds.left - transform.x) / transform.k,
    (event.clientY - bounds.top - transform.y) / transform.k,
  ];
}

/** Distance from a point to an axis-aligned segment. */
function segmentDistance(px: number, py: number, a: [number, number], b: [number, number]): number {
  const dx = Math.abs(b[0] - a[0]);
  const dy = Math.abs(b[1] - a[1]);
  const minX = Math.min(a[0], b[0]);
  const maxX = Math.max(a[0], b[0]);
  const minY = Math.min(a[1], b[1]);
  const maxY = Math.max(a[1], b[1]);
  const across = dy > dx ? Math.abs(px - a[0]) : Math.abs(py - a[1]);
  const loose = dy > dx ? py < minY - 0.5 || py > maxY + 0.5 : px < minX - 0.5 || px > maxX + 0.5;
  return loose ? Number.POSITIVE_INFINITY : across;
}

/** Draws the little handle on the link whose middle is under the pointer. */
function renderBend() {
  if (!bendLayer) return;
  const items = bendHover.value ? [bendHover.value] : [];
  bendLayer
    .selectAll<SVGCircleElement, { key: string; x: number; y: number }>("circle.graph__bend")
    .data(items, (item) => item.key)
    .join("circle")
    .attr("class", "graph__bend")
    .attr("r", 4.5)
    .attr("cx", (item) => item.x)
    .attr("cy", (item) => item.y)
    .on("pointerdown", (event: PointerEvent, item) => startBendDrag(event, item));
}

function onHostPointerMove(event: PointerEvent) {
  if (bendDrag || pressed || dragNodes.length > 0) return;
  const element = svg?.node();
  if (!element) return;
  const [px, py] = pointerGraph(event);
  const reach = 7 / (d3.zoomTransform(element).k || 1);
  let best: { key: string; x: number; y: number; distance: number } | null = null;
  for (const [key, shape] of shapes) {
    const distance = segmentDistance(px, py, shape.points[1], shape.points[2]);
    if (distance <= reach && (!best || distance < best.distance)) {
      best = { key, x: shape.handle[0], y: shape.handle[1], distance };
    }
  }
  const next = best ? { key: best.key, x: best.x, y: best.y } : null;
  const same = next?.key === bendHover.value?.key && next?.x === bendHover.value?.x;
  bendHover.value = next;
  if (!same) renderBend();
}

function reshapeOne(key: string) {
  linkPaths
    ?.filter((link) => linkKey(link) === key)
    .attr("d", (link) => {
      const bend = bends.value[key] ?? link.bend ?? null;
      const shape = linkShape(link.source, link.target, bend);
      shapes.set(key, shape);
      return rounded(shape.points, 7);
    });
}

function startBendDrag(event: PointerEvent, item: { key: string; x: number; y: number }) {
  const shape = shapes.get(item.key);
  if (!shape) return;
  event.preventDefault();
  event.stopPropagation();
  const key = item.key;
  const axis = shape.axis;
  bendDrag = { key, axis };

  const move = (moveEvent: PointerEvent) => {
    const [px, py] = pointerGraph(moveEvent);
    bends.value[key] = axis === "x" ? px : py;
    reshapeOne(key);
    const next = shapes.get(key);
    if (!next) return;
    bendHover.value = { key, x: next.handle[0], y: next.handle[1] };
    renderBend();
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    bendDrag = null;
    emit("bend", key, bends.value[key] ?? null);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

/** Polyline with rounded corners. */
function rounded(points: number[][], radius: number): string {
  if (points.length < 2) return "";
  let path = `M${points[0][0]},${points[0][1]}`;
  for (let index = 1; index < points.length - 1; index += 1) {
    const [px, py] = points[index - 1];
    const [cx, cy] = points[index];
    const [nx, ny] = points[index + 1];
    const inLength = Math.hypot(cx - px, cy - py);
    const outLength = Math.hypot(nx - cx, ny - cy);
    const inRadius = Math.min(radius, inLength / 2);
    const outRadius = Math.min(radius, outLength / 2);
    const ax = cx + ((px - cx) / (inLength || 1)) * inRadius;
    const ay = cy + ((py - cy) / (inLength || 1)) * inRadius;
    const bx = cx + ((nx - cx) / (outLength || 1)) * outRadius;
    const by = cy + ((ny - cy) / (outLength || 1)) * outRadius;
    path += ` L${ax},${ay} Q${cx},${cy} ${bx},${by}`;
  }
  const last = points[points.length - 1];
  path += ` L${last[0]},${last[1]}`;
  return path;
}

function render() {
  const el = host.value;
  if (!el || !nodeLayer || !linkLayer || !frameLayer) return;
  if (props.nodes.length === 0) {
    for (const layer of [frameLayer, linkLayer, nodeLayer]) {
      layer.selectAll("*").remove();
    }
    return;
  }

  currentNodes = props.nodes.map((node) => ({ ...node, x: node.x, y: node.y }));
  if (card.value && !currentNodes.some((node) => node.id === card.value?.id)) {
    card.value = null;
  }
  childrenOf = indexChildren(currentNodes);
  refreshBoxes();

  const byId = new Map(currentNodes.map((node) => [node.id, node]));
  const links = props.links
    .map((link) => ({
      source: byId.get(link.source),
      target: byId.get(link.target),
      relation: link.relation ?? null,
      bend: link.bend ?? null,
    }))
    .filter((link): link is Link =>
      Boolean(link.source && link.target),
    );

  const groups = currentNodes.filter((node) => isGroup(node));
  const leaves = currentNodes.filter((node) => !isGroup(node));

  // --- groups: rectangle + header, no node inside -------------------------
  const groupJoin = nodeLayer
    .selectAll<SVGGElement, SimNode>("g.graph-group")
    .data(groups, (node) => node.id);
  groupJoin.exit().remove();

  const groupEnter = groupJoin.enter().append("g").attr("class", "graph-group");
  const header = groupEnter.append("g").attr("class", "graph-group__head");
  header.append("rect").attr("class", "graph-group__head-bg");
  header.append("text").attr("class", "graph-group__status");
  header.append("text").attr("class", "graph-group__label");
  header.append("text").attr("class", "graph-group__chevron");

  groupNodes = groupEnter.merge(groupJoin);
  groupNodes
    .attr("data-kind", (node) => node.kind)
    .attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null))
    .attr("data-expanded", (node) => (node.expanded ? "true" : null))
    .attr("data-status", (node) => (tinted(node) ? node.status : null));

  groupNodes
    .select<SVGTextElement>(".graph-group__status")
    .attr("text-anchor", "middle")
    .text((node) => (showsStatus(node) ? statusGlyph(node.status as string) : ""));

  groupNodes
    .select<SVGTextElement>(".graph-group__label")
    .text((node) => truncate(node.label, 30));

  groupNodes
    .select<SVGTextElement>(".graph-group__chevron")
    .text((node) => {
      // A file holds one AST box; there is nothing to fold away.
      if (!props.expandable || node.child_count === 0 || node.kind === "file") return "";
      return node.expanded ? "▾" : "▸";
    })
    .style("display", (node) =>
      props.expandable && node.child_count > 0 && node.kind !== "file" ? null : "none",
    )
    .on("click", (event: MouseEvent, node) => {
      event.stopPropagation();
      emit("toggle", node.id);
    });

  // --- pills --------------------------------------------------------------
  const pillJoin = nodeLayer
    .selectAll<SVGGElement, SimNode>("g.graph-pill")
    .data(leaves, (node) => node.id);
  pillJoin.exit().remove();

  const pillEnter = pillJoin.enter().append("g").attr("class", "graph-pill");
  pillEnter.append("rect").attr("class", "graph-pill__box");
  pillEnter.append("text").attr("class", "graph-pill__status");
  pillEnter.append("rect").attr("class", "graph-pill__bar");
  pillEnter.append("text").attr("class", "graph-pill__label");

  const pillWidth = (node: SimNode) => pillHalfWidth(node) * 2;

  pillNodes = pillEnter.merge(pillJoin);
  pillNodes
    .attr("data-kind", (node) => node.kind)
    .attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null))
    .attr("data-status", (node) => (tinted(node) ? node.status : null));

  pillNodes
    .select<SVGRectElement>(".graph-pill__box")
    .attr("x", (node) => -pillWidth(node) / 2)
    .attr("y", -10.5)
    .attr("width", (node) => pillWidth(node))
    .attr("height", 21)
    .attr("rx", 3);

  pillNodes
    .select<SVGTextElement>(".graph-pill__status")
    .attr("x", (node) => -pillWidth(node) / 2 + 9)
    .attr("y", 4.2)
    .attr("text-anchor", "middle")
    .text((node) => (showsStatus(node) ? statusGlyph(node.status as string) : ""));

  pillNodes
    .select<SVGRectElement>(".graph-pill__bar")
    .attr("x", (node) => -pillWidth(node) / 2 + (showsStatus(node) ? 15 : 0))
    .attr("y", -10.5)
    .attr("width", 2.5)
    .attr("height", 21);

  pillNodes
    .select<SVGTextElement>(".graph-pill__label")
    .attr("x", (node) => -pillWidth(node) / 2 + (showsStatus(node) ? 22 : 9))
    .attr("y", 3.8)
    .text((node) => truncate(node.label, 27));

  // --- links --------------------------------------------------------------
  linkPaths = linkLayer
    .selectAll<SVGPathElement, Link>("path.graph-link")
    .data(links)
    .join("path")
    .attr("class", "graph-link")
    .attr("data-relation", (link) => link.relation);

  markLinks();

  // --- frames -------------------------------------------------------------
  const framed = groups.filter((node) => (childrenOf.get(node.id)?.length ?? 0) > 0);
  frameRects = frameLayer
    .selectAll<SVGRectElement, SimNode>("rect.graph-frame")
    .data(framed, (node) => node.id)
    .join("rect")
    .attr("class", "graph-frame")
    .attr("data-kind", (node) => node.kind)
    .attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null))
    .attr("rx", 3);

  // --- interaction --------------------------------------------------------
  groupNodes
    .on("pointerenter", (event: PointerEvent, node) => showCard(event, node))
    .on("pointerleave", () => hideCard())
    .on("pointerdown", (event: PointerEvent, node) => {
      event.stopPropagation();
      press(event, node);
    })
    .on("pointerup", (event: PointerEvent, node) => {
      event.stopPropagation();
      release(event, node);
    });

  groupNodes.select<SVGGElement>(".graph-group__head").call(
    d3
      .drag<SVGGElement, SimNode>()
      .container(() => nodeLayer?.node() as SVGGElement)
      .clickDistance(3)
      .subject((_event, node) => ({ x: node.x, y: node.y }))
      .on("start", (event, node) => {
        event.sourceEvent?.stopPropagation();
        startDrag(node, true, event.x, event.y);
      })
      .on("drag", (event) => dragTo(event.x, event.y))
      .on("end", () => endDrag()),
  );

  pillNodes
    .on("pointerenter", (event: PointerEvent, node) => showCard(event, node))
    .on("pointerleave", () => hideCard())
    .on("pointerdown", (event: PointerEvent, node) => {
      event.stopPropagation();
      press(event, node);
    })
    .on("pointerup", (event: PointerEvent, node) => {
      event.stopPropagation();
      release(event, node);
    })
    .call(
      d3
        .drag<SVGGElement, SimNode>()
        .container(() => nodeLayer?.node() as SVGGElement)
        .clickDistance(3)
        .subject((_event, node) => ({ x: node.x, y: node.y }))
        .on("start", (event, node) => {
          startDrag(node, false, event.x, event.y);
        })
        .on("drag", (event) => dragTo(event.x, event.y))
        .on("end", () => endDrag()),
    );

  draw();

  if (!fitted) {
    fitted = true;
    window.setTimeout(() => {
      if (props.camera) {
        applyCamera(props.camera);
        return;
      }
      ensureVisible();
      window.setTimeout(publishCamera, 620);
    }, 120);
  }
}

/** Repaints transforms, frames, headers and links from the current positions. */
function draw() {
  refreshBoxes();

  groupNodes?.attr("transform", (node) => `translate(${node.x},${node.y})`);
  pillNodes?.attr("transform", (node) => `translate(${node.x},${node.y})`);

  groupNodes?.each(function (node) {
    const box = boxes.get(node.id) ?? ownBox(node, kinds.value);
    const width = Math.max(box.w, 60);
    const left = box.cx - width / 2 - node.x;
    const top = box.cy - box.h / 2 - node.y;
    const group = d3.select(this);
    group
      .select(".graph-group__head-bg")
      .attr("x", left)
      .attr("y", top)
      .attr("width", width)
      .attr("height", HEADER_H);
    group
      .select(".graph-group__status")
      .attr("x", left + 9)
      .attr("y", top + 13);
    group
      .select(".graph-group__label")
      .attr("x", left + (showsStatus(node) ? 18 : 6))
      .attr("y", top + 13);
    group
      .select(".graph-group__chevron")
      .attr("x", left + width - 10)
      .attr("y", top + 13);
  });

  frameRects?.each(function (node) {
    const box = boxes.get(node.id) ?? ownBox(node, kinds.value);
    // The box includes the title strip; the frame starts under it.
    d3.select(this)
      .attr("x", box.cx - box.w / 2)
      .attr("y", box.cy - box.h / 2 + HEADER_H)
      .attr("width", box.w)
      .attr("height", Math.max(box.h - HEADER_H, 0));
  });

  shapePaths();
}

/** Link key: stable across renders, so bends survive a reload. */
function linkKey(link: Link): string {
  return `${link.source.id}->${link.target.id}`;
}

/**
 * Recomputes only the links that actually moved. Routing a link is cheap, but
 * on a big board most of them sit still while a single node is dragged.
 */
function shapePaths() {
  if (!linkPaths) return;
  linkPaths.attr("d", (link) => {
    const key = linkKey(link);
    const bend = bends.value[key] ?? link.bend ?? null;
    const shape = linkShape(link.source, link.target, bend);
    shapes.set(key, shape);
    const signature = `${shape.points.flat().join(",")}`;
    const cached = pathCache.get(key);
    if (cached?.signature === signature) return cached.d;
    const d = rounded(shape.points, 7);
    pathCache.set(key, { signature, d });
    return d;
  });
}

// ------------------------------------------------------------------- dragging

/** Local bend overrides (link key -> position of its middle segment). */
const bends = ref<Record<string, number>>({});
const pathCache = new Map<string, { signature: string; d: string }>();
const shapes = new Map<string, LinkShape>();
/** Link whose middle segment is under the pointer. */
const bendHover = ref<{ key: string; x: number; y: number } | null>(null);
let bendDrag: { key: string; axis: "x" | "y" } | null = null;

let dragNodes: { node: SimNode; x: number; y: number }[] = [];
let beforeDrag = new Map<string, readonly [number, number]>();
let dragStart = { x: 0, y: 0 };

function startDrag(node: SimNode, subtree: boolean, pointerX: number, pointerY: number) {
  dragStart = { x: pointerX, y: pointerY };
  // Regions slide their neighbours while a node moves, so the whole board is
  // compared against this snapshot when the drag ends.
  beforeDrag = new Map(currentNodes.map((entry) => [entry.id, [entry.x, entry.y] as const]));
  const targets = subtree ? [node, ...descendants(node)] : [node];
  dragNodes = targets.map((target) => ({ node: target, x: target.x, y: target.y }));
  nodeLayer?.selectAll("g").raise();
}

function descendants(node: SimNode): SimNode[] {
  const out: SimNode[] = [];
  const stack = [...(childrenOf.get(node.id) ?? [])];
  while (stack.length > 0) {
    const current = stack.pop() as SimNode;
    out.push(current);
    stack.push(...(childrenOf.get(current.id) ?? []));
  }
  return out;
}

function dragTo(pointerX: number, pointerY: number) {
  const dx = pointerX - dragStart.x;
  const dy = pointerY - dragStart.y;
  for (const entry of dragNodes) {
    entry.node.x = entry.x + dx;
    entry.node.y = entry.y + dy;
  }
  draw();
}

/**
 * Ends a move. Everything that shifted is written down — a wider region pushes
 * its neighbours, so the whole board is compared against the drag snapshot.
 */
function endDrag() {
  const moved: FixedNode[] = [];
  for (const node of currentNodes) {
    const before = beforeDrag.get(node.id);
    if (!before) {
      moved.push({ id: node.key, x: node.x, y: node.y });
      continue;
    }
    if (before[0] !== node.x || before[1] !== node.y) {
      moved.push({ id: node.key, x: node.x, y: node.y });
    }
  }
  beforeDrag = new Map();
  dragNodes = [];
  if (moved.length > 0) emit("move", moved);
}

/**
 * Selection does not rely on the browser's `click`: a press and a release on the
 * same node, without moving, is a click, and two of them in quick succession is
 * a double click. That works even when a drag handler is sitting on the node.
 */
let pressed: { id: string; x: number; y: number; moved: boolean } | null = null;
let lastPick = { id: "", time: 0 };

function press(event: PointerEvent, node: SimNode) {
  pressed = { id: node.id, x: event.clientX, y: event.clientY, moved: false };
  const track = (move: PointerEvent) => {
    if (!pressed) return;
    if (Math.hypot(move.clientX - pressed.x, move.clientY - pressed.y) > 4) pressed.moved = true;
  };
  window.addEventListener("pointermove", track);
  window.addEventListener(
    "pointerup",
    () => window.removeEventListener("pointermove", track),
    { once: true },
  );
}

function release(event: PointerEvent, node: SimNode) {
  const state = pressed;
  pressed = null;
  if (!state || state.id !== node.id || state.moved) return;
  const additive = event.ctrlKey || event.metaKey;
  const now = Date.now();
  // A double-click opens, modifier or not: the page reads the modifier from the
  // event to decide between another editor and the pane the user last touched.
  if (lastPick.id === node.id && now - lastPick.time < 380) {
    lastPick = { id: "", time: 0 };
    emit("open", node.id, event);
    return;
  }
  lastPick = { id: node.id, time: now };
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

  bandRect = root
    .append("rect")
    .attr("class", "graph__rubber")
    .attr("x", origin.x)
    .attr("y", origin.y)
    .attr("width", 0)
    .attr("height", 0);

  const move = (moveEvent: PointerEvent) => {
    const point = pointerInGraph(moveEvent);
    if (!bandRect || !point) return;
    bandRect
      .attr("x", Math.min(origin.x, point.x))
      .attr("y", Math.min(origin.y, point.y))
      .attr("width", Math.abs(point.x - origin.x))
      .attr("height", Math.abs(point.y - origin.y));
  };

  const up = (upEvent: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    const point = pointerInGraph(upEvent) ?? origin;
    bandRect?.remove();
    bandRect = null;
    const left = Math.min(origin.x, point.x);
    const right = Math.max(origin.x, point.x);
    const top = Math.min(origin.y, point.y);
    const bottom = Math.max(origin.y, point.y);
    if (right - left < 4 && bottom - top < 4) {
      emit("select", []);
      return;
    }
    emit(
      "select",
      currentNodes
        .filter((node) => node.x >= left && node.x <= right && node.y >= top && node.y <= bottom)
        .map((node) => node.id),
    );
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  event.preventDefault();
}

// ------------------------------------------------------------------ viewport

function contentBounds() {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const box of boxes.values()) {
    minX = Math.min(minX, box.cx - box.w / 2);
    maxX = Math.max(maxX, box.cx + box.w / 2);
    minY = Math.min(minY, box.cy - box.h / 2 - HEADER_H);
    maxY = Math.max(maxY, box.cy + box.h / 2);
  }
  if (minX === Infinity) return null;
  return { minX, minY, maxX, maxY };
}

let bandRect: d3.Selection<SVGRectElement, unknown, null, undefined> | null = null;

/** Camera changes are published at most a few times a second. */
let cameraTimer: number | null = null;

/**
 * The view transform, applied as a CSS transform once per animation frame.
 *
 * A wheel tick arrives as several zoom events, and writing an SVG `transform`
 * attribute for each one makes the browser lay the whole graph out again — every
 * node, every link, every label — which is what made zooming stutter and spike the
 * CPU on a large board. A CSS transform is composited instead: the browser moves the
 * picture it already has. Coalescing to one frame keeps a fast wheel from queueing
 * dozens of them.
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
    publishCamera();
  }, 260);
}

function publishCamera() {
  const element = svg?.node();
  if (!element) return;
  const transform = d3.zoomTransform(element);
  emit("camera", { x: transform.x, y: transform.y, k: transform.k });
}

function applyCamera(camera: Camera) {
  if (!svg || !zoom) return;
  svg.call(
    zoom.transform,
    d3.zoomIdentity.translate(camera.x, camera.y).scale(camera.k),
  );
}

function ensureVisible() {
  const bounds = contentBounds();
  if (!svg || !zoom || !host.value || !bounds) return;
  const padding = 44;
  const minX = bounds.minX - padding;
  const maxX = bounds.maxX + padding;
  const minY = bounds.minY - padding;
  const maxY = bounds.maxY + padding;
  const width = host.value.clientWidth;
  const height = host.value.clientHeight;
  const raw = Math.min(width / (maxX - minX), height / (maxY - minY));
  const scale = Math.max(0.25, Math.min(1, raw));
  // Always centre: the stored coordinates start near the origin.
  const transform = d3.zoomIdentity
    .translate(width / 2, height / 2)
    .scale(scale)
    .translate(-(minX + maxX) / 2, -(minY + maxY) / 2);
  svg.transition().duration(320).call(zoom.transform, transform);
}

onMounted(() => {
  guard(() => {
    mount();
    render();
  });
});

onBeforeUnmount(() => {
  observer?.disconnect();
  if (zoomFrame !== 0) window.cancelAnimationFrame(zoomFrame);
  zoomFrame = 0;
  svg?.remove();
  svg = null;
  root = null;
  frameLayer = null;
  linkLayer = null;
  nodeLayer = null;
});

watch(
  () => [props.nodes, props.links],
  () => guard(render),
);

function markLinks() {
  linkPaths?.attr("data-active", (link) =>
    selected.value.has(link.source.id) || selected.value.has(link.target.id) ? "true" : null,
  );
}

watch(
  () => props.selectedIds,
  () => {
    groupNodes?.attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null));
    pillNodes?.attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null));
    frameRects?.attr("data-selected", (node) => (selected.value.has(node.id) ? "true" : null));
    markLinks();
  },
);
</script>

<template>
  <div class="graph">
    <div ref="host" class="graph__host" />

    <div class="graph__hud graph__hud--top">
      <span class="k-label">graph</span>
      <span class="graph__count k-mono">{{ nodes.length }} nodes</span>
      <span class="graph__legend">
        <span class="graph__legend-square" /> group
        <span class="graph__legend-pill" /> declaration
      </span>
    </div>

    <div v-if="card" class="graph__card" :style="{ left: `${card.x}px`, top: `${card.y}px` }">
      <div class="graph__card-head">
        <span class="graph__card-title k-mono">{{ cardTitle }}</span>
        <span class="graph__card-kind">{{ card.kind }}</span>
      </div>
      <p v-if="cardComments" class="graph__card-comments">{{ cardComments }}</p>
      <dl class="graph__card-rows">
        <dt>state</dt>
        <dd :data-status="card.status ?? undefined">{{ cardStatus }}</dd>
        <dt>ns</dt>
        <dd :title="cardNamespace">{{ cardNamespace }}</dd>
        <dt>type</dt>
        <dd>{{ card.kind }}</dd>
        <dt>name</dt>
        <dd :title="card.label">{{ card.label }}</dd>
        <dt v-if="cardFile">file</dt>
        <dd v-if="cardFile" :title="cardFile">{{ cardFile }}</dd>
      </dl>
    </div>

    <div class="graph__tools">
      <button class="graph__tool" type="button" title="settings" @click="emit('settings')">
        <UiIcon name="settings" :size="12" />
      </button>
    </div>

    <div class="graph__hud graph__hud--bottom">
      <span><UiIcon name="drag" :size="10" /> drag to move</span>
      <span><UiIcon name="plus" :size="10" /> drag to select</span>
      <span><UiIcon name="search" :size="10" /> ctrl+drag pan · wheel zoom</span>
    </div>

    <div v-if="graphError" class="graph__error">
      <span class="k-label">graph error</span>
      <span class="graph__error-text">{{ graphError }}</span>
    </div>

    <div v-else-if="nodes.length === 0" class="graph__empty">
      <span class="k-label">no graph yet</span>
    </div>
  </div>
</template>

<style scoped>
/* The zoom layer keeps its own compositing layer: moving it is then a transform of
   a picture the browser already has, not a re-layout of every node and link. */
.graph :deep(.graph__root) {
  will-change: transform;
}

.graph {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background:
    radial-gradient(1200px 500px at 50% -10%, rgba(106, 169, 196, 0.045), transparent 70%),
    var(--bg-1);
}

.graph__host {
  position: absolute;
  inset: 0;
}

.graph__host :deep(svg) {
  display: block;
}

.graph__hud {
  position: absolute;
  display: flex;
  align-items: center;
  gap: 8px;
  pointer-events: none;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  letter-spacing: 0.04em;
}

.graph__hud--top {
  top: 8px;
  left: 10px;
}

.graph__card {
  position: absolute;
  z-index: 3;
  width: 196px;
  padding: 7px 9px 8px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  background: linear-gradient(180deg, rgba(24, 28, 31, 0.98), rgba(15, 18, 20, 0.98));
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.5);
  pointer-events: none;
  animation: card-fade 130ms var(--ease) both;
}

@keyframes card-fade {
  from {
    opacity: 0;
    transform: translateY(3px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.graph__card-head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding-bottom: 5px;
  border-bottom: 1px solid var(--line-soft);
}

.graph__card-title {
  flex: 1;
  overflow: hidden;
  color: var(--text);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.graph__card-kind {
  flex: none;
  color: var(--accent);
  font-family: var(--mono);
  font-size: var(--fs-micro);
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.graph__card-rows {
  display: grid;
  grid-template-columns: 58px 1fr;
  gap: 2px 6px;
  margin: 6px 0 0;
  font-size: var(--fs-micro);
}

.graph__card-rows dt {
  color: var(--text-ghost);
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.graph__card-rows dd {
  margin: 0;
  overflow: hidden;
  color: var(--text-dim);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.graph__card-rows dd[data-status="proven"] {
  color: var(--ok);
}

.graph__card-rows dd[data-status="sorry"] {
  color: var(--warning);
}

.graph__card-rows dd[data-status="error"] {
  color: var(--danger);
}

.graph__card-comments {
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
  -webkit-line-clamp: 5;
  white-space: pre-wrap;
}

.graph__tools {
  position: absolute;
  top: 6px;
  right: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.graph__tools-wrap {
  position: relative;
}

.graph__menu {
  position: absolute;
  top: 0;
  right: 24px;
  width: 208px;
  padding: 4px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  background: linear-gradient(180deg, rgba(24, 28, 31, 0.98), rgba(16, 19, 21, 0.98));
  box-shadow: 0 10px 24px rgba(0, 0, 0, 0.5);
}

.graph__menu-item {
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

.graph__menu-item:hover {
  background: rgba(255, 255, 255, 0.05);
  color: var(--text);
}

.graph__menu-note {
  margin: 4px 7px 5px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  line-height: 1.45;
}

.graph__host :deep(.graph__bend) {
  fill: #10161a;
  stroke: var(--accent-bright);
  stroke-width: 1.2;
  cursor: grab;
  opacity: 0.85;
}

.graph__host :deep(.graph__bend:hover) {
  opacity: 1;
}

.graph__tool {
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

.graph__tool:hover {
  color: var(--text);
  border-color: var(--line-strong);
  background: var(--bg-2);
}

.graph__hud--bottom {
  bottom: 8px;
  left: 10px;
  gap: 12px;
}

.graph__hud--bottom span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.graph__count {
  color: var(--text-faint);
  font-size: var(--fs-micro);
}

.graph__legend {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-left: 6px;
  padding-left: 10px;
  border-left: 1px solid var(--line);
  color: var(--text-ghost);
}

.graph__legend-square {
  width: 7px;
  height: 7px;
  border: 1px solid var(--accent-line);
  border-radius: 1px;
  background: var(--accent-soft);
}

.graph__legend-pill {
  width: 11px;
  height: 5px;
  margin-left: 4px;
  border: 1px solid #2f363b;
  border-radius: 1px;
  background: #161a1d;
}

.graph__empty,
.graph__error {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.graph__error {
  flex-direction: column;
  gap: 3px;
  align-items: flex-start;
  justify-content: flex-end;
  padding: 10px;
  background: linear-gradient(0deg, rgba(201, 115, 107, 0.12), transparent 40%);
}

.graph__error-text {
  color: var(--danger);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

/* D3-created elements need :deep selectors. */
.graph__host :deep(.graph-frame) {
  fill: rgba(255, 255, 255, 0.01);
  stroke: #262d32;
  stroke-width: 1;
  pointer-events: none;
  transition: stroke var(--t) var(--ease);
}

.graph__host :deep(.graph-frame[data-kind="lib"]) {
  fill: rgba(106, 169, 196, 0.04);
  stroke: rgba(106, 169, 196, 0.22);
}

.graph__host :deep(.graph-frame[data-kind="folder"]),
.graph__host :deep(.graph-frame[data-kind="namespace"]) {
  fill: rgba(255, 255, 255, 0.014);
  stroke: #2d343a;
}

.graph__host :deep(.graph-frame[data-kind="file"]) {
  fill: rgba(255, 255, 255, 0.02);
  stroke: #333b41;
}

.graph__host :deep(.graph-frame[data-kind="astgroup"]) {
  fill: none;
  stroke: #272e33;
  stroke-dasharray: 3 3;
}

.graph__host :deep(.graph-frame[data-selected="true"]) {
  stroke: var(--accent-line);
}

.graph__host :deep(.graph__rubber) {
  fill: rgba(106, 169, 196, 0.08);
  stroke: var(--accent-line);
  stroke-width: 1;
  stroke-dasharray: 3 3;
  pointer-events: none;
}

.graph__host :deep(.graph-group) {
  cursor: pointer;
}

/* Namespaces on the logic board are context, not content: their *background*
   fades, the label stays fully readable. */
.graph__host :deep(.graph-frame[data-kind="namespace"]) {
  fill: rgba(255, 255, 255, 0.005);
  stroke: rgba(255, 255, 255, 0.045);
}

.graph__host :deep(.graph-group[data-kind="namespace"] .graph-group__head-bg) {
  fill: rgba(28, 34, 39, 0.42);
  stroke: rgba(58, 67, 74, 0.45);
}

.graph__host :deep(.graph-group[data-kind="namespace"]:hover .graph-group__head-bg),
.graph__host :deep(.graph-group[data-kind="namespace"][data-selected="true"] .graph-group__head-bg) {
  fill: #1c2227;
  stroke: #3a434a;
}

.graph__host :deep(.graph-group__box) {
  fill: #14181b;
  stroke: #3b464d;
  stroke-width: 1;
  transition: fill var(--t) var(--ease), stroke var(--t) var(--ease);
}

.graph__host :deep(.graph-group:hover .graph-group__box) {
  fill: #1b2126;
  stroke: #4d5962;
}

.graph__host :deep(.graph-group[data-kind="lib"] .graph-group__box) {
  fill: rgba(106, 169, 196, 0.18);
  stroke: var(--accent-line);
}

.graph__host :deep(.graph-group[data-kind="folder"] .graph-group__box),
.graph__host :deep(.graph-group[data-kind="namespace"] .graph-group__box) {
  fill: #1a1e21;
  stroke: #434d54;
}

.graph__host :deep(.graph-group[data-kind="file"] .graph-group__box) {
  fill: #1d2226;
  stroke: #4e5962;
}

.graph__host :deep(.graph-group[data-kind="astgroup"] .graph-group__box) {
  fill: #15191c;
  stroke: #343c42;
  stroke-dasharray: 3 2;
}

.graph__host :deep(.graph-group[data-selected="true"] .graph-group__box) {
  stroke: var(--accent-bright);
  fill: #1a2429;
}

.graph__host :deep(.graph-group__code) {
  fill: #7e888f;
  font-family: var(--mono);
  font-size: 7.5px;
  letter-spacing: 0.04em;
  pointer-events: none;
}

.graph__host :deep(.graph-group[data-kind="lib"] .graph-group__code) {
  fill: var(--accent);
}

.graph__host :deep(.graph-group__head) {
  cursor: grab;
}

.graph__host :deep(.graph-group__head:active) {
  cursor: grabbing;
}

.graph__host :deep(.graph-group__head-bg) {
  fill: #1c2227;
  stroke: #3a434a;
  stroke-width: 1;
  rx: 2;
  transition: fill var(--t) var(--ease), stroke var(--t) var(--ease);
}

.graph__host :deep(.graph-group:hover .graph-group__head-bg) {
  fill: #232a30;
}

.graph__host :deep(.graph-group[data-kind="lib"] .graph-group__head-bg) {
  fill: rgba(106, 169, 196, 0.16);
  stroke: rgba(106, 169, 196, 0.3);
}

.graph__host :deep(.graph-group[data-selected="true"] .graph-group__head-bg) {
  stroke: var(--accent-bright);
}

.graph__host :deep(.graph-group__status),
.graph__host :deep(.graph-pill__status) {
  fill: var(--ok);
  font-family: var(--mono);
  font-size: 11px;
  pointer-events: none;
}

.graph__host :deep([data-status="sorry"] .graph-group__status) {
  fill: var(--warning);
}

.graph__host :deep([data-status="error"] .graph-group__status) {
  fill: var(--danger);
}

/* Logic board: the pill itself carries the proof state, and the glyph takes the
   board's background colour, so it reads as a hole punched in the tint. */
.graph__host :deep(.graph-pill[data-status="proven"] .graph-pill__box) {
  fill: rgba(124, 171, 136, 0.2);
  stroke: rgba(124, 171, 136, 0.5);
}

.graph__host :deep(.graph-pill[data-status="sorry"] .graph-pill__box) {
  fill: rgba(194, 166, 112, 0.2);
  stroke: rgba(194, 166, 112, 0.5);
}

.graph__host :deep(.graph-pill[data-status="error"] .graph-pill__box) {
  fill: rgba(201, 115, 107, 0.22);
  stroke: rgba(201, 115, 107, 0.55);
}

.graph__host :deep(.graph-pill[data-status] .graph-pill__status) {
  fill: #0b0d0f;
  font-size: 12px;
}

.graph__host :deep(.graph-group__label) {
  fill: #aeb7bd;
  font-family: var(--mono);
  font-size: 9.5px;
  pointer-events: none;
}

.graph__host :deep(.graph-group[data-kind="lib"] .graph-group__label) {
  fill: var(--accent-bright);
}

.graph__host :deep(.graph-group[data-selected="true"] .graph-group__label) {
  fill: var(--accent-bright);
}

.graph__host :deep(.graph-group__chevron) {
  fill: #8a949c;
  font-family: var(--mono);
  font-size: 9px;
  text-anchor: middle;
  cursor: pointer;
}

.graph__host :deep(.graph-pill) {
  cursor: pointer;
}

.graph__host :deep(.graph-pill__box) {
  fill: #161a1d;
  stroke: #262c30;
  stroke-width: 1;
  transition: fill var(--t) var(--ease), stroke var(--t) var(--ease);
}

.graph__host :deep(.graph-pill:hover .graph-pill__box) {
  fill: #1b2024;
  stroke: #333a3f;
}

.graph__host :deep(.graph-pill__bar) {
  fill: #6b757c;
  transition: fill var(--t) var(--ease);
}

.graph__host :deep(.graph-pill__label) {
  fill: #b9c2c9;
  font-family: var(--mono);
  font-size: 10px;
  pointer-events: none;
  transition: fill var(--t) var(--ease);
}

.graph__host :deep(.graph-pill[data-kind="namespace"] .graph-pill__bar) {
  fill: #93cbe0;
}

.graph__host :deep(.graph-pill[data-kind="def"] .graph-pill__bar) {
  fill: #8fb8a4;
}

.graph__host :deep(.graph-pill[data-kind="theorem"] .graph-pill__bar),
.graph__host :deep(.graph-pill[data-kind="lemma"] .graph-pill__bar) {
  fill: #a8a0cf;
}

.graph__host :deep(.graph-pill[data-kind="structure"] .graph-pill__bar),
.graph__host :deep(.graph-pill[data-kind="class"] .graph-pill__bar),
.graph__host :deep(.graph-pill[data-kind="inductive"] .graph-pill__bar) {
  fill: #c9a96b;
}

.graph__host :deep(.graph-pill[data-kind="axiom"] .graph-pill__bar) {
  fill: #c9a86b;
}

.graph__host :deep(.graph-pill[data-selected="true"] .graph-pill__box) {
  fill: #1a2328;
  stroke: var(--accent-bright);
}

.graph__host :deep(.graph-pill[data-selected="true"] .graph-pill__label) {
  fill: var(--accent-bright);
}

.graph__host :deep(.graph-link) {
  fill: none;
  stroke: #262e33;
  stroke-width: 1;
  transition: stroke var(--t) var(--ease);
}

.graph__host :deep(.graph-link[data-relation="axiom"]),
.graph__host :deep(.graph-link[data-relation="structure"]) {
  stroke: rgba(201, 168, 107, 0.45);
}

.graph__host :deep(.graph-link[data-relation="def"]) {
  stroke: rgba(143, 184, 164, 0.4);
}

.graph__host :deep(.graph-link[data-relation="theorem"]) {
  stroke: rgba(168, 160, 207, 0.42);
}

/* The selection pulls its links forward, in the same blue as a picked node.
   This rule sits last on purpose: relation colours would otherwise win. */
.graph__host :deep(path.graph-link[data-active="true"]) {
  stroke: var(--accent-bright);
  stroke-width: 1.7;
  opacity: 1;
}
</style>
