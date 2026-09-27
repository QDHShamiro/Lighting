const GROUP = "Lighting";
const OURS = /^Lighting( #\d+)?$/;
const colors = {};
let target = null;
const alias = { seq: 0, fwd: {}, back: {} };
let mru = [];
let epoch = 0;

export async function restore() {
  try {
    const s = await chrome.storage.session.get(["target", "alias", "mru", "epoch"]);
    if (typeof s.target === "number") target = s.target;
    if (s.alias && s.alias.fwd) Object.assign(alias, s.alias);
    if (Array.isArray(s.mru)) mru = s.mru;
    epoch = s.epoch || Date.now();
    if (!s.epoch) await chrome.storage.session.set({ epoch });
  } catch (e) {}
}

export function getEpoch() {
  return epoch;
}

export function sid(realId) {
  if (realId === null || realId === undefined) return "?";
  let n = alias.fwd[realId];
  if (!n) {
    n = ++alias.seq;
    alias.fwd[realId] = n;
    alias.back[n] = realId;
    chrome.storage.session.set({ alias }).catch(() => {});
  }
  return n;
}

export function rid(text) {
  const n = Number(String(text).replace(/^t/, ""));
  return alias.back[n] !== undefined ? alias.back[n] : null;
}

export function getTarget() {
  return target;
}

export function setTarget(id) {
  target = id;
  if (id !== null) mru = [id, ...mru.filter((x) => x !== id)].slice(0, 20);
  chrome.storage.session.set({ target: id, mru }).catch(() => {});
}

export async function previous(excluding) {
  for (const id of mru) {
    if (id !== excluding && (await inGroup(id))) return id;
  }
  const tabs = (await groupTabs()).filter((t) => t.id !== excluding);
  tabs.sort((a, b) => (b.lastAccessed || 0) - (a.lastAccessed || 0));
  return tabs.length ? tabs[0].id : null;
}

export function isOurs(title) {
  return OURS.test(title || "");
}

async function lightingGroup(windowId, label) {
  const found = await chrome.tabGroups.query({ title: label, windowId });
  return found.length ? found[0].id : null;
}

export async function addToGroup(tabId, label) {
  label = label || GROUP;
  try {
    const tab = await chrome.tabs.get(tabId);
    const gid = await lightingGroup(tab.windowId, label);
    if (gid !== null) {
      await chrome.tabs.group({ tabIds: [tabId], groupId: gid });
    } else {
      const ng = await chrome.tabs.group({ tabIds: [tabId], createProperties: { windowId: tab.windowId } });
      await chrome.tabGroups.update(ng, { title: label, color: colors[label] || "orange", collapsed: false });
    }
  } catch (e) {}
}

export async function paint(c, label) {
  if (label && colors[label] === c) return;
  if (label) colors[label] = c;
  else for (const k of Object.keys(colors)) colors[k] = c;
  const groups = await chrome.tabGroups.query({}).catch(() => []);
  for (const g of groups) {
    if ((label ? g.title === label : isOurs(g.title)) && g.color !== c) await chrome.tabGroups.update(g.id, { color: c }).catch(() => {});
  }
}

export async function groupOf(tabId) {
  try {
    const t = await chrome.tabs.get(tabId);
    if (t.groupId === -1) return null;
    const g = await chrome.tabGroups.get(t.groupId);
    return isOurs(g.title) ? g.title : null;
  } catch (e) {
    return null;
  }
}

export async function inGroup(tabId) {
  return (await groupOf(tabId)) !== null;
}

let ownWindow = true;
let wins = null;

export function setOwnWindow(on) {
  ownWindow = on;
}

async function loadWins() {
  if (wins === null) {
    const s = await chrome.storage.session.get(["wins"]).catch(() => ({}));
    wins = s.wins || {};
  }
  return wins;
}

const alive = (id) => chrome.windows.get(id).then((w) => w.type === "normal", () => false);

export async function ownWindows() {
  const out = new Set();
  for (const id of Object.values(await loadWins())) if (await alive(id)) out.add(id);
  return out;
}

async function tabInOwnWindow(label) {
  const w = await loadWins();
  if (w[label] !== undefined && (await alive(w[label]))) return chrome.tabs.create({ windowId: w[label], url: "about:blank", active: true });
  const nw = await chrome.windows.create({ url: "about:blank", focused: false });
  w[label] = nw.id;
  chrome.storage.session.set({ wins: w }).catch(() => {});
  return nw.tabs && nw.tabs[0] ? nw.tabs[0] : (await chrome.tabs.query({ windowId: nw.id }))[0];
}

export async function create(label) {
  label = label || GROUP;
  let tab;
  if (ownWindow) tab = await tabInOwnWindow(label);
  else {
    let win = null;
    try {
      win = await chrome.windows.getLastFocused({ windowTypes: ["normal"] });
    } catch (e) {}
    const props = { url: "about:blank", active: false };
    if (win) props.windowId = win.id;
    tab = await chrome.tabs.create(props);
  }
  await addToGroup(tab.id, label);
  await chrome.tabs.update(tab.id, { autoDiscardable: false }).catch(() => {});
  setTarget(tab.id);
  return tab.id;
}

export async function resolve(requested) {
  if (typeof requested === "string" && /^t\d+$/.test(requested)) {
    const id = rid(requested);
    return id === null ? null : chrome.tabs.get(id).then(() => id, () => null);
  }
  if (typeof requested === "number") {
    return chrome.tabs.get(requested).then(() => requested, () => null);
  }
  if (target !== null && (await chrome.tabs.get(target).then(() => true, () => false))) return target;
  setTarget(await previous(target));
  return target;
}

export async function groupTabs(label) {
  const groups = (await chrome.tabGroups.query({})).filter((g) => (label ? g.title === label : isOurs(g.title)));
  const out = [];
  for (const g of groups) out.push(...(await chrome.tabs.query({ groupId: g.id })));
  return out;
}

export function watchNav(tabId) {
  const st = { started: false, committed: false, dcl: false, done: false, error: null, spa: false };
  const mine = (d) => d.tabId === tabId && d.frameId === 0;
  const on = {
    onBeforeNavigate: (d) => { if (mine(d)) st.started = true; },
    onCommitted: (d) => { if (mine(d)) { st.started = true; st.committed = true; } },
    onDOMContentLoaded: (d) => { if (mine(d)) st.dcl = true; },
    onCompleted: (d) => { if (mine(d)) { st.dcl = true; st.done = true; } },
    onErrorOccurred: (d) => { if (mine(d) && d.error !== "net::ERR_ABORTED") { st.error = d.error; st.dcl = true; st.done = true; } },
    onHistoryStateUpdated: (d) => { if (mine(d)) st.spa = true; },
    onReferenceFragmentUpdated: (d) => { if (mine(d)) st.spa = true; },
  };
  for (const k in on) chrome.webNavigation[k].addListener(on[k]);
  st.stop = () => {
    for (const k in on) chrome.webNavigation[k].removeListener(on[k]);
  };
  st.wait = async (startBudget, loadBudget) => {
    const t0 = Date.now();
    while (!st.started && !st.dcl && !st.spa && Date.now() - t0 < startBudget) await sleep(15);
    if (st.started && !st.dcl) {
      const t1 = Date.now();
      while (!st.dcl && Date.now() - t1 < loadBudget) await sleep(20);
    }
    st.stop();
    return st;
  };
  return st;
}

export async function navigate(tabId, url, loadBudget) {
  const nav = watchNav(tabId);
  await chrome.tabs.update(tabId, { url });
  await nav.wait(3000, loadBudget);
  return nav;
}

export function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

export function short(url, max) {
  try {
    const u = new URL(url);
    if (u.protocol === "about:" || u.protocol === "data:") return url.slice(0, max || 60);
    for (const k of [...u.searchParams.keys()]) {
      if (/^(utm_|fbclid|gclid|mc_|ref_|igshid|si$)/i.test(k)) u.searchParams.delete(k);
    }
    const s = u.host.replace(/^www\./, "") + u.pathname.replace(/\/$/, "") + u.search + u.hash;
    const lim = max || 70;
    return s.length > lim ? s.slice(0, lim - 3) + "..." : s;
  } catch (e) {
    return String(url).slice(0, max || 70);
  }
}

export async function list() {
  const tabs = await chrome.tabs.query({});
  const lines = [];
  for (const t of tabs.slice(0, 60)) {
    const g = t.groupId !== -1 ? await groupOf(t.id) : null;
    const flags = (g ? "L" + g.replace(/^Lighting( #)?/, "") : "") + (t.id === target ? "*" : "") + (t.active ? "a" : "");
    const title = (t.title || "").replace(/\s+/g, " ").slice(0, 50);
    lines.push("t" + sid(t.id) + (flags ? " " + flags : "") + " " + title + " - " + short(t.url || t.pendingUrl || "", 60));
  }
  if (tabs.length > 60) lines.push("... +" + (tabs.length - 60) + " more tabs");
  return lines.join("\n");
}

async function painted(tabId) {
  try {
    await chrome.scripting.executeScript({ target: { tabId }, func: () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))) });
  } catch (e) {
    await sleep(120);
  }
}

export async function showBriefly(tabId, fn) {
  const tab = await chrome.tabs.get(tabId);
  if (tab.active) return fn();
  const [prev] = await chrome.tabs.query({ active: true, windowId: tab.windowId });
  await chrome.tabs.update(tabId, { active: true });
  await Promise.race([sleep(300), painted(tabId)]);
  try {
    return await fn();
  } finally {
    if (prev) await chrome.tabs.update(prev.id, { active: true }).catch(() => {});
  }
}
