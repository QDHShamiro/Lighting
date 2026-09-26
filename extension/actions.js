import * as C from "./cdp.js";
import * as T from "./tabs.js";
import * as I from "./input.js";

export const cfg = { blocklist: [], risk: [], pointer: true, navLines: 40, navChars: 1600, abort: false };

const LOAD_MS = 20000;

function trunc(s, n) {
  s = String(s || "");
  return s.length > n ? s.slice(0, n - 3) + "..." : s;
}

function blockedHost(url) {
  try {
    const h = new URL(url).hostname.replace(/^www\./, "").toLowerCase();
    return cfg.blocklist.find((d) => h === d || h.endsWith("." + d)) || null;
  } catch (e) {
    return null;
  }
}

function risky(name) {
  if (!cfg.risk.length || !name) return false;
  const n = name.toLowerCase();
  return cfg.risk.some((w) => new RegExp("(^|[^\\p{L}])" + w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "([^\\p{L}]|$)", "u").test(n));
}

function callLt(fn, args) {
  const L = globalThis.__lt;
  if (!L || !L.act) return { __missing: true };
  const dot = fn.indexOf(".");
  const f = dot > 0 ? L[fn.slice(0, dot)][fn.slice(dot + 1)] : L[fn];
  return f.apply(null, args || []);
}

function friendly(e, url) {
  const m = String((e && e.message) || e);
  if (/cannot access|cannot be scripted|chrome:\/\/|brave:\/\/|edge:\/\/|extensions gallery|Missing host permission/i.test(m)) {
    return new Error("cannot script this page (" + T.short(url || "", 60) + ") -> try: lighting windows, then lighting snap w<N> (desktop control)");
  }
  if (/No tab with id|Tab .* not found/i.test(m)) return new Error("tab is gone -> try: lighting tabs");
  if (/Frame with ID .* was removed|document.*unloaded|Execution context was destroyed/i.test(m)) return new Error("page navigated during the command -> try: lighting snap");
  return e instanceof Error ? e : new Error(m);
}

async function withDialog(tabId, promise, ms) {
  let timer;
  const guard = new Promise((_, reject) => {
    const t0 = Date.now();
    const tick = () => {
      const d = C.dialog(tabId);
      if (d) return reject(new Error("dialog open: " + d.type + ' "' + trunc(d.message, 80) + '" -> try: lighting dialog accept|dismiss'));
      if (Date.now() - t0 > ms) return reject(new Error("page did not answer within " + Math.round(ms / 1000) + "s"));
      timer = setTimeout(tick, 100);
    };
    timer = setTimeout(tick, 100);
  });
  try {
    return await Promise.race([promise, guard]);
  } finally {
    clearTimeout(timer);
  }
}

export async function page(tabId, fn, args, frameId, ms) {
  const target = { tabId, frameIds: [frameId || 0] };
  const call = () => chrome.scripting.executeScript({ target, func: callLt, args: [fn, args || []] });
  let tab = null;
  try {
    tab = await chrome.tabs.get(tabId);
    let [r] = await withDialog(tabId, call(), ms || 15000);
    if (r && r.result && r.result.__missing) {
      await chrome.scripting.executeScript({ target, files: ["page.js", "page-act.js"] });
      [r] = await withDialog(tabId, call(), ms || 15000);
    }
    if (!r) throw new Error("page did not answer");
    return r.result;
  } catch (e) {
    throw friendly(e, tab && tab.url);
  }
}

async function tabOf(requested, need) {
  const id = await T.resolve(requested);
  if (id === null && need !== false) throw new Error("no tab yet -> try: lighting open <url>  (or lighting tabs + lighting tab <id>)");
  return id;
}

async function guard(tabId, what) {
  const tab = await chrome.tabs.get(tabId);
  const b = blockedHost(tab.url || tab.pendingUrl || "");
  if (b) throw new Error("blocked: " + b + " is read-only for lighting (" + what + ") - edit ~/.lighting/blocklist.txt to change");
  return tab;
}

