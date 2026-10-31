import { board, type Project, type Todo } from "../api";
import { ago, enc, h, mount } from "../dom";
import { render } from "../md";
import { T } from "../text";
import { graph } from "./graph";

export type Tab = "notes" | "todos" | "sessions" | "graph";

function notes(p: Project): HTMLElement {
  const list = h("div", { class: "list" });
  const draw = (want: string) => {
    const w = want.trim().toLowerCase();
    const shown = p.notes.filter((n) => !w || n.title.toLowerCase().includes(w));
    mount(
      list,
      ...shown.map((n) =>
        h("a", { class: "item", href: `#/n/${n.id}` }, h("span", { class: "kind" }, n.kind || n.source), h("span", { class: "grow ellipsis" }, n.title), h("span", { class: "when" }, T.ago(ago(n.updated)))),
      ),
    );
    if (!shown.length) mount(list, h("div", { class: "empty" }, T.project.noNotes));
  };
  draw("");
  const filter = h("input", { type: "search", placeholder: T.project.filter, oninput: (e: Event) => draw((e.target as HTMLInputElement).value) });
  return h("div", {}, h("div", { class: "row", style: "margin-bottom:10px" }, filter), list);
}

async function setTodo(t: Todo, status: string, task?: string): Promise<void> {
  const args = ["todo", String(t.id), status];
  if (task !== undefined) args.push(`--task=${task}`);
  await board(...args);
}

function todoRow(t: Todo, redraw: () => void): HTMLElement {
  const task = h("span", { class: "task grow" }, t.task);
  const act = (label: string, status: string) =>
    h("button", { onclick: async () => { await setTodo(t, status); redraw(); } }, label);
  const edit = h("button", {
    onclick: async () => {
      if (task.isContentEditable) {
        task.contentEditable = "false";
        await setTodo(t, t.status, task.textContent ?? t.task);
        redraw();
        return;
      }
      task.contentEditable = "true";
      task.focus();
      edit.textContent = T.todo.save;
    },
  }, T.todo.edit);
  const open = t.status === "todo" || t.status === "doing";
  return h(
    "div",
    { class: `item todo ${t.status}` },
    h("span", { class: "box", title: open ? T.todo.done : T.todo.open, onclick: async () => { await setTodo(t, open ? "done" : "todo"); redraw(); } }),
    task,
    t.status === "doing" ? h("span", { class: "tag" }, T.todo.doing) : null,
    t.list ? h("span", { class: "tag" }, t.list) : null,
    h("span", { class: "acts" }, open && t.status !== "doing" ? act(T.todo.doing, "doing") : null, open ? act(T.todo.drop, "dropped") : act(T.todo.open, "todo"), edit),
  );
}

function todos(p: Project, redraw: () => void): HTMLElement {
  const order = (t: Todo) => ({ doing: 0, todo: 1, done: 2, dropped: 3 })[t.status] ?? 4;
  const rows = [...p.todos].sort((a, b) => order(a) - order(b) || b.updated - a.updated);
  const add = h("input", {
    type: "text",
    placeholder: T.project.add,
    style: "width:100%;margin-bottom:10px",
    onkeydown: async (e: Event) => {
      const k = e as KeyboardEvent;
      const input = k.target as HTMLInputElement;
      if (k.key === "Enter" && input.value.trim()) {
        await board("todo-add", p.name, `--task=${input.value.trim()}`);
        redraw();
      }
    },
  });
  return h("div", {}, add, h("div", { class: "list" }, rows.length ? rows.map((t) => todoRow(t, redraw)) : h("div", { class: "empty" }, T.project.noTodos)));
}

function sessions(p: Project): HTMLElement {
  if (!p.sessions.length) return h("div", { class: "empty" }, T.project.noSessions);
  return h(
    "div",
    { class: "list" },
    p.sessions.map((s) =>
      h(
        "div",
        { class: "item", style: "flex-direction:column;align-items:stretch;gap:2px" },
        h("div", { class: "row" }, h("span", { class: "grow ellipsis" }, s.title || s.id.slice(0, 8)), h("span", { class: "when" }, T.ago(ago(s.last)))),
        s.summary ? h("div", { class: "body" }, s.summary) : null,
        s.next ? h("div", { class: "body faint" }, `${T.project.next}: ${s.next}`) : null,
      ),
    ),
  );
}

export async function project(view: HTMLElement, name: string, tab: Tab): Promise<() => void> {
  const p = await board<Project>("project", name);
  const redraw = () => void project(view, name, tab);
  const tabs = (Object.keys(T.project.tabs) as Tab[]).map((k) => h("a", { href: `#/p/${enc(name)}/${k}`, class: k === tab ? "on" : "" }, T.project.tabs[k]));
  const open = p.todos.filter((t) => t.status === "todo" || t.status === "doing").length;
  const about = p.about ? h("div", { class: "md sub" }) : null;
  const lines = p.about.split("\n");
  if (lines[0]?.replace(/^#\s+/, "").trim() === p.name) lines.shift();
  if (about) about.innerHTML = render(lines.slice(0, 6).join("\n"));
  const body = h("div", {});
  mount(
    view,
    h(
      "div",
      { class: tab === "graph" ? "page wide" : "page" },
      tab === "graph" ? null : h("h1", {}, p.name),
      tab === "graph" ? null : h("div", { class: "muted" }, T.home.counts(p.notes.length, open, p.sessions.length).join(" · ")),
      tab === "graph" ? null : about,
      h("div", { class: "tabs", style: tab === "graph" ? "margin:0;padding:0 16px" : "" }, tabs),
      body,
    ),
  );
  if (tab === "notes") mount(body, notes(p));
  else if (tab === "todos") mount(body, todos(p, redraw));
  else if (tab === "sessions") mount(body, sessions(p));
  else {
    body.style.height = "calc(100% - 42px)";
    return graph(body, name);
  }
  return () => {};
}
