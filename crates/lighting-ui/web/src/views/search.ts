import { board, type Hit } from "../api";
import { ago, h, mount } from "../dom";
import { T } from "../text";

export async function search(view: HTMLElement, q: string): Promise<void> {
  const { hits } = await board<{ hits: Hit[] }>("search", `--words=${q}`);
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("h1", {}, T.search.title),
      h("div", { class: "sub" }, T.search.sub(q)),
      h(
        "div",
        { class: "list" },
        hits.length
          ? hits.map((n) =>
              h(
                "a",
                { class: "item", href: `#/n/${n.id}`, style: "flex-direction:column;align-items:stretch;gap:2px" },
                h("div", { class: "row" }, h("span", { class: "grow ellipsis" }, n.title), h("span", { class: "kind" }, n.project), h("span", { class: "when" }, T.ago(ago(n.updated)))),
                n.snippet ? h("div", { class: "body ellipsis" }, n.snippet) : null,
              ),
            )
          : h("div", { class: "empty" }, T.search.none),
      ),
    ),
  );
}