function header(tabId, res) {
  let h = "[t" + T.sid(tabId) + "] " + trunc(res.title || "(no title)", 70) + " - " + T.short(res.url || "", 80);
  if (res.scroll !== undefined) h += " (scroll " + res.scroll + "%)";
  return h;
}

const heads = new Map();

function sameHead(tabId, res, lines) {
  const bare = lines.map((l) => (/^e\d+ /.test(l) ? l.replace(/^e\d+ /, "").replace(/ ->\S*$/, "").replace(/ \*$/, "") : null));
  let origin = "";
  try {
    origin = new URL(res.url).origin;
  } catch (e) {}
  const prev = heads.get(tabId);
  heads.set(tabId, { origin, bare: bare.slice(0, 60), url: res.url });
  if (!prev || prev.origin !== origin || prev.url === res.url) return lines;
  let n = 0;
  while (n < lines.length && bare[n] !== null && bare[n] === prev.bare[n]) n++;
  if (n < 6) return lines;
  const first = lines[0].split(" ")[0], last = lines[n - 1].split(" ")[0];
  return [first + "-" + last + " same header as the last page (" + n + " items, lighting snap --all)"].concat(lines.slice(n));
}

function formatSnap(tabId, res, limit, whole) {
  if (res.error) throw new Error(res.error);
  if (res.unchanged) return header(tabId, res) + "\nunchanged since last snap (refs still valid)";
  const out = [header(tabId, res)];
  for (const n of res.notes || []) out.push(n);
  let lines = res.lines || [];
  if (!whole && res.removed === undefined) lines = sameHead(tabId, res, lines);
  let cut = 0;
  if (limit) {
    let n = 0;
    for (let chars = 0; n < lines.length && n < limit && chars + lines[n].length <= cfg.navChars; n++) chars += lines[n].length + 1;
    n = Math.max(n, Math.min(lines.length, 1));
    cut = lines.length - n;
    lines = lines.slice(0, n);
  }
  out.push(...lines);
  if (!lines.length && !res.searched) out.push(res.removed !== undefined ? "(nothing new)" : "(no interactive elements in view)");
  if (res.removed) out.push("-" + res.removed + " gone");
  if (cut) out.push("... +" + cut + " more (lighting snap)");
  const more = [];
  if (res.below) more.push(res.below + " below");
  if (res.above) more.push(res.above + " above");
  if (more.length) out.push("more: " + more.join(", ") + " (scroll or snap --all)");
  return out.join("\n");
}

function dialogText(d) {
  return "dialog open: " + d.type + ' "' + trunc(d.message, 80) + '" -> lighting dialog accept|dismiss';
}

async function dispatch(tabId, promise) {
  const t0 = Date.now();
  let done = false;
  promise.then(() => (done = true), () => (done = true));
  while (!done) {
    const d = C.dialog(tabId);
    if (d) return dialogText(d);
    if (Date.now() - t0 > 15000) throw new Error("the page did not react within 15s");
    await T.sleep(20);
  }
  await promise;
  const d = C.dialog(tabId);
  return d ? dialogText(d) : null;
}

async function point(tabId, x, y) {
  if (!cfg.pointer) return;
  await page(tabId, "act.pointer", [x, y], 0, 2000).catch(() => {});
}

function leavesPage(r, before) {
  if (!r.href || r.blank || /^javascript:/i.test(r.href)) return false;
  return r.href.split("#")[0] !== String(before || "").split("#")[0];
}

async function titleChange(tabId, title, ms) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const t = await chrome.tabs.get(tabId).catch(() => null);
    if (!t || t.title !== title) return;
    await T.sleep(50);
  }
}

