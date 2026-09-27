import * as C from "./cdp.js";
import * as T from "./tabs.js";
import * as A from "./actions.js";

const HOST = "dev.qdhshamiro.lighting";
const VERSION = chrome.runtime.getManifest().version;
const { cfg, page, formatSnap, header, guard, tabOf, trunc, LOAD_MS } = A;
let port = null;
let backoff = 300;
let timer = null;

function brandInfo() {
  const brands = (navigator.userAgentData && navigator.userAgentData.brands) || [];
  const pick = (re) => brands.find((b) => re.test(b.brand));
  const b = pick(/brave/i) || pick(/edge/i) || pick(/opera/i) || pick(/vivaldi/i) || pick(/google chrome/i) || pick(/chromium/i);
  const name = b ? b.brand.toLowerCase().replace("microsoft ", "").replace("google ", "") : "chromium";
  return { brand: name, browserVersion: b ? b.version : "" };
}

function post(msg) {
  if (!port) return;
  try {
    port.postMessage(msg);
  } catch (e) {}
}

function schedule() {
  clearTimeout(timer);
  timer = setTimeout(connect, backoff);
  backoff = Math.min(backoff * 2, 30000);
}

async function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    port = null;
    schedule();
    return;
  }
  port.onMessage.addListener(onMessage);
  port.onDisconnect.addListener(() => {
    port = null;
    void chrome.runtime.lastError;
    schedule();
  });
  let focused = false;
  try {
    const w = await chrome.windows.getLastFocused();
    focused = !!w.focused;
  } catch (e) {}
  post(Object.assign({ event: "hello", ext: VERSION, focused, epoch: T.getEpoch() }, brandInfo()));
}

function event(text) {
  post({ event: "note", text });
}

async function onMessage(msg) {
  backoff = 300;
  if (!msg || !msg.cmd) return;
  const id = msg.id;
  const busy = !NO_TAB.has(msg.cmd) && !msg.cmd.startsWith("record") && msg.cmd !== "where";
  const t0 = performance.now();
  try {
    if (busy) await T.paint("orange", msg.group);
    const res = await run(msg.cmd, Object.assign(msg.args || {}, msg.group ? { group: msg.group } : {}), msg.tab);
    if (id) post(Object.assign({ id, ok: true, extMs: performance.now() - t0 }, typeof res === "string" ? { out: res } : res));
  } catch (e) {
    if (busy) T.paint("red", msg.group);
    if (id) post({ id, ok: false, error: String((e && e.message) || e) });
  }
}

async function open(a, tab) {
  const res = await openTab(a, tab);
  const extras = a.quiet || !a.tabId ? {} : await pageExtras(a.tabId).catch(() => ({}));
  return Object.assign(typeof res === "string" ? { out: res } : res, extras, { created: a.created });
}

async function openTab(a, tab) {
  let tabId = a.new ? null : await tabOf(tab, false);
  if (tabId === null) {
    tabId = await T.create(a.group);
    a.created = true;
  } else T.setTarget(tabId);
  a.tabId = tabId;
  await C.attach(tabId).catch(() => {});
  const nav = await T.navigate(tabId, a.url, LOAD_MS);
  await page(tabId, "act.settle", [300, 3000, 0, 150], 0, 8000).catch(() => {});
  let t = await chrome.tabs.get(tabId);
  if (nav.error && nav.error !== "net::ERR_ABORTED") throw new Error("load failed: " + nav.error + " (" + T.short(a.url, 60) + ")");
  if (a.quiet) return header(tabId, { title: t.title, url: t.url });
  const narrow = !!(a.filter || a.scope);
  const opts = { force: true, filter: a.filter || null, scope: a.scope || null, media: !!a.media };
  let res = await page(tabId, "snap", [opts]).catch((e) => ({ error: e.message }));
  if (!narrow && !res.error && !(res.lines || []).length) {
    const peek = await page(tabId, "act.peek", [300]).catch(() => "");
    if (peek) res.peek = peek;
    else {
      await page(tabId, "act.settle", [400, 2500, 2500], 0, 8000).catch(() => {});
      res = await page(tabId, "snap", [opts]).catch((e) => ({ error: e.message }));
      t = await chrome.tabs.get(tabId);
      if (!res.error && !(res.lines || []).length) res.peek = await page(tabId, "act.peek", [300]).catch(() => "");
    }
  }
  if (res.error) return header(tabId, { title: t.title, url: t.url }) + "\n" + res.error;
  return withKeys(formatSnap(tabId, res, narrow ? 0 : cfg.navLines, narrow), res);
}

