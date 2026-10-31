import { board, type Card, type Home } from "../api";
import { ago, enc, h, mount } from "../dom";
import { T } from "../text";

type Sort = "active" | "notes" | "name";

const PINS = "wisply.pins";
const SORT = "wisply.sort";
const WEEK = 7 * 86400;
const DAY = 86400;

function pins(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(PINS) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function sorted(list: Card[], by: Sort): Card[] {
  const out = [...list];
  if (by === "notes") out.sort((a, b) => b.notes - a.notes || b.last - a.last);
  else if (by === "name") out.sort((a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }));
  else out.sort((a, b) => b.last - a.last);
  return out;
}

export async function home(view: HTMLElement): Promise<void> {
  const data = await board<Home>("home");
  const pinned = pins();
  let by = (localStorage.getItem(SORT) as Sort | null) ?? "active";
  let want = "";
  const body = h("div", {});

  const card = (p: Card): HTMLElement => {
    const on = pinned.has(p.name);
    const age = p.last ? ago(p.last) : Infinity;
    const star = h(
      "button",
      {
        class: `pin${on ? " on" : ""}`,
        title: on ? T.home.unpin : T.home.pin,
        onclick: (e: Event) => {
          e.preventDefault();
          e.stopPropagation();
          if (pinned.has(p.name)) pinned.delete(p.name);
          else pinned.add(p.name);
          localStorage.setItem(PINS, JSON.stringify([...pinned]));
          draw();
        },
      },
      on ? "★" : "☆",
    );
    return h(
      "a",
      { class: "card", href: `#/p/${enc(p.name)}` },
      h("div", { class: "row" }, h("span", { class: `pulse${age < DAY ? " hot" : age < WEEK ? " warm" : ""}` }), h("div", { class: "name ellipsis grow" }, p.name), star),
      h("div", { class: "title ellipsis" }, p.title),
      h("div", { class: "meta" }, ...T.home.counts(p.notes, p.todos, p.sessions).map((c) => h("span", {}, c)), h("span", { class: "grow" }), p.last ? T.ago(age) : ""),
    );
  };

  const section = (title: string, list: Card[]): HTMLElement | null =>
    list.length ? h("section", {}, h("h2", {}, `${title} · ${list.length}`), h("div", { class: "cards" }, ...list.map(card))) : null;

  const draw = () => {
    const w = want.trim().toLowerCase();
    const shown = sorted(
      data.projects.filter((p) => !w || p.name.toLowerCase().includes(w) || p.title.toLowerCase().includes(w)),
      by,
    );
    if (!shown.length) {
      mount(body, h("div", { class: "empty" }, T.home.none));
      return;
    }
    const top = shown.filter((p) => pinned.has(p.name));
    const rest = shown.filter((p) => !pinned.has(p.name));
    if (w || by !== "active") {
      mount(body, section(T.home.pinned, top), section(T.home.all, rest));
      return;
    }
    const recent = rest.filter((p) => p.last && ago(p.last) < WEEK);
    const older = rest.filter((p) => !p.last || ago(p.last) >= WEEK);
    mount(body, section(T.home.pinned, top), section(T.home.recent, recent), section(T.home.older, older));
  };

  const tabs = h(
    "div",
    { class: "seg" },
    (["active", "notes", "name"] as const).map((s) =>
      h(
        "button",
        {
          class: s === by ? "on" : "",
          onclick: (e: Event) => {
            by = s;
            localStorage.setItem(SORT, s);
            for (const b of tabs.querySelectorAll("button")) b.classList.toggle("on", b === e.currentTarget);
            draw();
          },
        },
        T.home.sort[s],
      ),
    ),
  );
  const filter = h("input", {
    type: "search",
    placeholder: T.home.filter,
    oninput: (e: Event) => {
      want = (e.target as HTMLInputElement).value;
      draw();
    },
  });
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("div", { class: "row head" }, h("div", { class: "grow" }, h("h1", {}, T.home.title), h("div", { class: "sub" }, T.home.sub(data.projects.length))), tabs, filter),
      body,
    ),
  );
  draw();
}
