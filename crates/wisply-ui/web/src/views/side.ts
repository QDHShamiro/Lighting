import { board, plain } from "../api";
import { h, mount } from "../dom";
import { T } from "../text";

type Lesson = { sig: string; cmd: string; target: string; kind: string; fix: string; ups: number; downs: number };
type Field = { key: string; kind: "bool" | "text" | "pick"; options?: string[] };

const FIELDS: Field[] = [
  { key: "gemini", kind: "bool" },
  { key: "gemini_model", kind: "text" },
  { key: "memory", kind: "bool" },
  { key: "memory_learn", kind: "bool" },
  { key: "learn", kind: "bool" },
  { key: "brain", kind: "bool" },
  { key: "cleanup", kind: "bool" },
  { key: "pointer", kind: "bool" },
  { key: "window", kind: "bool" },
  { key: "focus", kind: "pick", options: ["never", "away", "idle"] },
  { key: "browser", kind: "text" },
  { key: "vault", kind: "text" },
  { key: "vault_inbox", kind: "text" },
];

export async function tokens(view: HTMLElement): Promise<void> {
  const out = h("pre", { class: "md", style: "white-space:pre-wrap;font-family:var(--mono);font-size:13px" });
  const days = (n: string) => async () => {
    out.textContent = T.tokens.loading;
    out.textContent = await plain("tokens", n).catch((e: Error) => e.message);
  };
  const claude = h(
    "button",
    {
      onclick: async () => {
        out.textContent = T.tokens.slow;
        out.textContent = await plain("tokens", "claude", "7").catch((e: Error) => e.message);
      },
    },
    T.tokens.claude,
  );
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("h1", {}, T.tokens.title),
      h("div", { class: "sub" }, T.tokens.sub),
      h("div", { class: "row", style: "margin-bottom:14px" }, h("button", { onclick: days("1") }, T.tokens.today), h("button", { onclick: days("7") }, T.tokens.week), h("button", { onclick: days("30") }, T.tokens.month), claude),
      out,
    ),
  );
  await days("7")();
}

export async function learned(view: HTMLElement): Promise<void> {
  const { lessons } = await board<{ lessons: Lesson[] }>("lessons");
  const redraw = () => void learned(view);
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h("h1", {}, T.learned.title),
      h("div", { class: "sub" }, T.learned.sub),
      h(
        "div",
        { class: "list" },
        lessons.length
          ? lessons.map((l) =>
              h(
                "div",
                { class: "item todo" },
                h("span", { class: "kind" }, l.cmd),
                h("span", { class: "grow" }, l.target ? `${l.target}: ` : "", l.kind, h("span", { class: "muted" }, " → "), h("code", {}, `wisply ${l.fix}`)),
                h("span", { class: "when" }, T.learned.score(l.ups, l.downs)),
                h("span", { class: "acts" }, h("button", { class: "danger", onclick: async () => { await board("unlearn", l.sig); redraw(); } }, T.learned.forget)),
              ),
            )
          : h("div", { class: "empty" }, T.learned.none),
      ),
    ),
  );
}

export async function settings(view: HTMLElement): Promise<void> {
  const cfg = await board<Record<string, unknown>>("settings");
  const note = h("span", { class: "muted" });
  const set = async (key: string, value: string) => {
    note.textContent = await plain("config", key, value).catch((e: Error) => e.message);
  };
  const row = (f: Field) => {
    const v = cfg[f.key];
    let input: HTMLElement;
    if (f.kind === "bool") {
      input = h("input", { type: "checkbox", checked: v === true, onchange: (e: Event) => void set(f.key, (e.target as HTMLInputElement).checked ? "on" : "off") });
    } else if (f.kind === "pick") {
      input = h("select", { onchange: (e: Event) => void set(f.key, (e.target as HTMLSelectElement).value) }, (f.options ?? []).map((o) => h("option", { value: o, selected: o === v }, o)));
    } else {
      input = h("input", {
        type: "text",
        value: String(v ?? ""),
        style: "width:340px",
        onchange: (e: Event) => void set(f.key, (e.target as HTMLInputElement).value),
      });
    }
    return h("div", { class: "item" }, h("div", { class: "grow" }, h("div", {}, T.settings.labels[f.key] ?? f.key), h("div", { class: "body faint" }, f.key)), input);
  };
  mount(view, h("div", { class: "page" }, h("div", { class: "row" }, h("div", { class: "grow" }, h("h1", {}, T.settings.title), h("div", { class: "sub" }, T.settings.sub)), note), h("div", { class: "list" }, FIELDS.map(row))));
}
