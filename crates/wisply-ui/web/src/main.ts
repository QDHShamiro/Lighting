import { board, type Pulse } from "./api";
import { enc, go, h, mount } from "./dom";
import { T } from "./text";
import { graph } from "./views/graph";
import { blind } from "./views/blind";
import { home } from "./views/home";
import { live } from "./views/live";
import { note } from "./views/note";
import { project, type Tab } from "./views/project";
import { search } from "./views/search";
import { learned, settings, tokens } from "./views/side";

const STATUS_MS = 5000;
const bar = document.getElementById("bar") as HTMLElement;
const view = document.getElementById("view") as HTMLElement;
const status = document.getElementById("status") as HTMLElement;
let cleanup: () => void = () => {};
let turn = 0;

const links: [string, string][] = [
  ["#/", T.nav.projects],
  ["#/graph", T.nav.graph],
  ["#/live", T.nav.live],
  ["#/tokens", T.nav.tokens],
  ["#/learned", T.nav.learned],
  ["#/blind", T.nav.blind],
  ["#/settings", T.nav.settings],
];
const nav = h("nav", {}, links.map(([href, text]) => h("a", { href }, text)));
const find = h("input", {
  type: "search",
  class: "find",
  placeholder: T.find,
  onkeydown: (e: Event) => {
    const k = e as KeyboardEvent;
    const q = (k.target as HTMLInputElement).value.trim();
    if (k.key === "Enter" && q) go(`#/search/${enc(q)}`);
  },
});
mount(bar, h("a", { class: "brand", href: "#/" }, h("img", { src: "icon.svg", alt: "" }), T.app), nav, find);

function mark(hash: string): void {
  for (const a of nav.querySelectorAll("a")) {
    const href = a.getAttribute("href") ?? "";
    a.classList.toggle("on", href === "#/" ? hash === "#/" || hash.startsWith("#/p/") : hash.startsWith(href));
  }
}

async function route(): Promise<void> {
  const hash = location.hash || "#/";
  const mine = ++turn;
  cleanup();
  cleanup = () => {};
  mark(hash);
  view.scrollTop = 0;
  const parts = hash.slice(2).split("?")[0].split("/").map(decodeURIComponent);
  try {
    let done: void | (() => void);
    if (parts[0] === "p" && parts[1]) done = await project(view, parts[1], ((parts[2] as Tab) || "notes") as Tab);
    else if (parts[0] === "n" && parts[1]) done = await note(view, { id: Number(parts[1]) });
    else if (parts[0] === "t" && parts[1]) done = await note(view, { title: parts.slice(1).join("/") });
    else if (parts[0] === "graph") done = await graph(view, parts[1] || undefined);
    else if (parts[0] === "live") done = await live(view);
    else if (parts[0] === "tokens") done = await tokens(view);
    else if (parts[0] === "learned") done = await learned(view);
    else if (parts[0] === "blind") done = await blind(view);
    else if (parts[0] === "settings") done = await settings(view);
    else if (parts[0] === "search" && parts[1]) done = await search(view, parts.slice(1).join("/"));
    else done = await home(view);
    if (mine === turn && done) cleanup = done;
    else if (done) done();
  } catch (e) {
    if (mine === turn) mount(view, h("div", { class: "page" }, h("div", { class: "error" }, String((e as Error).message ?? e))));
  }
}

async function pulse(): Promise<void> {
  const data = await board<Pulse>("pulse").catch(() => null);
  if (!data) {
    mount(status, h("span", {}, h("span", { class: "dot off" }), T.status.offline));
    return;
  }
  mount(
    status,
    h("span", {}, h("span", { class: "dot on" }), T.status.running),
    h("span", {}, T.status.working(data.working)),
    h("span", {}, T.status.queued(data.queued)),
    data.drafts ? h("span", {}, T.status.drafts(data.drafts)) : null,
    h("span", {}, T.status.notes(data.notes)),
    data.problem ? h("span", { class: "error", style: "padding:0" }, data.problem) : null,
  );
}

window.addEventListener("hashchange", () => void route());
void route();
void pulse();
setInterval(() => void pulse(), STATUS_MS);
