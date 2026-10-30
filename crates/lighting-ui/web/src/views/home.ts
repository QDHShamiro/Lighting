import { board, type Home } from "../api";
import { ago, enc, h, mount } from "../dom";
import { T } from "../text";

export async function home(view: HTMLElement): Promise<void> {
  const data = await board<Home>("home");
  const grid = h("div", { class: "cards" });
  const draw = (filter: string) => {
    const want = filter.trim().toLowerCase();
    const shown = data.projects.filter((p) => !want || p.name.toLowerCase().includes(want) || p.title.toLowerCase().includes(want));
    mount(
      grid,
      ...shown.map((p) =>
        h(
          "a",
          { class: "card", href: `#/p/${enc(p.name)}` },
          h("div", { class: "name ellipsis" }, p.name),
          h("div", { class: "title ellipsis" }, p.title),
          h("div", { class: "meta" }, ...T.home.counts(p.notes, p.todos, p.sessions).map((c) => h("span", {}, c)), h("span", { class: "grow" }), p.last ? T.ago(ago(p.last)) : ""),
        ),
      ),
    );
    if (!shown.length) mount(grid, h("div", { class: "empty" }, T.home.none));
  };
  const filter = h("input", { type: "search", placeholder: T.home.filter, oninput: (e: Event) => draw((e.target as HTMLInputElement).value) });
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("div", { class: "row" }, h("div", { class: "grow" }, h("h1", {}, T.home.title), h("div", { class: "sub" }, T.home.sub(data.projects.length))), filter),
      grid,
    ),
  );
  draw("");
}
