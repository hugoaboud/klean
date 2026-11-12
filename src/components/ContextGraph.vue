<script setup lang="ts">
// The AST context of an AI session, drawn.
//
// A small force layout is enough for the few dozen declarations a context holds:
// seeds are marked, and an arrow runs from a declaration to the one it refers to,
// so it is visible at a glance which node pulled which into the prompt.
import * as d3 from "d3";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { ContextGraph } from "../api/ai";

const props = defineProps<{ graph: ContextGraph | null }>();

interface Point extends d3.SimulationNodeDatum {
  id: string;
  label: string;
  kind: string;
  seed: boolean;
}

interface Link extends d3.SimulationLinkDatum<Point> {
  source: string | Point;
  target: string | Point;
}

const host = ref<HTMLElement | null>(null);
const svg = ref<SVGSVGElement | null>(null);
let simulation: d3.Simulation<Point, Link> | null = null;

function shortLabel(label: string): string {
  // `Demo.inner.foo` reads better as `…inner.foo` in a small box.
  return label.length > 22 ? `…${label.slice(-21)}` : label;
}

function draw() {
  if (!svg.value || !host.value) return;
  const width = Math.max(220, host.value.clientWidth);
  const height = 200;
  const graph = props.graph;
  const nodes: Point[] = (graph?.nodes ?? []).map((node) => ({
    id: node.id,
    label: node.label,
    kind: node.kind,
    seed: node.seed,
  }));
  const byId = new Set(nodes.map((node) => node.id));
  const links: Link[] = (graph?.edges ?? [])
    .filter(([from, to]) => byId.has(from) && byId.has(to))
    .map(([from, to]) => ({ source: from, target: to }));

  const root = d3.select(svg.value);
  root.selectAll("*").remove();
  root.attr("viewBox", `0 0 ${width} ${height}`).attr("width", width).attr("height", height);

  const layer = root.append("g");
  // Arrow heads, so the direction of a reference is readable.
  root
    .append("defs")
    .append("marker")
    .attr("id", "klean-context-arrow")
    .attr("viewBox", "0 -4 8 8")
    .attr("refX", 16)
    .attr("refY", 0)
    .attr("markerWidth", 5)
    .attr("markerHeight", 5)
    .attr("orient", "auto")
    .append("path")
    .attr("d", "M0,-4 L8,0 L0,4")
    .attr("fill", "#5d6b74");

  const link = layer
    .append("g")
    .selectAll("line")
    .data(links)
    .join("line")
    .attr("stroke", "#3d4a52")
    .attr("stroke-width", 1)
    .attr("marker-end", "url(#klean-context-arrow)");

  const node = layer
    .append("g")
    .selectAll<SVGGElement, Point>("g")
    .data(nodes)
    .join("g")
    .attr("data-seed", (point) => (point.seed ? "true" : null));

  node.append("circle").attr("r", 4);
  node
    .append("text")
    .attr("x", 7)
    .attr("y", 3.5)
    .text((point) => shortLabel(point.label));

  simulation?.stop();
  if (nodes.length === 0) return;
  simulation = d3
    .forceSimulation<Point>(nodes)
    .force(
      "link",
      d3
        .forceLink<Point, Link>(links)
        .id((point) => point.id)
        .distance(60)
        .strength(0.6),
    )
    .force("charge", d3.forceManyBody<Point>().strength(-120))
    .force("center", d3.forceCenter(width / 2, height / 2))
    .force("collide", d3.forceCollide<Point>(26))
    .on("tick", () => {
      link
        .attr("x1", (entry) => (entry.source as Point).x ?? 0)
        .attr("y1", (entry) => (entry.source as Point).y ?? 0)
        .attr("x2", (entry) => (entry.target as Point).x ?? 0)
        .attr("y2", (entry) => (entry.target as Point).y ?? 0);
      node.attr("transform", (point) => {
        // Keep every label inside the box.
        const x = Math.min(Math.max(12, point.x ?? 0), width - 12);
        const y = Math.min(Math.max(10, point.y ?? 0), height - 10);
        return `translate(${x},${y})`;
      });
    });
}

watch(() => props.graph, draw, { deep: true });

let observer: ResizeObserver | null = null;
onMounted(() => {
  draw();
  if (!host.value) return;
  observer = new ResizeObserver(() => draw());
  observer.observe(host.value);
});

onBeforeUnmount(() => {
  simulation?.stop();
  observer?.disconnect();
});
</script>

<template>
  <div ref="host" class="context-graph">
    <svg ref="svg" class="context-graph__svg" />
    <p v-if="!(graph?.nodes.length ?? 0)" class="context-graph__empty k-mono">
      the context holds no declarations yet
    </p>
  </div>
</template>

<style scoped>
.context-graph {
  display: flex;
  flex-direction: column;
  align-items: center;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: #0b0d0f;
  overflow: hidden;
}

.context-graph__svg {
  display: block;
}

.context-graph__svg :deep(circle) {
  fill: #2a343b;
  stroke: #47555f;
}

.context-graph__svg :deep(g[data-seed] circle) {
  fill: #3a7086;
  stroke: var(--accent-bright);
}

.context-graph__svg :deep(text) {
  fill: var(--text-faint);
  font-family: var(--mono);
  font-size: 9px;
  pointer-events: none;
}

.context-graph__svg :deep(g[data-seed] text) {
  fill: var(--text);
}

.context-graph__empty {
  margin: 0;
  padding: 8px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}
</style>
