import Graph from "graphology";
import { inferSettings } from "graphology-layout-forceatlas2";
import FA2Layout from "graphology-layout-forceatlas2/worker";
import Sigma from "sigma";
import { animateNodes } from "sigma/utils";
import { board, type GraphData } from "../api";
import { enc, go, h, mount } from "../dom";
import { T } from "../text";

const PALETTE = ["#ffffff", "#c8d3e6", "#e6c8c8", "#c8e6d2", "#dacbe6", "#e6dcc0", "#c0e0e2", "#e6c8da", "#d6d6b8", "#b9bec8"];
const OTHER = "#6a6a6f";
const DIM = "#1b1b1d";
const EDGE = "rgba(255,255,255,0.07)";
const EDGE_ON = "rgba(255,255,255,0.45)";
const LABEL = "#f2f2f2";
const STORE = "wisply.graph.v1:";
const ANIMATE_MAX = 6000;
const NUDGE_MS = 900;
const KINDS = ["note", "project", "session"] as const;
const REPORT_TOP = 10;
const NAMED = 24;

type Kind = (typeof KINDS)[number];
type Attrs = { label: string; kind: Kind; group: string; t: number; x: number; y: number; size: number; color: string; deg: number };

function layoutMs(n: number): number {
  return Math.min(8000, 1500 + n / 3);
}

type Spots = Record<string, [number, number]>;

function loadSpots(key: string): Spots {
  try {
    return JSON.parse(localStorage.getItem(STORE + key) ?? "{}") as Spots;
  } catch {
    return {};
  }
}

function saveSpots(key: string, g: Graph<Attrs>): void {
  const out: Spots = {};
  g.forEachNode((n, a) => {
    out[n] = [Math.round(a.x * 10) / 10, Math.round(a.y * 10) / 10];
  });
  try {
    localStorage.setItem(STORE + key, JSON.stringify(out));
  } catch {
    localStorage.removeItem(STORE + key);
  }
}

function place(g: Graph<Attrs>, spots: Spots): number {
  let fresh = 0;
  g.forEachNode((n, a) => {
    const s = spots[n];
    if (s) {
      a.x = s[0];
      a.y = s[1];
      return;
    }
    fresh++;
    const near = g.neighbors(n).map((m) => spots[m]).filter((x): x is [number, number] => !!x);
    if (near.length) {
      a.x = near.reduce((v, s2) => v + s2[0], 0) / near.length + (Math.random() - 0.5) * 4;
      a.y = near.reduce((v, s2) => v + s2[1], 0) / near.length + (Math.random() - 0.5) * 4;
    }
  });
  return fresh;
}

function frame(f: () => void): () => void {
  let queued = false;
  return () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      f();
    });
  };
}

function seed(data: GraphData): { g: Graph<Attrs>; colors: Map<string, string> } {
  const g = new Graph<Attrs>({ type: "undirected", multi: false, allowSelfLoops: false });
  const deg = new Array<number>(data.nodes.length).fill(0);
  for (const [a, b] of data.edges) {
    deg[a]++;
    deg[b]++;
  }
  const sizes = new Map<string, number>();
  for (const n of data.nodes) sizes.set(n[3], (sizes.get(n[3]) ?? 0) + 1);
  const groups = [...sizes.entries()].sort((a, b) => b[1] - a[1]);
  const colors = new Map<string, string>();
  groups.filter(([name]) => name).slice(0, PALETTE.length).forEach(([name], i) => colors.set(name, PALETTE[i]));
  const centers = new Map<string, [number, number]>();
  groups.forEach(([name, count], i) => {
    const r = 40 * Math.sqrt(i) + Math.sqrt(count);
    const a = i * 2.39996;
    centers.set(name, [r * Math.cos(a), r * Math.sin(a)]);
  });
  data.nodes.forEach(([id, label, kind, group, t], i) => {
    const [cx, cy] = centers.get(group) ?? [0, 0];
    const spread = Math.sqrt(sizes.get(group) ?? 1) * 1.5;
    const k = kind as Kind;
    g.addNode(id, {
      label: label || id,
      kind: k,
      group,
      t,
      x: cx + (Math.random() - 0.5) * spread,
      y: cy + (Math.random() - 0.5) * spread,
      size: k === "project" ? 6 + Math.sqrt(deg[i]) * 0.6 : 1.5 + Math.sqrt(deg[i]) * 0.9,
      color: colors.get(group) ?? OTHER,
      deg: deg[i],
    });
  });
  for (const [a, b] of data.edges) {
    const s = data.nodes[a][0];
    const d = data.nodes[b][0];
    if (s !== d && !g.hasEdge(s, d)) g.addEdge(s, d);
  }
  return { g, colors };
}