function withKeys(out, res) {
  return res && res.keys && res.keys.length ? { out, keys: res.keys } : out;
}

async function net(a, tab) {
  const tabId = await tabOf(tab);
  const list = C.netList(tabId);
  if (a.op !== "body") return { calls: list.map((x) => ({ id: x.id, url: x.url, status: x.status, size: x.size })) };
  const hit = list.find((x) => x.id === a.id);
  if (!hit) throw new Error("that call is gone -> try: lighting net");
  const b = await C.netBody(tabId, hit.id).catch((e) => {
    throw new Error("the browser no longer has that answer (" + String(e.message || e).slice(0, 60) + ") -> reload and try again");
  });
  const text = b.base64Encoded ? new TextDecoder().decode(Uint8Array.from(atob(b.body), (c) => c.charCodeAt(0))) : b.body;
  return { url: hit.url, size: hit.size, body: text.slice(0, 200000) };
}

const TOOL_LIST = "JSON.stringify({ api: !!(document.modelContext || navigator.modelContext), tools: [...(window.__ltWebMCP || new Map()).values()].map((t) => ({ name: t.name, description: String(t.description || '').slice(0, 200), readOnly: !!(t.annotations && t.annotations.readOnlyHint), schema: t.inputSchema || null })) })";

async function webmcp(a, tab) {
  const tabId = await tabOf(tab);
  if (a.op === "list") return JSON.parse(await C.evaluate(tabId, TOOL_LIST));
  const name = JSON.stringify(String(a.name || ""));
  const args = JSON.stringify(a.args || {});
  const code = "(async () => { const t = (window.__ltWebMCP || new Map()).get(" + name + "); if (!t) throw new Error('no tool ' + " + name + "); const r = await t.execute(" + args + ", { requestUserInteraction: async (cb) => cb() }); return JSON.stringify(r === undefined ? null : r); })()";
  return { result: await C.evaluate(tabId, code) };
}

async function pageExtras(tabId) {
  const bigJson = C.netList(tabId).filter((x) => x.size > 1024).length;
  const tools = await C.evaluate(tabId, "window.__ltWebMCP ? window.__ltWebMCP.size : 0").catch(() => 0);
  return { net: bigJson, webmcp: tools || 0 };
}

async function mediaAudio(a, tab) {
  const tabId = await tabOf(tab);
  let ref = a.ref;
  if (!ref) {
    const found = await page(tabId, "firstMedia", []);
    if (found.none) throw new Error("no video or audio on this page -> try: lighting listen <url>");
    ref = found.ref;
  }
  if (a.op === "cues") return Object.assign({ ref }, await page(tabId, "media", [ref, "cues"], 0, 8000));
  const t = await chrome.tabs.get(tabId);
  const muted = !!(t.mutedInfo && t.mutedInfo.muted);
  if (!muted) await chrome.tabs.update(tabId, { muted: true }).catch(() => {});
  try {
    if (typeof a.from === "number") await page(tabId, "media", [ref, "seek", a.from], 0, 12000);
    return Object.assign({ ref }, await page(tabId, "media", [ref, "record", a.ms], 0, a.ms + 20000));
  } finally {
    if (!muted) await chrome.tabs.update(tabId, { muted: false }).catch(() => {});
  }
}

async function history(kind, tab) {
  const tabId = await tabOf(tab);
  const nav = T.watchNav(tabId);
  if (kind === "back") await chrome.tabs.goBack(tabId).catch(() => {});
  else if (kind === "forward") await chrome.tabs.goForward(tabId).catch(() => {});
  else await chrome.tabs.reload(tabId);
  await nav.wait(1500, LOAD_MS);
  await page(tabId, "act.settle", [300, 3000, 0, 150], 0, 8000).catch(() => {});
  const res = await page(tabId, "snap", [{ force: true }]);
  return formatSnap(tabId, res, cfg.navLines);
}

async function frameId(tabId, host) {
  const frames = (await chrome.webNavigation.getAllFrames({ tabId })) || [];
  const hit = frames.find((f) => f.frameId !== 0 && f.url && f.url.includes(host));
  if (!hit) throw new Error("no frame matching " + host);
  return hit.frameId;
}