async function after(tabId, nav, beforeUrl, extra, mark, startBudget, beforeTitle, target) {
  const quiet = page(tabId, "act.settle", [100, 1500], 0, 8000).catch(() => null);
  const st = await nav.wait(startBudget || 100, LOAD_MS);
  if (mark) mark("nav");
  const navigated = st.started || st.dcl;
  if (navigated || st.spa) {
    await quiet;
    if (st.spa && !navigated && beforeTitle !== undefined) await titleChange(tabId, beforeTitle, 3000);
    await page(tabId, "act.settle", [250, 3000], 0, 8000).catch(() => {});
  } else {
    await quiet;
  }
  if (mark) mark("settle");
  let tab = null;
  try {
    tab = await chrome.tabs.get(tabId);
  } catch (e) {
    return "ok" + (extra || "") + " (tab closed)";
  }
  if (navigated || (tab.url && tab.url !== beforeUrl)) {
    const res = await page(tabId, "snap", [{ force: true }]).catch(() => null);
    const tail = res ? "\n" + formatSnap(tabId, res, cfg.navLines) : "";
    return "ok" + (extra || "") + " -> " + T.short(tab.url, 80) + (st.error ? " (" + st.error + ")" : "") + tail;
  }
  let d = await page(tabId, "diff", []).catch(() => null);
  if (d && !d.added.length && !d.removed && target) {
    const now = await page(tabId, "act.lineOf", [target.ref]).catch(() => null);
    if (now && now !== target.line) return "ok" + (extra || "") + " -> " + now;
    await page(tabId, "act.settle", [150, 2000, 500], 0, 8000).catch(() => {});
    d = await page(tabId, "diff", []).catch(() => null);
  }
  if (!d || (!d.added.length && !d.removed)) return "ok" + (extra || "");
  const shown = d.added.slice(0, 5);
  let s = "ok" + (extra || "");
  if (d.added.length) s += " (+" + d.added.length + " new" + (d.removed ? ", -" + d.removed + " gone" : "") + ")\n" + shown.join("\n");
  else s += " (-" + d.removed + " gone)";
  if (d.added.length > shown.length) s += "\n... +" + (d.added.length - shown.length) + " more (lighting snap --diff)";
  return s;
}

async function resolveRef(tabId, a) {
  if (a.ref) return { ref: a.ref };
  if (!a.text) throw new Error("need a ref (e12) or \"text\"");
  const f = await page(tabId, "act.find", [a.text, { role: a.role, first: a.first }]);
  if (f.none) throw new Error('nothing matches "' + a.text + '" -> try: lighting snap -f "' + a.text.split(" ")[0] + '"');
  if (f.ambiguous) throw new Error('"' + a.text + '" matches several:\n' + f.ambiguous.join("\n") + "\n-> use a ref");
  return f;
}

export async function click(a, tab) {
  const t0 = Date.now();
  const marks = [];
  const mark = (n) => marks.push(n + " " + (Date.now() - t0));
  const tabId = await tabOf(tab);
  await guard(tabId, "click");
  mark("guard");
  const f = await resolveRef(tabId, a);
  mark("find");
  const r = await page(tabId, "rect", [f.ref]);
  mark("rect");
  if (r.gone) throw new Error(f.ref + " is gone (page changed) -> try: lighting snap");
  if (r.covered && !a.force) throw new Error(f.ref + " is covered by " + r.covered + " -> try: lighting dismiss, press Escape, or click --force");
  if (!a.yes && risky(r.name)) throw new Error('confirm: "' + trunc(r.name, 50) + '" looks irreversible -> rerun with --yes');
  const bt = await chrome.tabs.get(tabId);
  const before = bt.url;
  await point(tabId, r.x, r.y);
  mark("point");
  const nav = T.watchNav(tabId);
  const dlg = await dispatch(tabId, I.click(tabId, r.x, r.y, { button: a.button, count: a.count }));
  mark("click");
  if (dlg) {
    nav.stop();
    return "ok" + (a.text ? " " + f.ref : "") + ", then " + dlg;
  }
  const res = await after(tabId, nav, before, a.text ? " " + f.ref : "", a.trace ? mark : null, leavesPage(r, before) ? 3000 : 100, bt.title, { ref: f.ref, line: r.line });
  return a.trace ? res + "\ntrace: " + marks.join(", ") + " | input: " + I.timings.join(", ") : res;
}

