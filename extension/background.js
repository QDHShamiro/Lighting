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
  post(Object.assign({ event: "hello", ext: VERSION, focused }, brandInfo()));
}

function event(text) {
  post({ event: "note", text });
}

async function onMessage(msg) {
  backoff = 300;
  if (!msg || !msg.cmd) return;
  const id = msg.id;
  try {
    const res = await run(msg.cmd, msg.args || {}, msg.tab);
    if (id) post(Object.assign({ id, ok: true }, typeof res === "string" ? { out: res } : res));
  } catch (e) {
    if (id) post({ id, ok: false, error: String((e && e.message) || e) });
  }
}

async function open(a, tab) {
  let tabId = a.new ? null : await tabOf(tab, false);
  if (tabId === null) tabId = await T.create();
  else T.setTarget(tabId);
  await C.attach(tabId).catch(() => {});
  const nav = await T.navigate(tabId, a.url, LOAD_MS);
  await page(tabId, "act.settle", [300, 3000], 0, 8000).catch(() => {});
  const t = await chrome.tabs.get(tabId);
  if (nav.error && nav.error !== "net::ERR_ABORTED") throw new Error("load failed: " + nav.error + " (" + T.short(a.url, 60) + ")");
  const res = await page(tabId, "snap", [{ force: true }]).catch((e) => ({ error: e.message }));
  if (res.error) return header(tabId, { title: t.title, url: t.url }) + "\n" + res.error;
  return formatSnap(tabId, res, cfg.navLines);
}

async function history(kind, tab) {
  const tabId = await tabOf(tab);
  const nav = T.watchNav(tabId);
  if (kind === "back") await chrome.tabs.goBack(tabId).catch(() => {});
  else if (kind === "forward") await chrome.tabs.goForward(tabId).catch(() => {});
  else await chrome.tabs.reload(tabId);
  await nav.wait(1500, LOAD_MS);
  await page(tabId, "act.settle", [300, 3000], 0, 8000).catch(() => {});
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
  const opts = { all: !!a.all, scope: a.scope || null, filter: a.filter || null, diff: !!a.diff, force: !!a.force };
  if (a.frame) {
    const fid = await frameId(tabId, a.frame);
    const res = await page(tabId, "snap", [Object.assign(opts, { force: true })], fid);
    return formatSnap(tabId, res).replace(/^e(\d+) /gm, "f" + fid + ".e$1 ").replace(/^\[t\d+\]/, "[t" + T.sid(tabId) + " frame " + a.frame + "]");
  }
  const res = await page(tabId, "snap", [opts]);
  return formatSnap(tabId, res);
}

async function text(a, tab) {
  const tabId = await tabOf(tab);
  if (!a.raw) {
    try {
      const [probe] = await chrome.scripting.executeScript({ target: { tabId }, func: () => !!globalThis.Defuddle });
      if (!probe.result) await chrome.scripting.executeScript({ target: { tabId }, files: ["defuddle.js"] });
    } catch (e) {}
  }
  const res = await page(tabId, "act.text", [{ filter: a.filter || null, raw: !!a.raw }], 0, 25000);
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

async function run(cmd, a, tab) {
  switch (cmd) {
    case "config":
      cfg.blocklist = a.blocklist || [];
      cfg.risk = a.risk || [];
      cfg.pointer = a.pointer !== false;
      if (a.navLines) cfg.navLines = a.navLines;
      return {};
    case "abort":
      cfg.abort = true;
      setTimeout(() => (cfg.abort = false), 800);
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
    case "expect":
      return expect(a, tab);
    case "js":
      return js(a, tab);
    case "fetch":
      return fetchJson(a, tab);
    case "shot":
      return shot(a, tab);
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
  await T.addToGroup(d.tabId);
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

chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create("lighting", { periodInMinutes: 0.5 });
  connect();
});

chrome.alarms.get("lighting").then((a) => {
  if (!a) chrome.alarms.create("lighting", { periodInMinutes: 0.5 });
});
T.restore().then(connect);