async function snap(a, tab) {
  const tabId = await tabOf(tab);
  const opts = { all: !!a.all, scope: a.scope || null, filter: a.filter || null, diff: !!a.diff, force: !!a.force || !!a.full, media: !!a.media };
  opts.plain = !a.full && !a.frame && !opts.all && !opts.scope && !opts.filter && !opts.diff && !opts.force && !opts.media;
  if (a.frame) {
    const fid = await frameId(tabId, a.frame);
    const res = await page(tabId, "snap", [Object.assign(opts, { force: true })], fid);
    return formatSnap(tabId, res, 0, true).replace(/^e(\d+) /gm, "f" + fid + ".e$1 ").replace(/^\[t\d+\]/, "[t" + T.sid(tabId) + " frame " + a.frame + "]");
  }
  const res = await page(tabId, "snap", [opts]);
  return withKeys(formatSnap(tabId, res, 0, opts.all || !!opts.scope || !!opts.filter || opts.force || opts.diff || opts.media), res);
}

async function text(a, tab) {
  const tabId = await tabOf(tab);
  let why = "";
  if (!a.raw) {
    try {
      const [probe] = await chrome.scripting.executeScript({ target: { tabId }, func: () => !!globalThis.Defuddle });
      if (!probe.result) await chrome.scripting.executeScript({ target: { tabId }, files: ["defuddle.js"] });
    } catch (e) {
      why = String((e && e.message) || e).slice(0, 120);
    }
  }
  const res = await page(tabId, "act.text", [{ filter: a.filter || null, raw: !!a.raw, links: !!a.links, why }], 0, 25000);
  return header(tabId, { title: res.title, url: res.url }) + "\n" + res.text;
}

async function table(a, tab) {
  const tabId = await tabOf(tab);
  const res = await page(tabId, "act.table", [a.target || null]);
  if (res.none) throw new Error("no table found -> try: lighting snap");
  return res.ref + " table " + res.rows.length + " rows\n" + res.rows.join("\n");
}

async function wait(a, tab) {
  const tabId = await tabOf(tab);
  const ms = Number(a.timeout || 10000);
  const t0 = Date.now();
  if (a.ms) {
    await T.sleep(Math.min(Number(a.ms), 60000));
    return "ok (" + a.ms + " ms)";
  }
  if (a.url) {
    while (Date.now() - t0 < ms) {
      if (cfg.abort) throw new Error("stopped by hotkey");
      const t = await chrome.tabs.get(tabId);
      const hit = (t.url || "").toLowerCase().includes(a.url.toLowerCase());
      if (a.gone ? !hit : hit) return "ok (" + (Date.now() - t0) + " ms) -> " + T.short(t.url, 80);
      await T.sleep(100);
    }
    throw new Error("timeout after " + ms + " ms waiting for url " + (a.gone ? "to leave " : "") + a.url);
  }
  const spec = { text: a.text || null, ref: a.ref || null, css: a.css || null, gone: !!a.gone };
  while (Date.now() - t0 < ms) {
    const left = ms - (Date.now() - t0);
    try {
      const r = await page(tabId, "act.waitFor", [spec, Math.min(left, 4000)], 0, Math.min(left, 4000) + 3000);
      if (r.ok) return "ok (" + (Date.now() - t0) + " ms)";
    } catch (e) {
      if (/dialog open/.test(e.message)) throw e;
      await T.sleep(150);
    }
    if (cfg.abort) throw new Error("stopped by hotkey");
  }
  throw new Error("timeout after " + ms + " ms waiting for " + (a.gone ? "gone " : "") + (a.text || a.ref || a.css));
}

async function expect(a, tab) {
  const tabId = await tabOf(tab);
  if (a.url) {
    const t = await chrome.tabs.get(tabId);
    const hit = (t.url || "").toLowerCase().includes(a.url.toLowerCase());
    if (a.gone ? !hit : hit) return "ok";
    throw new Error("fail: url is " + T.short(t.url, 80));
  }
  const spec = { text: a.text || null, ref: a.ref || null, css: a.css || null, gone: !!a.gone };
  const r = await page(tabId, "act.waitFor", [spec, 0]);
  if (r.ok) return "ok";
  throw new Error("fail: " + (a.gone ? "still present: " : "not found: ") + (a.text || a.ref || a.css));
}

