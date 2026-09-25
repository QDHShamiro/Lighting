const GROUP = "Lighting";
let target = null;
let groupId = null;
const alias = { seq: 0, fwd: {}, back: {} };

export async function restore() {
  try {
    const s = await chrome.storage.session.get(["target", "alias"]);
    if (typeof s.target === "number") target = s.target;
    if (s.alias && s.alias.fwd) Object.assign(alias, s.alias);
  } catch (e) {}
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
  return alias.back[n] !== undefined ? alias.back[n] : n;
}

export function getTarget() {
  return target;
}

export function setTarget(id) {
  target = id;
  chrome.storage.session.set({ target: id }).catch(() => {});
}

async function lightingGroup(windowId) {
  if (groupId !== null) {
    try {
      const g = await chrome.tabGroups.get(groupId);
      if (g.windowId === windowId) return groupId;
    } catch (e) {}
  }
  const found = await chrome.tabGroups.query({ title: GROUP, windowId });
  groupId = found.length ? found[0].id : null;
  return groupId;
}

export async function addToGroup(tabId) {
  try {
    const tab = await chrome.tabs.get(tabId);
    const gid = await lightingGroup(tab.windowId);
    if (gid !== null) {
      await chrome.tabs.group({ tabIds: [tabId], groupId: gid });
    } else {
      groupId = await chrome.tabs.group({ tabIds: [tabId], createProperties: { windowId: tab.windowId } });
      await chrome.tabGroups.update(groupId, { title: GROUP, color: "orange", collapsed: false });
    }
  } catch (e) {}
}

export async function inGroup(tabId) {
  try {
    const t = await chrome.tabs.get(tabId);
    if (t.groupId === -1) return false;
    const g = await chrome.tabGroups.get(t.groupId);
    return g.title === GROUP;
  } catch (e) {
    return false;
  }
}

export async function create() {
  let win = null;
  try {
    win = await chrome.windows.getLastFocused({ windowTypes: ["normal"] });
  } catch (e) {}
  const props = { url: "about:blank", active: false };
  if (win) props.windowId = win.id;
  const tab = await chrome.tabs.create(props);
  await addToGroup(tab.id);
  await chrome.tabs.update(tab.id, { autoDiscardable: false }).catch(() => {});
  setTarget(tab.id);
  return tab.id;
}

export async function resolve(requested) {
  const id = typeof requested === "number" ? requested : target;
  if (id === null || id === undefined) return null;
  try {
    await chrome.tabs.get(id);
    return id;
  } catch (e) {
    if (id === target) {
      const tabs = await groupTabs();
      if (tabs.length) {
        setTarget(tabs[0].id);
        return tabs[0].id;
      }
      setTarget(null);
    }
    return null;
  }
}

export async function groupTabs() {
  const groups = await chrome.tabGroups.query({ title: GROUP });
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
    const lit = t.groupId !== -1 && (await inGroup(t.id));
    const flags = (lit ? "L" : "") + (t.id === target ? "*" : "") + (t.active ? "a" : "");
    const title = (t.title || "").replace(/\s+/g, " ").slice(0, 50);
    lines.push("t" + sid(t.id) + (flags ? " " + flags : "") + " " + title + " - " + short(t.url || t.pendingUrl || "", 60));
  }
  if (tabs.length > 60) lines.push("... +" + (tabs.length - 60) + " more tabs");
  return lines.join("\n");
}

export async function showBriefly(tabId, fn) {
  const tab = await chrome.tabs.get(tabId);
  if (tab.active) return fn();
  const [prev] = await chrome.tabs.query({ active: true, windowId: tab.windowId });
  await chrome.tabs.update(tabId, { active: true });
  await sleep(120);
  try {
    return await fn();
  } finally {
    if (prev) await chrome.tabs.update(prev.id, { active: true }).catch(() => {});
  }
}
