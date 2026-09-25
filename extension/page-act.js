(() => {
  const L = globalThis.__lt;
  if (!L || L.act) return;
  const { S, get, ref, nameOf, clean, trunc, q, describe } = L;
  const REJECT = /(reject|decline|deny|refuse|necessary only|only necessary|essential only|ablehnen|nur notwendige|nur erforderliche|verweigern)/i;
  const ACCEPT = /(accept all|accept|agree|allow all|got it|okay|^ok$|akzeptieren|zustimmen|einverstanden|alle erlauben|verstanden)/i;
  const FIELD = 'input:not([type="hidden"]):not([type="button"]):not([type="submit"]):not([type="reset"]):not([type="image"]),textarea,select,[contenteditable=""],[contenteditable="true"],[role="textbox"],[role="combobox"],[role="searchbox"],[role="checkbox"],[role="radio"],[role="switch"]';

  const visible = (el) => {
    if (!el || !el.checkVisibility || !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };

  function score(name, ql) {
    const n = name.toLowerCase();
    if (!n) return 0;
    if (n === ql) return 100;
    if (n.startsWith(ql)) return 60;
    if (new RegExp("(^|\\W)" + ql.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "(\\W|$)").test(n)) return 45;
    if (n.includes(ql)) return 20;
    return 0;
  }

  function rank(cands, ql, opts) {
    const list = [];
    for (const c of cands) {
      let s = score(c.name, ql);
      if (!s && c.el.value && typeof c.el.value === "string") s = score(clean(c.el.value), ql) / 2;
      if (!s) continue;
      if (c.inView) s += 10;
      if (c.role === "button" || c.role === "link") s += 3;
      if (opts && opts.role && c.role !== opts.role) continue;
      list.push({ c, s });
    }
    return list.sort((a, b) => b.s - a.s);
  }

  function pick(list, opts) {
    const top = list[0].s;
    const same = list.filter((x) => x.s === top && x.c.name.toLowerCase() === list[0].c.name.toLowerCase());
    if (same.length > 1 && !(opts && opts.first)) {
      return { ambiguous: same.slice(0, 4).map((x) => L.line(x.c.el, x.c.role, x.c.name)) };
    }
    const b = list[0].c;
    return { ref: ref(b.el), name: b.name, role: b.role };
  }

  function find(query, opts) {
    const ql = clean(query).toLowerCase();
    if (!ql) return { none: true };
    const needle = ql.replace(/\s+/g, "");
    const quick = rank(L.candidates(null, needle, true), ql, opts);
    if (quick.length && quick[0].s >= 110) return pick(quick, opts);
    const list = rank(L.candidates(null, needle), ql, opts);
    if (list.length) return pick(list, opts);
    const tw = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
    let t;
    while ((t = tw.nextNode())) {
      if (!t.data || !t.data.toLowerCase().includes(ql)) continue;
      const host = t.parentElement;
      if (!host || !visible(host)) continue;
      const target = host.closest(L.CONTROL) || host;
      return { ref: ref(target), name: trunc(clean(target.innerText || t.data), 60), role: "text" };
    }
    return { none: true };
  }

  function focusFor(r, append) {
    const el = get(r);
    if (!el) return { gone: true };
    el.focus({ preventScroll: false });
    const t = el.tagName;
    if (!append) {
      if ((t === "INPUT" || t === "TEXTAREA") && typeof el.select === "function") {
        try {
          el.select();
        } catch (e) {}
      } else if (el.isContentEditable) {
        const range = document.createRange();
        range.selectNodeContents(el);
        const sel = getSelection();
        sel.removeAllRanges();
        sel.addRange(range);
      }
    }
    return { ok: true, focused: document.activeElement === el || el.contains(document.activeElement) };
  }

  function valueOf(r) {
    const el = get(r);
    if (!el) return { gone: true };
    const v = el.isContentEditable ? el.innerText : el.value;
    return { value: typeof v === "string" ? v : "" };
  }

  function setValue(r, text) {
    const el = get(r);
    if (!el) return { gone: true };
    if (el.isContentEditable) {
      el.innerText = text;
    } else {
      const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, "value").set;
      setter.call(el, text);
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function selectOpt(r, option) {
    const el = get(r);
    if (!el) return { gone: true };
    if (el.tagName !== "SELECT") return { notSelect: true };
    const ol = clean(option).toLowerCase();
    const opts = [...el.options];
    const hit = opts.find((o) => clean(o.text).toLowerCase() === ol) || opts.find((o) => o.value.toLowerCase() === ol) || opts.find((o) => clean(o.text).toLowerCase().includes(ol));
    if (!hit) return { missing: opts.slice(0, 12).map((o) => clean(o.text)) };
    el.value = hit.value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { chosen: clean(hit.text) };
  }

  function checked(r) {
    const el = get(r);
    if (!el) return { gone: true };
    const a = el.getAttribute("aria-checked");
    return { on: el.checked === true || a === "true" };
  }

  function fields(labels) {
    const pool = [...document.querySelectorAll(FIELD)].filter(visible);
    const used = new Set();
    return labels.map((label) => {
      const ll = clean(label).toLowerCase();
      let best = null, bs = 0;
      for (const el of pool) {
        if (used.has(el)) continue;
        let s = score(nameOf(el), ll);
        const attrs = [el.getAttribute("name"), el.id, el.getAttribute("autocomplete")].filter(Boolean).map((x) => x.toLowerCase());
        if (attrs.includes(ll)) s = Math.max(s, 80);
        if (s > bs) {
          bs = s;
          best = el;
        }
      }
      if (!best) return { label, none: true };
      used.add(best);
      const role = L.roleOf(best);
      return { label, ref: ref(best), role, tag: best.tagName, type: (best.type || "").toLowerCase() };
    });
  }

  function submitOf(r) {
    const el = get(r);
    if (!el) return { gone: true };
    const form = el.form || el.closest("form");
    if (!form) return { none: true };
    const btn = form.querySelector('button[type="submit"],input[type="submit"],button:not([type])');
    if (!btn || !visible(btn)) return { form: true, none: true };
    return { ref: ref(btn), name: nameOf(btn) };
  }

  function hiddenFree() {
    const marked = [];
    const tw = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_ELEMENT, {
      acceptNode(n) {
        const gone = n.tagName === "LT-POINTER" || n.getAttribute("aria-hidden") === "true" || !n.checkVisibility({ opacityProperty: true, visibilityProperty: true });
        if (!gone) return NodeFilter.FILTER_ACCEPT;
        if (getComputedStyle(n).display === "contents") return NodeFilter.FILTER_SKIP;
        n.setAttribute("data-lt-hidden", "");
        marked.push(n);
        return NodeFilter.FILTER_REJECT;
      },
    });
    while (tw.nextNode()) {}
    const clone = document.documentElement.cloneNode(true);
    for (const n of marked) n.removeAttribute("data-lt-hidden");
    for (const n of clone.querySelectorAll("[data-lt-hidden],script,style,noscript,template")) n.remove();
    const doc = document.implementation.createHTMLDocument(document.title);
    doc.replaceChild(doc.importNode(clone, true), doc.documentElement);
    return doc;
  }

  function text(opts) {
    opts = opts || {};
    let md = "", title = document.title, note = "";
    if (!globalThis.Defuddle && !opts.raw) note = "readable view not loaded" + (opts.why ? ": " + opts.why : "");
    if (globalThis.Defuddle && !opts.raw) {
      try {
        const res = new globalThis.Defuddle(hiddenFree(), { markdown: true, url: location.href }).parse();
        md = res.content || "";
        title = res.title || title;
        if (!md.trim()) note = "readable view was empty";
      } catch (e) {
        md = "";
        note = "readable view failed: " + String((e && e.message) || e).slice(0, 120);
      }
    }
    if (!opts.links) {
      md = md.split(/(```[\s\S]*?```|`[^`\n]*`)/).map((p, i) => (i % 2 ? p : p.replace(/!?\[([^\]]*)\]\((?:[^()]|\([^()]*\))*\)/g, "$1").replace(/\[\^[^\]]+\](?!:)/g, "").replace(/<(iframe|video|audio|embed|object|svg)\b[^>]*>(?:[\s\S]*?<\/\1>)?/gi, ""))).join("");
    }
    const main = document.querySelector('main,[role="main"],article') || document.body || document.documentElement;
    if (md.trim() && md.length < 3000) {
      const plain = main.innerText || "";
      if (plain.length > 1500 && md.length < plain.length * 0.3) md = "";
    }
    if (!md.trim()) {
      md = main.innerText || "";
      if (note) md = "(" + note + ", plain text follows)\n" + md;
    }
    md = md.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
    if (opts.filter) {
      const fl = opts.filter.toLowerCase().split("|").map((s) => s.trim()).filter(Boolean);
      const parts = md.split(/\n\n+/);
      const keep = [];
      parts.forEach((p, i) => {
        const low = p.toLowerCase();
        if (fl.some((f) => low.includes(f))) {
          if (i > 0 && /^#/.test(parts[i - 1]) && !keep.includes(parts[i - 1])) keep.push(parts[i - 1]);
          keep.push(p);
        }
      });
      md = keep.join("\n\n") || "(no paragraph contains '" + opts.filter + "')";
    }
    return { title, url: location.href, text: md };
  }

  function table(target) {
    let el = null;
    if (target) el = /^e\d+$/.test(target) ? get(target) : document.querySelector(target);
    if (el && el.tagName !== "TABLE" && el.getAttribute("role") !== "grid" && el.getAttribute("role") !== "table") {
      el = el.closest('table,[role="grid"],[role="table"]') || el.querySelector('table,[role="grid"],[role="table"]');
    }
    if (!el) el = [...document.querySelectorAll('table,[role="grid"],[role="table"]')].find(visible);
    if (!el) return { none: true };
    const rows = el.tagName === "TABLE" ? [...el.rows] : [...el.querySelectorAll('[role="row"]')];
    const out = rows.map((r) => {
      const cells = el.tagName === "TABLE" ? [...r.cells] : [...r.querySelectorAll('[role="cell"],[role="gridcell"],[role="columnheader"],[role="rowheader"]')];
      return cells.map((c) => clean(c.innerText).replace(/\t/g, " ")).join("\t");
    }).filter((r) => r.trim());
    return { ref: ref(el), rows: out };
  }

  function test(spec) {
    if (spec.ref) {
      const el = get(spec.ref);
      return !!el && visible(el);
    }
    if (spec.css) {
      const el = document.querySelector(spec.css);
      return !!el && visible(el);
    }
    if (spec.text) return (document.body ? document.body.innerText : "").toLowerCase().includes(spec.text.toLowerCase());
    return false;
  }

  function waitFor(spec, timeout) {
    return new Promise((resolve) => {
      const t0 = Date.now();
      const tick = () => {
        const hit = test(spec);
        if (spec.gone ? !hit : hit) return resolve({ ok: true, ms: Date.now() - t0 });
        if (Date.now() - t0 >= timeout) return resolve({ ok: false, ms: Date.now() - t0 });
        setTimeout(tick, 100);
      };
      tick();
    });
  }

  function settle(quiet, max, waitFirst) {
    return new Promise((resolve) => {
      const t0 = Date.now(), start = S.muts;
      let last = S.muts, changed = t0, seen = false;
      const iv = setInterval(() => {
        const now = Date.now();
        if (S.muts !== last) {
          last = S.muts;
          changed = now;
          seen = true;
        }
        if (waitFirst && !seen) {
          if (now - t0 >= waitFirst) {
            clearInterval(iv);
            resolve({ ms: now - t0, changed: false });
          }
          return;
        }
        if (now - changed >= quiet || now - t0 >= max) {
          clearInterval(iv);
          resolve({ ms: now - t0, changed: S.muts !== start });
        }
      }, 25);
    });
  }

  function marks(on) {
    const old = document.querySelector("lt-pointer[data-marks]");
    if (old) old.remove();
    if (!on) return { n: 0 };
    const host = document.createElement("lt-pointer");
    host.setAttribute("data-marks", "");
    host.style.cssText = "all:initial;position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none";
    const sh = host.attachShadow({ mode: "closed" });
    let n = 0;
    for (const r of S.last) {
      const el = get(r);
      if (!el) continue;
      const b = el.getBoundingClientRect();
      const off = L.frameOffset(el.ownerDocument);
      const x = b.left + off.x, y = b.top + off.y;
      if (b.width < 2 || b.height < 2 || y + b.height <= 0 || x + b.width <= 0 || y >= innerHeight || x >= innerWidth) continue;
      const tag = document.createElement("span");
      tag.textContent = r.replace(/^e/, "");
      tag.style.cssText = "position:fixed;left:" + Math.max(0, Math.round(x) - 2) + "px;top:" + Math.max(0, Math.round(y) - 2) + "px;font:bold 11px/13px Arial,sans-serif;color:#fff;background:#ff8a00;border:1px solid #fff;border-radius:3px;padding:0 3px;box-shadow:0 1px 2px rgba(0,0,0,.6)";
      sh.appendChild(tag);
      n++;
    }
    document.documentElement.appendChild(host);
    return { n };
  }

  function pointer(x, y) {
    let host = document.querySelector("lt-pointer:not([data-marks])");
    if (!host) {
      host = document.createElement("lt-pointer");
      const sh = host.attachShadow({ mode: "closed" });
      const ns = "http://www.w3.org/2000/svg";
      const svg = document.createElementNS(ns, "svg");
      svg.setAttribute("width", "22");
      svg.setAttribute("height", "28");
      svg.setAttribute("viewBox", "0 0 22 28");
      const path = document.createElementNS(ns, "path");
      path.setAttribute("d", "M1 1 L1 22 L7 16 L11 26 L15 24 L11 15 L19 15 Z");
      path.setAttribute("fill", "#ff8a00");
      path.setAttribute("stroke", "#ffffff");
      path.setAttribute("stroke-width", "1.5");
      svg.appendChild(path);
      svg.style.cssText = "display:block;filter:drop-shadow(0 1px 2px rgba(0,0,0,.55))";
      sh.appendChild(svg);
      host.style.cssText = "all:initial;position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;transition:transform .16s ease-out,opacity .35s;opacity:0";
      document.documentElement.appendChild(host);
    }
    host.style.transform = "translate(" + (x - 1) + "px," + (y - 1) + "px)";
    host.style.opacity = "1";
    clearTimeout(host.__t);
    host.__t = setTimeout(() => {
      host.style.opacity = "0";
    }, 1500);
    return { ok: true };
  }

  function buttonsIn(root) {
    return [...root.querySelectorAll('button,a,[role="button"],input[type="button"],input[type="submit"]')];
  }

  function dismiss() {
    const sel = '#onetrust-banner-sdk,#didomi-host,#usercentrics-root,#CybotCookiebotDialog,.cc-window,[id*="cookie" i],[class*="cookie" i],[id*="consent" i],[class*="consent" i],[id*="gdpr" i],[class*="gdpr" i],[aria-label*="cookie" i],[aria-label*="consent" i]';
    const banners = [...document.querySelectorAll(sel)].filter((b) => visible(b) || (b.shadowRoot && b.shadowRoot.childElementCount));
    for (const re of [REJECT, ACCEPT]) {
      for (const b of banners) {
        const pool = buttonsIn(b).concat(b.shadowRoot ? buttonsIn(b.shadowRoot) : []);
        for (const btn of pool) {
          const n = nameOf(btn);
          if (re.test(n) && visible(btn)) {
            btn.click();
            return { clicked: trunc(n, 40), kind: re === REJECT ? "rejected" : "accepted" };
          }
        }
      }
    }
    return { none: true };
  }

  function mark(r, id) {
    const el = get(r);
    if (!el) return { gone: true };
    el.setAttribute("data-lt-mark", id);
    return { ok: true, tag: el.tagName, type: (el.type || "").toLowerCase() };
  }

  function unmark() {
    for (const el of document.querySelectorAll("[data-lt-mark]")) el.removeAttribute("data-lt-mark");
    return { ok: true };
  }

  function scrollInfo() {
    return { y: Math.round(scrollY), h: document.documentElement.scrollHeight, vh: innerHeight, vw: innerWidth };
  }

  function roomIn(el, dy) {
    if (el.scrollHeight <= el.clientHeight + 1) return 0;
    return dy < 0 ? el.scrollTop : el.scrollHeight - el.clientHeight - el.scrollTop;
  }

  function scroller(x, y, dy) {
    let el = document.elementFromPoint(x, y);
    while (el && el !== document.body && el !== document.documentElement) {
      if (/(auto|scroll|overlay)/.test(getComputedStyle(el).overflowY) && roomIn(el, dy) > 1) return el;
      el = el.parentElement || (el.getRootNode() && el.getRootNode().host) || null;
    }
    return document.scrollingElement || document.documentElement;
  }

  function scrollBy(x, y, dy, to) {
    const el = scroller(x, y, to === "top" ? -1 : to === "bottom" ? 1 : dy);
    const before = el.scrollTop;
    if (to === "top") el.scrollTo({ top: 0, behavior: "instant" });
    else if (to === "bottom") el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
    else el.scrollBy({ top: dy, behavior: "instant" });
    const max = el.scrollHeight - el.clientHeight;
    return {
      moved: Math.abs(el.scrollTop - before) >= 1,
      scrollable: max > 1,
      pct: max > 1 ? Math.round((el.scrollTop / max) * 100) : 0,
    };
  }

  function visibleText(t) {
    const tl = t.toLowerCase();
    const tw = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
    let n;
    while ((n = tw.nextNode())) {
      if (!n.data.toLowerCase().includes(tl)) continue;
      const p = n.parentElement;
      if (!p || !visible(p)) continue;
      const r = p.getBoundingClientRect();
      if (r.bottom > 0 && r.top < innerHeight) return { ok: true, ref: ref(p) };
    }
    return { ok: false };
  }

  function lineOf(r) {
    const el = get(r);
    return el ? L.line(el, L.roleOf(el), nameOf(el)) : null;
  }

  function info(r) {
    const el = get(r);
    if (!el) return { gone: true };
    return { name: nameOf(el), role: L.roleOf(el), tag: el.tagName, desc: describe(el) };
  }

  L.act = { find, focusFor, valueOf, setValue, selectOpt, checked, fields, submitOf, text, table, waitFor, settle, pointer, marks, dismiss, mark, unmark, scrollInfo, scrollBy, visibleText, lineOf, info };
})();