function wrap(code) {
  if (/\breturn\b/.test(code)) return "(async()=>{" + code + "\n})()";
  if (/\bawait\b/.test(code) && !/;\s*\S/.test(code.trim().replace(/;\s*$/, ""))) return "(async()=>{return (" + code.trim().replace(/;\s*$/, "") + ")\n})()";
  if (/(^|[;{}\n])\s*(const|let|class)\s/.test(code)) return "{" + code + "\n}";
  return code;
}

async function js(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "js");
  await C.attach(tabId);
  const r = await C.send(tabId, "Runtime.evaluate", { expression: wrap(a.code), awaitPromise: true, returnByValue: true, userGesture: true, timeout: 15000 });
  if (r.exceptionDetails) {
    const d = r.exceptionDetails;
    throw new Error("js: " + String((d.exception && d.exception.description) || d.text).split("\n")[0]);
  }
  const v = r.result;
  if (v.type === "undefined") return "undefined";
  if (v.value === undefined) return v.description || v.type;
  return typeof v.value === "string" ? v.value : JSON.stringify(v.value);
}

async function fetchJson(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "fetch");
  await C.attach(tabId);
  const init = { method: a.method || "GET", credentials: a.cookies ? "include" : "same-origin" };
  if (a.body) {
    init.body = a.body;
    init.headers = { "content-type": /^\s*[{[]/.test(a.body) ? "application/json" : "application/x-www-form-urlencoded" };
  }
  const expr = "fetch(" + JSON.stringify(a.url) + "," + JSON.stringify(init) + ").then(async r=>({status:r.status,type:r.headers.get('content-type')||'',text:await r.text(),url:r.url}))";
  const r = await C.send(tabId, "Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true, timeout: 20000 });
  if (r.exceptionDetails) throw new Error("fetch failed: " + String((r.exceptionDetails.exception && r.exceptionDetails.exception.description) || r.exceptionDetails.text).split("\n")[0]);
  return { fetch: r.result.value };
}

async function shot(a, tab) {
  const tabId = await tabOf(tab);
  const t = await chrome.tabs.get(tabId);
  const win = await chrome.windows.get(t.windowId);
  if (win.state === "minimized") throw new Error("browser window is minimized -> restore it, or use: lighting shot screen");
  return T.showBriefly(tabId, async () => {
    await C.attach(tabId);
    if (a.ref && !a.marks) return { image: await clipShot(tabId, a.ref, a.quality || 70) };
    let crop = null;
    if (a.ref) {
      const r = await page(tabId, "rect", [a.ref]);
      if (r.gone) throw new Error(a.ref + " is gone -> try: lighting snap");
      crop = { x: r.x - r.w / 2, y: r.y - r.h / 2, w: r.w, h: r.h };
    }
    let marks = 0;
    if (a.marks) {
      await page(tabId, "snap", [{ force: true }]);
      marks = (await page(tabId, "act.marks", [true])).n;
    }
    try {
      const m = await C.send(tabId, "Page.getLayoutMetrics");
      const vp = m.cssVisualViewport || m.visualViewport || {};
      const img = await C.send(tabId, "Page.captureScreenshot", { format: "jpeg", quality: a.quality || 70 });
      return { image: img.data, crop, vw: vp.clientWidth, vh: vp.clientHeight, marks };
    } finally {
      if (a.marks) await page(tabId, "act.marks", [false]).catch(() => {});
    }
  });
}

async function frames(a, tab) {
  const tabId = await tabOf(tab);
  const t = await chrome.tabs.get(tabId);
  const win = await chrome.windows.get(t.windowId);
  if (win.state === "minimized") throw new Error("browser window is minimized -> restore it");
  const muted = !!(t.mutedInfo && t.mutedInfo.muted);
  if (!muted) await chrome.tabs.update(tabId, { muted: true }).catch(() => {});
  return T.showBriefly(tabId, async () => {
    await C.attach(tabId);
    const info = await page(tabId, "media", [a.ref, "info"]);
    if (info.gone) throw new Error(a.ref + " is gone -> try: lighting snap");
    if (info.notMedia) throw new Error(a.ref + " is not a video -> try: lighting snap -f video");
    if (!info.seekable && !a.live) Object.assign(info, await page(tabId, "media", [a.ref, "wait"], 0, 8000));
    const live = a.live || !info.seekable;
    const out = [];
    try {
      if (live) {
        for (let i = 0; i < a.count; i++) {
          if (i) await T.sleep(a.every);
          const st = await page(tabId, "media", [a.ref, "state"]);
          out.push({ image: await clipShot(tabId, a.ref, 70), media: st.media, cap: st.cap });
        }
      } else {
        const from = Math.max(0, Math.min(a.from || 0, info.duration - 0.5));
        const to = Math.max(from + 0.5, Math.min(a.to || info.duration, info.duration));
        const n = a.samples || a.count;
        await page(tabId, "media", [a.ref, "prime"], 0, 8000);
        for (let i = 0; i < n; i++) {
          const st = await page(tabId, "media", [a.ref, "seek", from + ((i + 0.5) * (to - from)) / n], 0, 12000);
          out.push({ image: await clipShot(tabId, a.ref, 70), media: st.media, cap: st.cap });
        }
      }
    } finally {
      await page(tabId, "media", [a.ref, "restore", live ? -1 : 0]).catch(() => {});
      if (!muted) await chrome.tabs.update(tabId, { muted: false }).catch(() => {});
    }
    return { frames: out, live, duration: info.duration };
  });
}

async function clipShot(tabId, ref, quality) {
  const r = await page(tabId, "rect", [ref]);
  if (r.gone) throw new Error(ref + " is gone -> try: lighting snap");
  const m = await C.send(tabId, "Page.getLayoutMetrics");
  const vp = m.cssVisualViewport || m.visualViewport || {};
  const clip = { x: r.x - r.w / 2 + (vp.pageX || 0), y: r.y - r.h / 2 + (vp.pageY || 0), width: Math.max(1, r.w), height: Math.max(1, r.h), scale: 1 };
  const img = await C.send(tabId, "Page.captureScreenshot", { format: "jpeg", quality, clip });
  return img.data;
}

async function upload(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "upload");
  await C.attach(tabId);
  const id = "u" + Date.now();
  const m = await page(tabId, "act.mark", [a.ref, id]);
  if (m.gone) throw new Error(a.ref + " is gone -> try: lighting snap");
  try {
    const doc = await C.send(tabId, "DOM.getDocument", { depth: 0 });
    const q = await C.send(tabId, "DOM.querySelector", { nodeId: doc.root.nodeId, selector: '[data-lt-mark="' + id + '"]' });
    if (!q.nodeId) throw new Error("file input not reachable (inside a frame?)");
    await C.send(tabId, "DOM.setFileInputFiles", { nodeId: q.nodeId, files: a.files });
  } finally {
    await page(tabId, "act.unmark", []).catch(() => {});
  }
  return "ok (" + a.files.length + " file" + (a.files.length === 1 ? "" : "s") + " set on " + a.ref + ")";
}

async function dialog(a, tab) {
  const tabId = await tabOf(tab);
  const d = C.dialog(tabId);
  if (!d) return "no dialog open";
  await C.answerDialog(tabId, a.accept !== false, a.text);
  return "ok (" + (a.accept !== false ? "accepted" : "dismissed") + " " + d.type + ' "' + trunc(d.message, 60) + '")';
}

async function downloads() {
  const items = await chrome.downloads.search({ limit: 8, orderBy: ["-startTime"] });
  if (!items.length) return "no downloads";
  return items.map((d) => {
    const st = d.state === "complete" ? "done" : d.state === "interrupted" ? "failed" : Math.round((d.bytesReceived / Math.max(1, d.totalBytes)) * 100) + "%";
    const size = d.fileSize > 0 ? " " + (d.fileSize / 1048576).toFixed(1) + " MB" : "";
    return st + size + " " + (d.filename || d.url);
  }).join("\n");
}

async function innerWidth(tabId) {
  const r = await C.send(tabId, "Runtime.evaluate", { expression: "innerWidth", returnByValue: true }).catch(() => null);
  return r && r.result ? r.result.value : null;
}

async function viewport(a, tab) {
  const tabId = await tabOf(tab);
  await C.attach(tabId);
  const before = await innerWidth(tabId);
  const mobile = a.width <= 600;
  const changed = async (ms) => {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      if ((await innerWidth(tabId)) !== before) return true;
      await T.sleep(25);
    }
    return false;
  };
  if (!a.width) {
    await C.send(tabId, "Emulation.clearDeviceMetricsOverride");
    await C.send(tabId, "Emulation.setTouchEmulationEnabled", { enabled: false }).catch(() => {});
  } else {
    await C.send(tabId, "Emulation.setDeviceMetricsOverride", { width: a.width, height: a.height, deviceScaleFactor: 0, mobile });
    await C.send(tabId, "Emulation.setTouchEmulationEnabled", { enabled: mobile, maxTouchPoints: mobile ? 5 : 1 }).catch(() => {});
  }
  if (!(await changed(250))) await T.showBriefly(tabId, () => changed(500));
  if (!a.width) return "ok (viewport reset)";
  return "ok (viewport " + a.width + "x" + a.height + (mobile ? ", mobile" : "") + ", until lighting viewport reset or 5 min idle)";
}