function report(g: Graph<Attrs>, pick: (node: string) => void): HTMLElement {
  const notes = g.filterNodes((_, a) => a.kind === "note");
  const hubs = [...notes].sort((a, b) => g.getNodeAttribute(b, "deg") - g.getNodeAttribute(a, "deg")).slice(0, REPORT_TOP);
  const reach = (n: string) => new Set(g.mapNeighbors(n, (_, a) => a.group).filter((x) => x && x !== g.getNodeAttribute(n, "group"))).size;
  const bridges = notes
    .map((n) => [n, reach(n)] as const)
    .filter(([, r]) => r > 0)
    .sort((a, b) => b[1] - a[1])
    .slice(0, REPORT_TOP)
    .map(([n]) => n);
  const asks: [string, string][] = [];
  for (const a of hubs) {
    const b = hubs.find((x) => x !== a && g.getNodeAttribute(x, "group") !== g.getNodeAttribute(a, "group") && !g.hasEdge(a, x));
    if (b && asks.length < 3) asks.push([a, b]);
  }
  const link = (n: string) => h("a", { href: "#", onclick: (e: Event) => { e.preventDefault(); pick(n); } }, g.getNodeAttribute(n, "label"));
  return h(
    "div",
    {},
    h("h2", {}, T.graph.hubs),
    hubs.map(link),
    bridges.length ? h("h2", {}, T.graph.bridges) : null,
    bridges.map(link),
    asks.length ? h("h2", {}, T.graph.questions) : null,
    asks.map(([a, b]) => h("div", { class: "muted", style: "padding:3px 0" }, T.graph.ask(g.getNodeAttribute(a, "label"), g.getNodeAttribute(b, "label")))),
  );
}