export async function type(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "type");
  const f = await resolveRef(tabId, a);
  const r = await page(tabId, "rect", [f.ref]);
  if (r.gone) throw new Error(f.ref + " is gone -> try: lighting snap");
  await point(tabId, r.x, r.y);
  const fo = await page(tabId, "act.focusFor", [f.ref, !!a.append]);
  if (!fo.focused) await I.click(tabId, r.x, r.y);
  const text = String(a.value || "");
  await I.insertText(tabId, text);
  const v = await page(tabId, "act.valueOf", [f.ref]).catch(() => ({ value: text }));
  if (typeof v.value === "string" && text && !v.value.includes(text)) await page(tabId, "act.setValue", [f.ref, a.append ? v.value + text : text]);
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  if (a.enter) await I.press(tabId, "Enter");
  const shown = a.secret ? text.length + " secret chars" : text.length + " chars";
  return after(tabId, nav, before, " (typed " + shown + ")");
}

function truthy(v) {
  return /^(1|true|on|yes|x|checked|an|ja)$/i.test(String(v).trim());
}

export async function fill(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "fill");
  const pairs = a.pairs || [];
  const found = await page(tabId, "act.fields", [pairs.map((p) => p.label)]);
  const done = [];
  let lastRef = null;
  for (let i = 0; i < found.length; i++) {
    const fld = found[i];
    const value = pairs[i].value;
    if (fld.none) throw new Error('no field for "' + fld.label + '" (filled: ' + (done.join(", ") || "none") + ") -> try: lighting snap -f " + fld.label.split(" ")[0]);
    const r = await page(tabId, "rect", [fld.ref]);
    if (fld.role === "checkbox" || fld.role === "radio" || fld.role === "switch") {
      const st = await page(tabId, "act.checked", [fld.ref]);
      if (st.on !== truthy(value) || fld.role === "radio") await I.click(tabId, r.x, r.y);
    } else if (fld.tag === "SELECT") {
      const s = await page(tabId, "act.selectOpt", [fld.ref, value]);
      if (s.missing) throw new Error('"' + value + '" not in ' + fld.label + ": " + s.missing.join(" | "));
    } else {
      await point(tabId, r.x, r.y);
      const fo = await page(tabId, "act.focusFor", [fld.ref, false]);
      if (!fo.focused) await I.click(tabId, r.x, r.y);
      await I.insertText(tabId, String(value));
      const v = await page(tabId, "act.valueOf", [fld.ref]).catch(() => ({ value }));
      if (typeof v.value === "string" && value && !v.value.includes(String(value))) await page(tabId, "act.setValue", [fld.ref, String(value)]);
    }
    done.push(fld.ref + " " + fld.label);
    lastRef = fld.ref;
  }
  let extra = " (filled " + done.join(", ") + ")";
  if (!a.submit || !lastRef) return "ok" + extra;
  const sub = await page(tabId, "act.submitOf", [lastRef]);
  if (sub.ref && !a.yes && risky(sub.name)) throw new Error('filled, but submit "' + trunc(sub.name, 50) + '" looks irreversible -> rerun with --yes, or: lighting click ' + sub.ref + " --yes");
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  let dlg = null;
  if (sub.ref) {
    const r = await page(tabId, "rect", [sub.ref]);
    await point(tabId, r.x, r.y);
    dlg = await dispatch(tabId, I.click(tabId, r.x, r.y));
    extra += ", submitted via " + sub.ref;
  } else {
    dlg = await dispatch(tabId, I.press(tabId, "Enter"));
    extra += ", submitted with Enter";
  }
  if (dlg) {
    nav.stop();
    return "ok" + extra + ", then " + dlg;
  }
  return after(tabId, nav, before, extra);
}

export async function press(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "press");
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  for (const k of a.keys) {
    const dlg = await dispatch(tabId, I.press(tabId, k));
    if (dlg) {
      nav.stop();
      return "ok, then " + dlg;
    }
  }
  return after(tabId, nav, before);
}

export async function select(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "select");
  const f = await resolveRef(tabId, a);
  const s = await page(tabId, "act.selectOpt", [f.ref, a.option]);
  if (s.gone) throw new Error(f.ref + " is gone -> try: lighting snap");
  if (s.notSelect) throw new Error(f.ref + " is not a <select> -> click it, then click the option text");
  if (s.missing) throw new Error('"' + a.option + '" not found, options: ' + s.missing.join(" | "));
  const before = (await chrome.tabs.get(tabId)).url;
  return after(tabId, T.watchNav(tabId), before, ' (selected "' + s.chosen + '")');
}