async function consoleCmd(a, tab) {
  const tabId = await tabOf(tab);
  const fresh = !C.isAttached(tabId);
  await C.enableConsole(tabId);
  const lines = C.consoleLines(tabId).slice(-30);
  if (!lines.length) return fresh ? "console capture is on now; no errors yet (reload to catch load errors)" : "no console errors";
  return lines.join("\n");
}

async function dismiss(a, tab) {
  const tabId = await tabOf(tab);
  await guard(tabId, "dismiss");
  const r = await page(tabId, "act.dismiss", []);
  if (r.none) return "no cookie banner found";
  await page(tabId, "act.settle", [200, 1500]).catch(() => {});
  return 'ok (' + r.kind + ' "' + r.clicked + '")';
}

async function tabCmd(a) {
  const id = T.rid(a.id);
  const t = id === null ? null : await chrome.tabs.get(id).catch(() => null);
  if (!t) throw new Error("no tab " + a.id + " -> try: lighting tabs");
  T.setTarget(id);
  return "target t" + T.sid(id) + " " + trunc(t.title, 50) + " - " + T.short(t.url, 60);
}

async function close(a, tab) {
  const id = a.id ? T.rid(a.id) : await tabOf(tab);
  if (id === null) throw new Error("no tab " + a.id + " -> try: lighting tabs");
  if (!a.force && !(await T.inGroup(id))) throw new Error("t" + T.sid(id) + " is not a Lighting tab -> add --force to close it anyway");
  await chrome.tabs.remove(id);
  if (id === T.getTarget()) T.setTarget(await T.previous(id));
  const next = T.getTarget();
  return "closed t" + T.sid(id) + (next ? ", target now t" + T.sid(next) : "");
}