export async function graph(view: HTMLElement, project?: string): Promise<() => void> {
  mount(view, h("div", { class: "empty", style: "padding:24px" }, T.graph.loading));
  const data = await board<GraphData>("graph", ...(project ? [project] : []));
  const { g, colors } = seed(data);
  const key = project ?? "all";
  const fresh = place(g, loadSpots(key));
  const canvas = h("div", { class: "canvas" });
  const info = h("span", { class: "info" }, T.graph.layout);
  const hidden = new Set<Kind>();
  let orphans = false;
  const named = new Set(g.filterNodes((_, a) => a.kind === "project").sort((a, b) => g.getNodeAttribute(b, "deg") - g.getNodeAttribute(a, "deg")).slice(0, NAMED));
  let focus: string | null = null;
  let near = new Set<string>();
  const times = g.mapNodes((_, a) => a.t).filter((t) => t > 0);
  const [t0, t1] = times.length ? [Math.min(...times), Math.max(...times)] : [0, 0];
  let until = t1;
  const shown = (a: Attrs) => !hidden.has(a.kind) && (orphans || a.deg > 0) && (a.t === 0 || a.t <= until);

  const pick = (n: string | null) => {
    focus = n;
    near = n ? new Set(g.neighbors(n)) : new Set();
    redraw();
    if (n) {
      const d = renderer.getNodeDisplayData(n);
      if (d) void renderer.getCamera().animate({ x: d.x, y: d.y, ratio: 0.25 }, { duration: 400 });
    }
  };
  const done = () => (info.textContent = T.graph.nodes(g.order, g.size));
  const work = g.copy();
  const layout = fresh ? new FA2Layout(work, { settings: { ...inferSettings(work), barnesHutOptimize: work.order > 2000, slowDown: 2 } }) : null;
  const settle = () => {
    layout?.stop();
    const targets: Record<string, { x: number; y: number }> = {};
    work.forEachNode((n, a) => (targets[n] = { x: a.x, y: a.y }));
    if (g.order <= ANIMATE_MAX) animateNodes(g, targets, { duration: 500 }, () => saveSpots(key, g));
    else {
      g.updateEachNodeAttributes((n, a) => ({ ...a, ...targets[n] }), { attributes: ["x", "y"] });
      saveSpots(key, g);
    }
    done();
  };
  layout?.start();
  const stop = layout ? window.setTimeout(settle, fresh === g.order ? layoutMs(g.order) : NUDGE_MS) : 0;
  if (!layout) done();

  let frames = 0;
  let fps = 0;
  let alive = true;
  const fpsBox = h("span", { class: "info" });
  const tick = (t0f: number) => (t: number) => {
    if (!alive) return;
    frames++;
    if (t - t0f >= 1000) {
      fps = Math.round((frames * 1000) / (t - t0f));
      fpsBox.textContent = `${fps} fps`;
      frames = 0;
      requestAnimationFrame(tick(t));
      return;
    }
    requestAnimationFrame(tick(t0f));
  };
  if (location.hash.includes("fps")) requestAnimationFrame(tick(performance.now()));

  const toggles = h(
    "div",
    { class: "legend" },
    KINDS.map((k) =>
      h(
        "label",
        {},
        h("input", {
          type: "checkbox",
          checked: true,
          onchange: (e: Event) => {
            if ((e.target as HTMLInputElement).checked) hidden.delete(k);
            else hidden.add(k);
            redraw();
          },
        }),
        T.graph.kinds[k],
      ),
    ),
    h(
      "label",
      {},
      h("input", {
        type: "checkbox",
        checked: false,
        onchange: (e: Event) => {
          orphans = (e.target as HTMLInputElement).checked;
          redraw();
        },
      }),
      T.graph.orphans,
    ),
    [...colors.entries()].map(([name, c]) => h("label", { onclick: () => pick(`p:${name}`) }, h("span", { class: "swatch", style: `background:${c}` }), name)),
  );
  const find = h("input", {
    type: "search",
    class: "find",
    placeholder: T.graph.find,
    onkeydown: (e: Event) => {
      if ((e as KeyboardEvent).key !== "Enter") return;
      const w = (e.target as HTMLInputElement).value.trim().toLowerCase();
      if (!w) return pick(null);
      const hits = g.filterNodes((_, a) => a.label.toLowerCase().includes(w));
      hits.sort((x, y) => Number(g.getNodeAttribute(y, "kind") === "project") - Number(g.getNodeAttribute(x, "kind") === "project") || g.getNodeAttribute(y, "deg") - g.getNodeAttribute(x, "deg"));
      if (hits.length) pick(hits[0]);
    },
  });
  const label = h("span", {}, T.graph.all);
  const slider = h("input", {
    type: "range",
    min: 0,
    max: 1000,
    value: 1000,
    oninput: (e: Event) => {
      const v = Number((e.target as HTMLInputElement).value) / 1000;
      until = t0 + (t1 - t0) * v;
      label.textContent = v >= 1 ? T.graph.all : `${T.graph.until} ${new Date(until * 1000).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" })}`;
      redraw();
    },
  });
  mount(
    view,
    h(
      "div",
      { class: "graph" },
      h("div", { class: "stage" }, canvas, h("div", { class: "tools" }, find, info, fpsBox), t1 > t0 ? h("div", { class: "time" }, slider, label) : null),
      h("aside", {}, h("div", { class: "muted", style: "font-size:12px;margin-bottom:8px" }, T.graph.focus), toggles, h("h2", {}, T.graph.report), report(g, pick)),
    ),
  );
  const renderer = new Sigma(g, canvas, {
    settings: {
      hideEdgesOnMove: true,
      hideLabelsOnMove: g.order > ANIMATE_MAX,
      labelRenderedSizeThreshold: 10,
      labelDensity: 0.5,
      labelGridCellSize: 120,
      minEdgeThickness: 0.5,
    },
    nodeReducer: (n, d, a) => {
      const out: Partial<typeof d> = { labelColor: LABEL };
      if (!shown(a)) out.visibility = "hidden";
      else if (focus && n !== focus && !near.has(n)) {
        out.color = DIM;
        out.labelVisibility = "hidden";
      } else if (focus) out.labelVisibility = "visible";
      if (!focus && named.has(n)) out.labelVisibility = "visible";
      return out;
    },
    edgeReducer: (e, _d, _a, _s, _gs, gr) => {
      const [s, t] = gr.extremities(e);
      const a = gr.getNodeAttributes(s) as Attrs;
      const b = gr.getNodeAttributes(t) as Attrs;
      if (!shown(a) || !shown(b)) return { visibility: "hidden" };
      if (focus) return s === focus || t === focus ? { color: EDGE_ON, size: 1 } : { visibility: "hidden" };
      return { color: EDGE };
    },
  });
  const redraw = frame(() => renderer.refresh({ skipIndexation: true }));
  renderer.on("clickNode", ({ node }) => pick(node === focus ? null : node));
  renderer.on("clickStage", () => pick(null));
  renderer.on("doubleClickNode", ({ node }) => {
    const a = g.getNodeAttributes(node);
    if (a.kind === "note") go(`#/n/${node.slice(1)}`);
    else if (a.kind === "project") go(`#/p/${enc(a.group)}`);
  });

  return () => {
    alive = false;
    clearTimeout(stop);
    layout?.kill();
    renderer.kill();
  };
}
