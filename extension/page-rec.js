(() => {
  const L = globalThis.__lt;
  if (!L || L.rec) return;
  const { nameOf, clean, CONTROL } = L;
  const pending = new Map();
  const last = new WeakMap();
  let active = true;
  let scroll = null;

  const isText = (el) =>
    !!el && el.nodeType === 1 &&
    ((el.tagName === "INPUT" && /^(text|email|search|url|tel|number|password|date|time|datetime-local|month|week)$/.test(el.type)) ||
      el.tagName === "TEXTAREA" || el.isContentEditable);
  const isToggle = (el) => !!el && el.tagName === "INPUT" && /^(checkbox|radio)$/.test(el.type);
  const cut = (s, n) => clean(s).slice(0, n);

  function send(cmd, args, flags) {
    if (!active) return;
    try {
      chrome.runtime.sendMessage({ lt: "rec", step: { cmd, args, flags: flags || {}, url: location.href } });
    } catch (e) {}
  }

  function fieldLabel(el) {
    return cut(nameOf(el), 60) || el.getAttribute("name") || el.id || el.getAttribute("autocomplete") || "";
  }

  function valueOf(el) {
    return el.isContentEditable ? el.innerText : el.value;
  }

  function flush(el) {
    if (!pending.has(el)) return;
    pending.delete(el);
    const v = valueOf(el);
    const secret = el.type === "password";
    if (last.get(el) === v) return;
    last.set(el, v);
    const label = fieldLabel(el);
    if (label) send("fill", [label + "=" + (secret ? "@secret" : v)], secret ? { secret: true } : {});
  }

  function flushAll() {
    for (const el of [...pending.keys()]) flush(el);
    endScroll();
  }

  function endScroll() {
    if (!scroll) return;
    clearTimeout(scroll.timer);
    const d = Math.round(scroll.el.scrollTop - scroll.start);
    scroll = null;
    if (Math.abs(d) >= 80) send("scroll", [d > 0 ? "down" : "up"], { delta: String(Math.abs(d)) });
  }

  function targetOf(node) {
    const el = node && node.nodeType === 1 ? node : node && node.parentElement;
    if (!el) return null;
    const c = el.closest(CONTROL);
    if (c) return c;
    for (let p = el; p && p !== document.body; p = p.parentElement) {
      if (p.hasAttribute("onclick") || p.matches('[tabindex]:not([tabindex="-1"])')) return p;
    }
    let top = null;
    for (let p = el; p && p !== document.body; p = p.parentElement) {
      if (getComputedStyle(p).cursor === "pointer") top = p;
      else if (top) break;
    }
    return top || el;
  }

  function clickLabel(el) {
    const n = cut(nameOf(el), 80);
    if (n) return n;
    const h = L.hintOf(el);
    if (h) return "#" + h;
    return cut(el.innerText || el.textContent || "", 60);
  }

  function keyName(e) {
    const k = e.key;
    if (!k || ["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock", "Dead", "Unidentified"].includes(k)) return null;
    const mods = [];
    if (e.ctrlKey && !e.altKey) mods.push("ctrl");
    if (e.altKey && !e.ctrlKey) mods.push("alt");
    if (e.metaKey) mods.push("meta");
    if (e.shiftKey && k.length > 1) mods.push("shift");
    return mods.concat(k === " " ? "Space" : k.length === 1 ? k.toLowerCase() : k).join("+");
  }

  const on = (type, fn) => document.addEventListener(type, (e) => {
    if (active && e.isTrusted) fn(e);
  }, true);

  on("click", (e) => {
    endScroll();
    const path = e.composedPath ? e.composedPath() : [];
    const el = targetOf(path[0] || e.target);
    if (!el || el.tagName === "LT-POINTER") return;
    if (isText(el) || isToggle(el) || el.tagName === "SELECT" || el.tagName === "OPTION") return;
    const lab = el.closest("label");
    if (lab && lab.control && (isToggle(lab.control) || isText(lab.control))) return;
    const label = clickLabel(el);
    if (!label) return;
    const flags = {};
    if (e.button === 2) flags.right = true;
    if (e.detail === 2) flags.double = true;
    send("click", [label], flags);
  });

  on("input", (e) => {
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    if (isText(el)) pending.set(el, true);
  });

  on("change", (e) => {
    endScroll();
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    if (isText(el)) return flush(el);
    const label = fieldLabel(el);
    if (!label) return;
    if (el.tagName === "SELECT") {
      const o = el.selectedOptions && el.selectedOptions[0];
      if (o) send("fill", [label + "=" + clean(o.text)]);
    } else if (isToggle(el)) {
      if (el.type === "radio" && !el.checked) return;
      send("fill", [label + "=" + (el.checked ? "on" : "off")]);
    }
  });

  on("focusout", (e) => {
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    if (isText(el)) flush(el);
  });

  on("keydown", (e) => {
    if (e.repeat) return;
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    const field = isText(el) ? el : null;
    if (field) {
      if (e.key === "Enter" && !e.shiftKey && field.tagName !== "TEXTAREA" && !field.isContentEditable) {
        pending.set(field, true);
        flush(field);
        send("press", ["Enter"]);
      } else if (e.key === "Escape") {
        flush(field);
        send("press", ["Escape"]);
      }
      return;
    }
    endScroll();
    const n = keyName(e);
    if (n && n !== "Tab" && n !== "shift+Tab") send("press", [n]);
  });

  document.addEventListener("scroll", (e) => {
    if (!active) return;
    const el = e.target === document ? document.scrollingElement : e.target;
    if (!el || el.nodeType !== 1) return;
    if (!scroll || scroll.el !== el) {
      endScroll();
      scroll = { el, start: el.scrollTop };
    }
    clearTimeout(scroll.timer);
    scroll.timer = setTimeout(endScroll, 600);
  }, { capture: true, passive: true });

  addEventListener("pagehide", flushAll, true);

  const pill = document.createElement("lt-pointer");
  pill.setAttribute("data-rec", "");
  pill.style.cssText = "all:initial;position:fixed;right:12px;top:12px;z-index:2147483647;pointer-events:none";
  const sh = pill.attachShadow({ mode: "closed" });
  const badge = document.createElement("span");
  badge.textContent = "● REC Lighting";
  badge.style.cssText = "font:bold 12px/16px Arial,sans-serif;color:#fff;background:#d00000;padding:3px 9px;border-radius:10px;box-shadow:0 1px 3px rgba(0,0,0,.5)";
  sh.appendChild(badge);
  document.documentElement.appendChild(pill);

  L.rec = {
    stop() {
      flushAll();
      active = false;
      pill.remove();
      return true;
    },
  };
})();