const seen = new Map();
const NO_TAB = new Set(["config", "abort", "reload-extension", "ping", "close-url", "tabs", "downloads", "cleanup", "keep"]);
const BY_TEXT = new Set(["click", "type", "select", "check", "hover", "fill"]);

function pageKey(url) {
  try {
    const u = new URL(url);
    return u.origin + u.pathname + u.search;
  } catch (e) {
    return String(url || "");
  }
}

async function stale(cmd, a, tab) {
  if (!BY_TEXT.has(cmd) || (cmd !== "fill" && !a.text)) return;
  const id = await tabOf(tab, false).catch(() => null);
  if (id === null || !seen.has(id)) return;
  const t = await chrome.tabs.get(id).catch(() => null);
  if (!t || pageKey(t.url) === seen.get(id)) return;
  seen.set(id, pageKey(t.url));
  throw new Error("the page changed since the last command (now " + T.short(t.url, 70) + "), nothing done -> try: lighting snap, then repeat");
}

const BY_REF = new Set(["click", "type", "select", "check", "hover"]);

async function remember(tab) {
  const id = await tabOf(tab, false).catch(() => null);
  if (id === null) return null;
  const t = await chrome.tabs.get(id).catch(() => null);
  if (!t) return null;
  seen.set(id, pageKey(t.url));
  return { url: t.url, title: t.title, tab: "t" + T.sid(id) };
}

async function describeRef(ref, tab) {
  const id = await tabOf(tab, false).catch(() => null);
  if (id === null) return null;
  const info = await page(id, "act.info", [ref]).catch(() => null);
  return info && !info.gone ? { name: info.name, role: info.role, hint: info.hint } : null;
}

async function run(cmd, a, tab) {
  if (NO_TAB.has(cmd) || cmd.startsWith("record")) return route(cmd, a, tab);
  await stale(cmd, a, tab);
  const target = BY_REF.has(cmd) && a.ref ? await describeRef(a.ref, tab) : null;
  let res, where = null;
  try {
    res = await route(cmd, a, tab);
  } finally {
    where = await remember(typeof a.tabId === "number" ? a.tabId : tab);
  }
  const out = typeof res === "string" ? { out: res } : res || {};
  if (target) out.target = target;
  if (where) out.where = where;
  return out;
}

