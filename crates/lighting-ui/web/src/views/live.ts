import { board, plain, type Live } from "../api";
import { enc, h, mount } from "../dom";
import { T } from "../text";

export async function live(view: HTMLElement): Promise<() => void> {
  const list = h("div", { class: "list live" });
  const note = h("span", { class: "muted" });
  const stop = h(
    "button",
    {
      class: "danger",
      onclick: async () => {
        await plain("abort").catch(() => "");
        note.textContent = T.live.stopped;
      },
    },
    T.live.stop,
  );
  mount(view, h("div", { class: "page" }, h("div", { class: "row" }, h("div", { class: "grow" }, h("h1", {}, T.live.title), h("div", { class: "sub" }, T.live.sub)), note, stop), list));
  let poll = 2000;
  const draw = async () => {
    const data = await board<Live>("live").catch(() => null);
    if (!data) return;
    poll = data.poll_ms;
    if (!data.sessions.length) {
      mount(list, h("div", { class: "empty" }, T.live.none));
      return;
    }
    mount(
      list,
      ...data.sessions.map((s) => {
        const working = s.age <= data.working_s;
        const who = s.title || (s.sid ? s.sid.slice(0, 8) : T.live.terminal);
        return h(
          "div",
          { class: "item" },
          h("span", { class: `pill ${working ? "on" : ""}` }, working ? T.live.working : T.live.idle),
          h("span", { class: "grow ellipsis" }, s.project ? h("a", { href: `#/p/${enc(s.project)}` }, s.project) : "", s.project ? " · " : "", who),
          h("span", { class: `cmd ${s.ok ? "" : "pill err"}`, title: s.error ?? "" }, [s.cmd, s.ref].filter(Boolean).join(" ")),
          h("span", { class: "when" }, T.live.calls(s.count)),
          h("span", { class: "when" }, T.ago(s.age)),
        );
      }),
    );
  };
  await draw();
  let alive = true;
  const loop = async () => {
    while (alive) {
      await new Promise((r) => setTimeout(r, poll));
      if (alive) await draw();
    }
  };
  void loop();
  return () => {
    alive = false;
  };
}