export async function check(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "check");
  const f = await resolveRef(tabId, a);
  const st = await page(tabId, "act.checked", [f.ref]);
  if (st.gone) throw new Error(f.ref + " is gone -> try: lighting snap");
  const want = a.on !== false;
  if (st.on === want) return "ok (already " + (want ? "on" : "off") + ")";
  const r = await page(tabId, "rect", [f.ref]);
  await point(tabId, r.x, r.y);
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  await I.click(tabId, r.x, r.y);
  return after(tabId, nav, before, " (" + (want ? "on" : "off") + ")");
}

export async function hover(a, tab) {
  const tabId = await tabOf(tab);
  const f = await resolveRef(tabId, a);
  const r = await page(tabId, "rect", [f.ref]);
  if (r.gone) throw new Error(f.ref + " is gone -> try: lighting snap");
  await point(tabId, r.x, r.y);
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  await I.move(tabId, r.x, r.y);
  return after(tabId, nav, before);
}

export async function drag(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "drag");
  const r1 = await page(tabId, "rect", [a.from]);
  const r2 = await page(tabId, "rect", [a.to, { noScroll: true }]);
  if (r1.gone || r2.gone) throw new Error("a ref is gone -> try: lighting snap");
  await point(tabId, r1.x, r1.y);
  const before = (await chrome.tabs.get(tabId)).url;
  const nav = T.watchNav(tabId);
  await I.drag(tabId, r1, r2);
  return after(tabId, nav, before);
}

export async function scroll(a, tab) {
  const tabId = await tabOf(tab);
  const info = await page(tabId, "act.scrollInfo", []);
  let x = Math.round(info.vw / 2), y = Math.round(info.vh / 2);
  if (a.ref) {
    const r = await page(tabId, "rect", [a.ref, { noScroll: true }]);
    if (r.gone) throw new Error(a.ref + " is gone -> try: lighting snap");
    x = r.x;
    y = r.y;
  }
  const dir = a.dir || "down";
  const end = dir === "up" || dir === "top" ? "top" : "end";
  const move = async (dy, to) => {
    const s = await page(tabId, "act.scrollBy", [x, y, dy, to]);
    if (s.moved || s.scrollable) return s;
    await I.wheel(tabId, x, y, to === "top" ? -info.h : to === "bottom" ? info.h : dy);
    return null;
  };
  let last = null;
  if (dir === "top" || dir === "bottom") {
    last = await move(0, dir);
  } else if (a.until) {
    const max = a.max || 12;
    for (let i = 0; i < max; i++) {
      if (cfg.abort) throw new Error("stopped by hotkey");
      const v = await page(tabId, "act.visibleText", [a.until]);
      if (v.ok) return 'found "' + trunc(a.until, 40) + '" after ' + i + " scrolls";
      last = await move((dir === "up" ? -1 : 1) * Math.round(info.vh * 0.8));
      if (last && !last.moved) throw new Error('"' + a.until + '" not found (reached the ' + end + " after " + i + " scrolls)");
      await page(tabId, "act.settle", [150, 1200]).catch(() => {});
    }
    throw new Error('"' + a.until + '" not found after ' + max + " scrolls");
  } else {
    const px = a.delta ? Number(a.delta) : Math.round(info.vh * 0.8);
    last = await move(dir === "up" ? -px : px);
  }
  await page(tabId, "act.settle", [150, 1500]).catch(() => {});
  if (last) return "ok (scroll " + last.pct + "%" + (last.moved ? "" : ", already at the " + end) + ")";
  const now = await page(tabId, "act.scrollInfo", []);
  const pct = Math.round((now.y / Math.max(1, now.h - now.vh)) * 100);
  return "ok (scroll " + Math.max(0, Math.min(100, pct)) + "%)";
}

export { formatSnap, header, guard, tabOf, blockedHost, trunc, LOAD_MS };
