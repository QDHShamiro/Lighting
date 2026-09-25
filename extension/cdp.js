const attached = new Map();
const dialogs = new Map();
const logs = new Map();
const listeners = [];
const IDLE_MS = 300000;

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
  }
});

chrome.debugger.onDetach.addListener((src) => {
  attached.delete(src.tabId);
});

chrome.tabs.onRemoved.addListener((tabId) => {
  attached.delete(tabId);
  dialogs.delete(tabId);
  logs.delete(tabId);
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
