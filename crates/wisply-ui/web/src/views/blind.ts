import { board } from "../api";
import { h, mount } from "../dom";
import { T } from "../text";

type Side = { kind: "text" | "image" | "missing"; text?: string; src?: string; cut?: boolean };
type Open = { run: string; name: string; n: number; a: Side; b: Side };

function side(label: string, s: Side): HTMLElement {
  const body =
    s.kind === "image"
      ? h("img", { src: s.src ?? "", alt: label, class: "blind-img" })
      : s.kind === "text"
        ? h("pre", { class: "blind-text" }, s.text ?? "", s.cut ? `\n${T.blind.cut}` : "")
        : h("div", { class: "empty" }, T.blind.missing);
  return h("div", { class: "blind-side" }, h("div", { class: "blind-label" }, label), body);
}

export async function blind(view: HTMLElement): Promise<void> {
  const { open } = await board<{ open: Open[] }>("blind");
  const redraw = () => void blind(view);
  const pick = (o: Open, verdict: string) => async () => {
    await board("blind-pick", o.run, String(o.n), verdict);
    redraw();
  };
  const first = open[0];
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("h1", {}, T.blind.title),
      h("div", { class: "sub" }, T.blind.sub),
      first
        ? h(
            "div",
            {},
            h("div", { class: "muted", style: "margin-bottom:10px" }, T.blind.which(first.name, open.length)),
            h("div", { class: "blind" }, side(T.blind.one, first.a), side(T.blind.two, first.b)),
            h(
              "div",
              { class: "row", style: "margin-top:14px" },
              h("button", { onclick: pick(first, "1") }, T.blind.pickOne),
              h("button", { onclick: pick(first, "2") }, T.blind.pickTwo),
              h("button", { onclick: pick(first, "same") }, T.blind.same),
            ),
          )
        : h("div", { class: "empty" }, T.blind.none),
    ),
  );
}
