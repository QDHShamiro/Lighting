const attached = new Map();
const dialogs = new Map();
const logs = new Map();
const nets = new Map();
const pending = new Map();
const listeners = [];
const IDLE_MS = 300000;
const WEBMCP_HOOK = `(() => {
  if (window.__ltWebMCP) return;
  const tools = new Map();
  window.__ltWebMCP = tools;
  const wrap = (mc) => {
    if (!mc || mc.__ltWrapped) return;
    const reg = mc.registerTool && mc.registerTool.bind(mc);
    if (reg) mc.registerTool = (tool, ...rest) => { try { if (tool && tool.name) tools.set(tool.name, tool); } catch (e) {} return reg(tool, ...rest); };
    const unreg = mc.unregisterTool && mc.unregisterTool.bind(mc);
    if (unreg) mc.unregisterTool = (name, ...rest) => { tools.delete(name); return unreg(name, ...rest); };
    const provide = mc.provideContext && mc.provideContext.bind(mc);
    if (provide) mc.provideContext = (c, ...rest) => { try { tools.clear(); for (const t of (c && c.tools) || []) tools.set(t.name, t); } catch (e) {} return provide(c, ...rest); };
    try { mc.__ltWrapped = true; } catch (e) {}
  };
  try { wrap(document.modelContext); } catch (e) {}
  try { wrap(navigator.modelContext); } catch (e) {}
})();`;

export function onEvent(fn) {
  listeners.push(fn);
}

function emit(kind, tabId, data) {
  for (const fn of listeners) {
    try {
      fn(kind, tabId, data);
    } catch (e) {}
  }
}

export function send(tabId, method, params) {
  return chrome.debugger.sendCommand({ tabId }, method, params || {});
}

export async function attach(tabId) {
  const now = Date.now();
  const a = attached.get(tabId);
  if (a) {
    a.last = now;
    return;
  }
  try {
    await chrome.debugger.attach({ tabId }, "1.3");
  } catch (e) {
    const m = String((e && e.message) || e);
    if (!/already attached/i.test(m)) throw new Error("cannot control this tab (" + m + ")");
  }
  attached.set(tabId, { last: now, runtime: false });
  await send(tabId, "Page.enable").catch(() => {});
  await send(tabId, "Emulation.setFocusEmulationEnabled", { enabled: true }).catch(() => {});
  await send(tabId, "Network.enable", { maxTotalBufferSize: 20000000, maxResourceBufferSize: 5000000 }).catch(() => {});
  await send(tabId, "Page.addScriptToEvaluateOnNewDocument", { source: WEBMCP_HOOK }).catch(() => {});
}

export function netList(tabId) {
  return nets.get(tabId) || [];
}

export async function netBody(tabId, requestId) {
  await attach(tabId);
  return send(tabId, "Network.getResponseBody", { requestId });
}

export async function evaluate(tabId, expression) {
  await attach(tabId);
  const r = await send(tabId, "Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, userGesture: true });
  if (r.exceptionDetails) {
    const d = r.exceptionDetails;
    throw new Error((d.exception && d.exception.description ? d.exception.description : d.text || "error").split("\n")[0]);
  }
  return r.result ? r.result.value : undefined;
}

export function isAttached(tabId) {
  return attached.has(tabId);
}

export function dialog(tabId) {
  return dialogs.get(tabId);
}

export async function answerDialog(tabId, accept, text) {
  await attach(tabId);
  const params = { accept };
  if (text !== undefined && text !== null) params.promptText = String(text);
  await send(tabId, "Page.handleJavaScriptDialog", params);
  dialogs.delete(tabId);
}

export async function enableConsole(tabId) {
  await attach(tabId);
  const a = attached.get(tabId);
  if (a && !a.runtime) {
    a.runtime = true;
    if (!logs.has(tabId)) logs.set(tabId, []);
    await send(tabId, "Runtime.enable");
  }
}

export function consoleLines(tabId) {
  return logs.get(tabId) || [];
}

function remote(args) {
  return (args || [])
    .map((a) => (a.value !== undefined ? String(a.value) : a.description || a.type))
    .join(" ")
    .replace(/\s+/g, " ")
    .slice(0, 200);
}

function push(tabId, line) {
  const list = logs.get(tabId) || [];
  list.push(line);
  if (list.length > 60) list.shift();
  logs.set(tabId, list);
}

chrome.debugger.onEvent.addListener((src, method, params) => {
  const tabId = src.tabId;
  if (method === "Page.javascriptDialogOpening") {
    dialogs.set(tabId, { type: params.type, message: params.message, prompt: params.defaultPrompt });
    emit("dialog", tabId, params);
  } else if (method === "Page.javascriptDialogClosed") {
    dialogs.delete(tabId);
  } else if (method === "Runtime.consoleAPICalled") {
    if (params.type === "error" || params.type === "warning" || params.type === "assert") {
      push(tabId, params.type + ": " + remote(params.args));
    }
  } else if (method === "Runtime.exceptionThrown") {
    const d = params.exceptionDetails || {};
    const text = (d.exception && d.exception.description) || d.text || "exception";
    push(tabId, "exception: " + String(text).split("\n")[0].slice(0, 200));
  } else if (method === "Network.requestWillBeSent") {
    if (params.type === "XHR" || params.type === "Fetch") pending.set(params.requestId, params.request.method);
  } else if (method === "Network.responseReceived") {
    const m = pending.get(params.requestId);
    const mime = (params.response && params.response.mimeType) || "";
    if (m === "GET" && /json/i.test(mime) && params.response.status < 400) {
      const list = nets.get(tabId) || [];
      list.push({ id: params.requestId, url: params.response.url, status: params.response.status, size: 0 });
      if (list.length > 30) list.shift();
      nets.set(tabId, list);
    }
    pending.delete(params.requestId);
  } else if (method === "Network.loadingFinished") {
    const hit = (nets.get(tabId) || []).find((x) => x.id === params.requestId);
    if (hit) hit.size = params.encodedDataLength || 0;
  } else if (method === "Network.loadingFailed") {
    pending.delete(params.requestId);
  }
});

chrome.debugger.onDetach.addListener((src) => {
  attached.delete(src.tabId);
});

chrome.tabs.onRemoved.addListener((tabId) => {
  attached.delete(tabId);
  dialogs.delete(tabId);
  logs.delete(tabId);
  nets.delete(tabId);
});

export function sweep() {
  const now = Date.now();
  for (const [tabId, a] of attached) {
    if (now - a.last > IDLE_MS && !dialogs.has(tabId)) {
      attached.delete(tabId);
      chrome.debugger.detach({ tabId }).catch(() => {});
    }
  }
}

export function detachAll() {
  for (const tabId of attached.keys()) chrome.debugger.detach({ tabId }).catch(() => {});
  attached.clear();
}
