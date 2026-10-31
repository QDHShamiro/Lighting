import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { board, save, type Note } from "../api";
import { ago, enc, h, mount } from "../dom";
import { render } from "../md";
import { T } from "../text";

const WATCH_MS = 3000;

const theme = EditorView.theme(
  {
    "&": { backgroundColor: "var(--panel)", color: "var(--text)" },
    ".cm-content": { fontFamily: "var(--mono)", padding: "14px 0", caretColor: "var(--ink)" },
    ".cm-line": { padding: "0 16px" },
    ".cm-cursor": { borderLeftColor: "var(--ink)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "#33333d" },
    ".cm-gutters": { display: "none" },
  },
  { dark: true },
);

const marks = HighlightStyle.define([
  { tag: tags.heading, fontWeight: "700", color: "#ffffff" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.link, color: "#b8b8c4", textDecoration: "underline" },
  { tag: tags.url, color: "#8b8b95" },
  { tag: tags.monospace, color: "#d6d6de" },
  { tag: tags.quote, color: "#8b8b95" },
  { tag: tags.processingInstruction, color: "#5a5a63" },
]);

type Mode = "read" | "edit";

export async function note(view: HTMLElement, spec: { id?: number; title?: string }): Promise<() => void> {
  let n: Note;
  try {
    n = spec.title !== undefined ? await board<Note>("note", `--title=${spec.title}`) : await board<Note>("note", String(spec.id));
  } catch {
    mount(view, h("div", { class: "page" }, h("div", { class: "empty" }, T.note.missing(spec.title ?? String(spec.id)))));
    return () => {};
  }
  let mode: Mode = "read";
  let dirty = false;
  let editor: EditorView | null = null;
  const content = h("div", {});
  const banner = h("div", {});
  const state = h("span", { class: "faint" });
  const toggle = h("button", { onclick: () => show(mode === "read" ? "edit" : "read") }, T.note.edit);
  const saveBtn = h("button", { class: "primary", disabled: true, onclick: () => void store() }, T.note.save);

  const text = () => (editor ? editor.state.doc.toString() : n.body);
  const mark = (on: boolean) => {
    dirty = on;
    saveBtn.disabled = !on;
    state.textContent = on ? T.note.unsaved : "";
  };

  const show = (m: Mode) => {
    if (m === "read" && editor) {
      n.body = editor.state.doc.toString();
      editor.destroy();
      editor = null;
    }
    mode = m;
    toggle.textContent = m === "read" ? T.note.edit : T.note.read;
    if (m === "read") {
      const md = h("div", { class: "md" });
      md.innerHTML = render(n.body);
      mount(content, md);
      return;
    }
    const box = h("div", { class: "editor" });
    mount(content, box);
    editor = new EditorView({
      parent: box,
      state: EditorState.create({
        doc: n.body,
        extensions: [
          history(),
          keymap.of([{ key: "Mod-s", run: () => (void store(), true) }, ...defaultKeymap, ...historyKeymap]),
          markdown(),
          syntaxHighlighting(marks),
          theme,
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => u.docChanged && mark(true)),
        ],
      }),
    });
    editor.focus();
  };

  const store = async (force = false) => {
    if (!n.editable || (!dirty && !force)) return;
    const r = await save(n.path, text(), n.mtime);
    if (r.conflict) {
      conflict(r.mtime);
      return;
    }
    n.mtime = r.mtime;
    n.body = text();
    mark(false);
    state.textContent = T.note.saved;
    mount(banner);
  };

  const conflict = (theirs: number) => {
    mount(
      banner,
      h(
        "div",
        { class: "banner" },
        h("span", { class: "grow" }, T.note.changed),
        h("button", { onclick: () => void reload() }, T.note.reload),
        h("button", { class: "primary", onclick: async () => { n.mtime = theirs; await store(true); } }, T.note.keep),
      ),
    );
  };

  const reload = async () => {
    const fresh = await board<Note>("note", String(n.id));
    n = fresh;
    mark(false);
    mount(banner);
    show(mode);
  };

  const timer = window.setInterval(async () => {
    if (!n.path) return;
    const fresh = await board<Note>("note", String(n.id)).catch(() => null);
    if (!fresh || fresh.mtime === n.mtime) return;
    if (dirty) {
      conflict(fresh.mtime);
      return;
    }
    n = fresh;
    if (editor) editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: n.body } });
    else show("read");
    mark(false);
    state.textContent = T.note.reloaded;
  }, WATCH_MS);

  const side = h(
    "div",
    { class: "side" },
    h("h2", {}, T.note.backlinks),
    n.backlinks.length ? n.backlinks.map((b) => h("a", { href: `#/n/${b.id}` }, b.title)) : h("div", { class: "faint" }, T.note.noBacklinks),
    n.links.length ? h("h2", {}, T.note.links) : null,
    n.links.map((l) => h("a", { href: `#/t/${enc(l)}` }, l)),
    n.tags.length ? h("h2", {}, T.note.tags) : null,
    n.tags.length ? h("div", { class: "chips" }, n.tags.map((t) => h("span", { class: "chip" }, `#${t}`))) : null,
  );
  mount(
    view,
    h(
      "div",
      { class: "page" },
      h(
        "div",
        { class: "note" },
        h(
          "div",
          {},
          h(
            "div",
            { class: "head" },
            h("div", { class: "grow" }, h("h1", {}, n.title), h("div", { class: "muted" }, n.project ? h("a", { href: `#/p/${enc(n.project)}` }, n.project) : "", n.project ? " · " : "", n.kind || n.source, " · ", T.ago(ago(n.updated)))),
            state,
            n.editable ? toggle : h("span", { class: "faint", title: T.note.readOnly }, T.note.readOnly),
            n.editable ? saveBtn : null,
          ),
          banner,
          content,
        ),
        side,
      ),
    ),
  );
  show("read");
  return () => {
    clearInterval(timer);
    editor?.destroy();
  };
}