const rec = { on: false, tabs: new Set(), steps: [], started: 0 };

function recPush(step) {
  if (rec.on) rec.steps.push(Object.assign({ t: Date.now() }, step));
}

async function recInject(tabId) {
  await chrome.scripting.executeScript({ target: { tabId }, files: ["page.js", "page-act.js", "page-rec.js"] }).catch(() => {});
}

async function recStart(a, tab) {
  const tabId = await tabOf(tab);
  const t = await chrome.tabs.get(tabId);
  Object.assign(rec, { on: true, tabs: new Set([tabId]), steps: [], started: Date.now() });
  recPush({ cmd: "open", args: [t.url] });
  await recInject(tabId);
  return { out: "t" + T.sid(tabId) + " " + trunc(t.title, 50) + " - " + T.short(t.url, 60) };
}

async function recStop() {
  const was = rec.on;
  for (const id of rec.tabs) {
    await chrome.scripting.executeScript({ target: { tabId: id }, func: () => globalThis.__lt && globalThis.__lt.rec && globalThis.__lt.rec.stop() }).catch(() => {});
  }
  await T.sleep(80);
  rec.on = false;
  const steps = rec.steps;
  rec.steps = [];
  rec.tabs = new Set();
  return { out: was ? steps.length + " browser steps" : "not recording", steps };
}

chrome.runtime.onMessage.addListener((m, sender) => {
  if (!m || m.lt !== "rec" || !rec.on || !sender.tab || !rec.tabs.has(sender.tab.id) || sender.frameId) return;
  recPush(m.step);
});

chrome.webNavigation.onCommitted.addListener((d) => {
  if (!rec.on || d.frameId !== 0 || !rec.tabs.has(d.tabId)) return;
  const q = d.transitionQualifiers || [];
  if (d.transitionType === "reload") recPush({ cmd: "reload", args: [] });
  else if (q.includes("forward_back") || ["typed", "auto_bookmark", "generated", "keyword", "start_page"].includes(d.transitionType)) recPush({ cmd: "open", args: [d.url] });
});

chrome.webNavigation.onDOMContentLoaded.addListener((d) => {
  if (rec.on && d.frameId === 0 && rec.tabs.has(d.tabId)) recInject(d.tabId);
});

async function route(cmd, a, tab) {
  switch (cmd) {
    case "where":
      return { out: "" };
    case "record-start":
      return recStart(a, tab);
    case "record-stop":
      return recStop();
    case "record-status":
      return { out: rec.on ? rec.steps.length + " browser steps" : "not recording" };
    case "config":
      cfg.blocklist = a.blocklist || [];
      cfg.risk = a.risk || [];
      cfg.pointer = a.pointer !== false;
      T.setOwnWindow(a.window !== false);
      if (a.navLines) cfg.navLines = a.navLines;
      if (a.navChars) cfg.navChars = a.navChars;
      return {};
    case "abort":
      cfg.abort = true;
      setTimeout(() => (cfg.abort = false), 800);
      T.paint("red");
      return {};
    case "reload-extension":
      setTimeout(() => chrome.runtime.reload(), 100);
      return { out: "reloading extension" };
    case "ping":
      return { out: "pong" };
    case "close-url": {
      const all = await chrome.tabs.query({});
      const path = String(a.prefix || "").replace(/^[a-z]+:\/\//, "");
      const hits = all.filter((t) => path && (t.url || t.pendingUrl || "").replace(/^[a-z]+:\/\//, "").startsWith(path)).sort((x, y) => y.id - x.id);
      const n = a.all ? hits.length : Math.min(1, hits.length);
      for (const t of hits.slice(0, n)) await chrome.tabs.remove(t.id).catch(() => {});
      return { out: "closed " + n };
    }
    case "cleanup": {
      await T.paint("green", a.group);
      const only = Array.isArray(a.only) ? new Set(a.only.map((x) => T.rid(x)).filter((x) => x !== null)) : null;
      const tabs = (await T.groupTabs(a.group)).filter((t) => !only || only.has(t.id));
      const own = await T.ownWindows();
      const last = await chrome.windows.getLastFocused().catch(() => null);
      const looked = (t) => t.active && (!own.has(t.windowId) || (last && last.focused && last.id === t.windowId));
      const shut = a.close === false ? [] : tabs.filter((t) => !looked(t)).map((t) => t.id);
      if (shut.length) await chrome.tabs.remove(shut).catch(() => {});
      if (shut.includes(T.getTarget())) T.setTarget(null);
      return { out: "", closed: shut.length, kept: tabs.length - shut.length, shut: shut.map((id) => "t" + T.sid(id)) };
    }
    case "keep": {
      const only = Array.isArray(a.only) ? new Set(a.only.map((x) => T.rid(x)).filter((x) => x !== null)) : null;
      const ids = a.id ? [T.rid(a.id)].filter((x) => x !== null) : (await T.groupTabs(a.group)).map((t) => t.id).filter((id) => !only || only.has(id));
      if (ids.length) await chrome.tabs.ungroup(ids).catch(() => {});
      return { out: "", kept: ids.length };
    }
    case "tabs":
      return T.list();
    case "tab":
      return tabCmd(a);
    case "open":
      return open(a, tab);
    case "back":
    case "forward":
    case "reload":
      return history(cmd, tab);
    case "close":
      return close(a, tab);
    case "snap":
      return snap(a, tab);
    case "text":
      return text(a, tab);
    case "table":
      return table(a, tab);
    case "click":
      return A.click(a, tab);
    case "type":
      return A.type(a, tab);
    case "fill":
      return A.fill(a, tab);
    case "press":
      return A.press(a, tab);
    case "select":
      return A.select(a, tab);
    case "check":
      return A.check(a, tab);
    case "hover":
      return A.hover(a, tab);
    case "drag":
      return A.drag(a, tab);
    case "scroll":
      return A.scroll(a, tab);
    case "wait":
      return wait(a, tab);
    case "search-field":
      return page(await tabOf(tab), "act.searchField", []);
    case "media-audio":
      return mediaAudio(a, tab);
    case "net":
      return net(a, tab);
    case "webmcp":
      return webmcp(a, tab);
    case "expect":
      return expect(a, tab);
    case "js":
      return js(a, tab);
    case "fetch":
      return fetchJson(a, tab);
    case "shot":
      return shot(a, tab);
    case "frames":
      return frames(a, tab);
    case "upload":
      return upload(a, tab);
    case "dialog":
      return dialog(a, tab);
    case "downloads":
      return downloads();
    case "console":
      return consoleCmd(a, tab);
    case "viewport":
      return viewport(a, tab);
    case "dismiss":
      return dismiss(a, tab);
    default:
      throw new Error("unknown browser command " + cmd);
  }
}

C.onEvent((kind, tabId, params) => {
  if (kind === "dialog") event("dialog on t" + T.sid(tabId) + ": " + params.type + ' "' + trunc(params.message, 80) + '" -> lighting dialog accept|dismiss');
});

chrome.webNavigation.onCreatedNavigationTarget.addListener(async (d) => {
  const src = d.sourceTabId;
  if (src !== T.getTarget() && !(await T.inGroup(src))) return;
  if (rec.on && rec.tabs.has(src)) rec.tabs.add(d.tabId);
  await T.addToGroup(d.tabId, (await T.groupOf(src)) || undefined);
  chrome.tabs.update(d.tabId, { autoDiscardable: false }).catch(() => {});
  T.setTarget(d.tabId);
  event("new tab t" + T.sid(d.tabId) + " opened from t" + T.sid(src) + " (now the target; lighting tab t" + T.sid(src) + " to go back)");
});

chrome.downloads.onChanged.addListener(async (d) => {
  if (!d.state || d.state.current !== "complete") return;
  const [item] = await chrome.downloads.search({ id: d.id });
  if (item) event("download done: " + item.filename);
});

chrome.windows.onFocusChanged.addListener((id) => {
  post({ event: "focus", focused: id !== chrome.windows.WINDOW_ID_NONE });
});

chrome.alarms.onAlarm.addListener(() => {
  C.sweep();
  if (!port) connect();
});

const ready = T.restore();
chrome.runtime.onStartup.addListener(async () => {
  await ready;
  await T.fresh();
  if (port) post(Object.assign({ event: "hello", ext: VERSION, focused: false, epoch: T.getEpoch() }, brandInfo()));
  else connect();
});
chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create("lighting", { periodInMinutes: 0.5 });
  connect();
});

chrome.alarms.get("lighting").then((a) => {
  if (!a) chrome.alarms.create("lighting", { periodInMinutes: 0.5 });
});
ready.then(connect);
