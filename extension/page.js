(() => {
  if (globalThis.__lt) return;
  const S = { seq: 0, refs: new Map(), ids: new WeakMap(), last: new Set(), muts: 0, lastMuts: -1, lastHref: "", lastY: -1, sig: "" };
  const OURS = "LT-POINTER";
  const INTERACTIVE =
    'a[href],button,input:not([type="hidden"]),select,textarea,summary,[role="button"],[role="link"],[role="checkbox"],[role="radio"],[role="switch"],[role="tab"],[role="menuitem"],[role="menuitemcheckbox"],[role="menuitemradio"],[role="option"],[role="combobox"],[role="textbox"],[role="searchbox"],[role="slider"],[role="spinbutton"],[role="treeitem"],[contenteditable=""],[contenteditable="true"],[tabindex]:not([tabindex="-1"]),[onclick]';
  const CONTROL =
    'a[href],button,input:not([type="hidden"]),select,textarea,summary,[role="button"],[role="link"],[role="checkbox"],[role="radio"],[role="switch"],[role="tab"],[role="menuitem"],[role="menuitemcheckbox"],[role="menuitemradio"],[role="option"],[role="combobox"],[role="textbox"],[role="searchbox"],[role="slider"],[role="spinbutton"],[role="treeitem"],[contenteditable=""],[contenteditable="true"]';
  const LANDMARK = 'nav,aside,footer,[role="navigation"],[role="complementary"],[role="contentinfo"]';
  const ITEM = 'li,tr,article,[role="listitem"],[role="row"],[role="article"]';
  const ROLE_INPUT = { checkbox: "checkbox", radio: "radio", range: "slider", number: "spinbutton", search: "searchbox", button: "button", submit: "button", reset: "button", image: "button", file: "file", color: "color", date: "date", "datetime-local": "date", time: "time", month: "date", week: "date" };
  const TEXTISH = new Set(["textbox", "searchbox", "spinbutton", "combobox", "date", "time", "color"]);
  const HIT_BUDGET_MS = 60;

  const ours = (n) => n.nodeName === OURS;
  new MutationObserver((list) => {
    for (const m of list) {
      const t = m.target;
      if (t && t.nodeType === 1 && (t.tagName === OURS || t.closest(OURS))) continue;
      if (m.type === "childList" && [...m.addedNodes].every(ours) && [...m.removedNodes].every(ours)) continue;
      S.muts++;
      return;
    }
  }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });

  const clean = (s) => String(s || "").replace(/\s+/g, " ").trim();
  const trunc = (s, n) => (s.length > n ? s.slice(0, n - 3) + "..." : s);
  const q = (s) => s.replace(/"/g, "'");

  function ref(el) {
    let r = S.ids.get(el);
    if (!r) {
      r = "e" + ++S.seq;
      S.ids.set(el, r);
      S.refs.set(r, new WeakRef(el));
    }
    return r;
  }

  function get(r) {
    const w = S.refs.get(r);
    const el = w && w.deref();
    return el && el.isConnected ? el : null;
  }

  function roleOf(el) {
    const r = el.getAttribute("role");
    if (r) return r.split(/\s+/)[0];
    const t = el.tagName;
    if (t === "A") return "link";
    if (t === "BUTTON" || t === "SUMMARY") return "button";
    if (t === "SELECT") return el.multiple ? "listbox" : "combobox";
    if (t === "TEXTAREA") return "textbox";
    if (t === "INPUT") return ROLE_INPUT[(el.type || "text").toLowerCase()] || "textbox";
    if (el.isContentEditable) return "textbox";
    if (/^H[1-6]$/.test(t)) return "heading";
    return "clickable";
  }

  function nameOf(el) {
    let s = el.getAttribute("aria-label");
    if (!s) {
      const ids = el.getAttribute("aria-labelledby");
      if (ids) {
        s = ids.split(/\s+/).map((i) => {
          const e = el.ownerDocument.getElementById(i);
          return e ? e.innerText || e.textContent : "";
        }).join(" ");
      }
    }
    const t = el.tagName;
    if (!s && el.labels && el.labels.length) s = [...el.labels].map((l) => l.innerText).join(" ");
    if (!s && (t === "INPUT" || t === "TEXTAREA" || t === "SELECT")) {
      const ty = (el.type || "").toLowerCase();
      s = el.getAttribute("placeholder") || el.getAttribute("title") || (ty === "submit" || ty === "button" || ty === "reset" ? el.value : "") || el.getAttribute("name") || "";
    }
    if (!s && t === "IMG") s = el.alt;
    if (!s && t !== "INPUT" && t !== "TEXTAREA" && t !== "SELECT") s = el.innerText;
    if (!clean(s)) {
      const inner = el.querySelector("img[alt],[aria-label],[title]");
      if (inner) s = inner.getAttribute("alt") || inner.getAttribute("aria-label") || inner.getAttribute("title");
    }
    if (!clean(s)) s = el.getAttribute("title") || "";
    if (!clean(s) && t === "A" && !el.children.length && el.parentElement) {
      const p = el.parentElement;
      const titled = p.querySelector('h1,h2,h3,h4,[class*="title" i],[class*="name" i]');
      s = titled ? titled.innerText : (p.innerText || "").trim().split("\n")[0];
    }
    return clean(s);
  }

  function short(href) {
    try {
      const u = new URL(href, location.href);
      if (u.protocol === "javascript:") return "";
      const yt = /(^|\.)youtube\.com$/.test(u.hostname);
      for (const k of [...u.searchParams.keys()]) {
        if (/^(utm_|fbclid|gclid|dclid|msclkid|yclid|twclid|ttclid|mc_|igshid|_hsenc|_hsmi|mkt_tok)/i.test(k) || (yt && /^(pp|si|feature)$/.test(k))) u.searchParams.delete(k);
      }
      const qs = u.search.length > 40 ? "?…" : u.search;
      if (u.origin === location.origin) {
        if (u.pathname === location.pathname && u.search === location.search) return u.hash || ".";
        const here = location.pathname.replace(/\/$/, "");
        const p = here && u.pathname.startsWith(here + "/") ? "." + u.pathname.slice(here.length) : u.pathname;
        return trunc(p + qs + (u.hash.length > 30 ? "" : u.hash), 60);
      }
      return trunc(u.host.replace(/^www\./, "") + u.pathname.replace(/\/$/, "") + qs, 60);
    } catch (e) {
      return "";
    }
  }

  function state(el, role) {
    let s = "";
    if (role === "checkbox" || role === "radio" || role === "switch" || role === "menuitemcheckbox" || role === "menuitemradio") {
      const a = el.getAttribute("aria-checked");
      const on = el.checked === true || a === "true";
      s += a === "mixed" ? " [-]" : on ? " [x]" : " [ ]";
    } else if (el.hasAttribute("aria-pressed")) {
      const p = el.getAttribute("aria-pressed");
      s += p === "mixed" ? " [-]" : p === "true" ? " [x]" : " [ ]";
    } else if (TEXTISH.has(role)) {
      if (el.tagName === "SELECT") {
        const o = el.selectedOptions && el.selectedOptions[0];
        if (o) s += ' ="' + q(trunc(clean(o.text), 40)) + '"';
      } else if ((el.type || "").toLowerCase() === "password") {
        if (el.value) s += " =***";
      } else if (typeof el.value === "string" && el.value) {
        s += ' ="' + q(trunc(clean(el.value), 40)) + '"';
      }
    }
    if (el.disabled || el.getAttribute("aria-disabled") === "true") s += " (disabled)";
    if (el.getAttribute("aria-selected") === "true" || (el.getAttribute("aria-current") && el.getAttribute("aria-current") !== "false")) s += " *";
    const ex = el.getAttribute("aria-expanded");
    if (ex === "true") s += " (expanded)";
    else if (ex === "false") s += " (collapsed)";
    if (role === "link" && el.href) {
      const h = short(el.href);
      if (h) s += " ->" + h;
    }
    return s;
  }

  function line(el, role, name) {
    return ref(el) + " " + role + (name ? ' "' + q(trunc(name, 60)) + '"' : "") + state(el, role);
  }

  function frameOffset(doc) {
    let x = 0, y = 0, win = doc.defaultView;
    while (win && win !== window && win.frameElement) {
      const fe = win.frameElement;
      const r = fe.getBoundingClientRect();
      x += r.left + fe.clientLeft;
      y += r.top + fe.clientTop;
      win = win.parent;
    }
    return { x, y };
  }

  function covered(el, r) {
    const root = el.getRootNode();
    const pts = [[r.left + r.width / 2, r.top + r.height / 2], [r.left + r.width * 0.25, r.top + r.height / 2], [r.left + r.width * 0.75, r.top + r.height / 2]];
    let hit = null;
    for (const [x, y] of pts) {
      hit = root.elementFromPoint ? root.elementFromPoint(x, y) : null;
      if (!hit || hit === el || el.contains(hit) || hit.tagName === OURS) return null;
      if (hit.shadowRoot && hit.shadowRoot.contains(el)) return null;
    }
    return hit;
  }

  function describe(el) {
    if (!el) return "?";
    const cls = typeof el.className === "string" ? el.className.trim().split(/\s+/).slice(0, 2).join(".") : "";
    const nm = trunc(nameOf(el), 30);
    return el.tagName.toLowerCase() + (el.id ? "#" + el.id : "") + (cls ? "." + cls : "") + (nm ? ' "' + q(nm) + '"' : "");
  }

  function pointerish(el, cs) {
    if (cs.cursor !== "pointer") return false;
    const p = el.parentElement;
    return !p || getComputedStyle(p).cursor !== "pointer";
  }

  function where(ax, ay, w, h) {
    if (ax + w <= 0 || ax >= innerWidth) return "away";
    if (ay >= innerHeight) return "below";
    if (ay + h <= 0) return scrollY > 0 && ay + scrollY + h > 0 ? "above" : "away";
    return "in";
  }

  function offscreen(n, off, cut) {
    const r = n.getBoundingClientRect();
    if (!r.width && !r.height) return false;
    const pos = where(r.left + off.x, r.top + off.y, r.width, r.height);
    if (pos === "in") return false;
    if (pos !== "away") cut[pos] += n.querySelectorAll(INTERACTIVE).length;
    return true;
  }

  function walk(root, off, out, frames, cut) {
    const doc = root.ownerDocument || root;
    const tw = doc.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {
      acceptNode(n) {
        const t = n.tagName;
        if (t === "SCRIPT" || t === "STYLE" || t === "NOSCRIPT" || t === "TEMPLATE" || t === OURS || t === "HEAD") return NodeFilter.FILTER_REJECT;
        if (!n.checkVisibility()) {
          return getComputedStyle(n).display === "contents" ? NodeFilter.FILTER_SKIP : NodeFilter.FILTER_REJECT;
        }
        if (cut && cut.prune && n.matches(ITEM) && offscreen(n, off, cut)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    let n = tw.nextNode();
    while (n) {
      const t = n.tagName;
      const cs = t === "BODY" || t === "HTML" ? null : getComputedStyle(n);
      let isInt = n.matches(INTERACTIVE);
      if (isInt && !n.matches(CONTROL)) {
        isInt = !n.querySelector(INTERACTIVE) && (n.hasAttribute("onclick") || (cs && cs.cursor === "pointer"));
      }
      if (cut && cs && (cs.position === "fixed" || cs.position === "sticky" || cs.position === "absolute")) cut.layers.push({ el: n, off });
      const head = !isInt && (/^H[1-3]$/.test(t) || n.getAttribute("role") === "heading");
      if (isInt || head || (cs && pointerish(n, cs) && !n.closest(CONTROL) && !n.querySelector(INTERACTIVE))) {
        out.push({ el: n, off, heading: head });
      }
      if (n.shadowRoot) walk(n.shadowRoot, off, out, frames, cut);
      if (t === "IFRAME" || t === "FRAME") {
        let inner = null;
        try {
          inner = n.contentDocument;
        } catch (e) {}
        if (inner && inner.documentElement) {
          const r = n.getBoundingClientRect();
          walk(inner.documentElement, { x: off.x + r.left + n.clientLeft, y: off.y + r.top + n.clientTop }, out, frames, cut);
        } else {
          frames.push(n);
        }
      }
      n = tw.nextNode();
    }
  }

  function flushAnimations() {
    if (!document.getAnimations) return;
    for (const a of document.getAnimations()) {
      try {
        if (a.playState === "running" && a.effect && a.effect.getComputedTiming().endTime !== Infinity) a.finish();
      } catch (e) {}
    }
  }

  function overlays(cut) {
    const out = [];
    for (const { el, off } of cut.layers) {
      const r = el.getBoundingClientRect();
      if (r.width < 8 || r.height < 8) continue;
      const x = r.left + off.x, y = r.top + off.y;
      if (y >= innerHeight || x >= innerWidth || y + r.height <= 0 || x + r.width <= 0) continue;
      if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
      out.push({ el, x, y, r: x + r.width, b: y + r.height });
    }
    return out;
  }

  function mayBeCovered(el, x, y, w, h, layers) {
    for (const o of layers) {
      if (o.x >= x + w || o.r <= x || o.y >= y + h || o.b <= y) continue;
      if (o.el === el || o.el.contains(el) || el.contains(o.el)) continue;
      return true;
    }
    return false;
  }

  function modalOf() {
    const list = document.querySelectorAll('dialog[open],[aria-modal="true"],[role="dialog"],[role="alertdialog"]');
    let best = null;
    for (const m of list) {
      if (!m.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
      const r = m.getBoundingClientRect();
      if (r.width < 80 || r.height < 40) continue;
      best = m;
    }
    return best;
  }

  function notes(frames) {
    const out = [];
    const cap = document.querySelector('iframe[src*="recaptcha"],iframe[src*="hcaptcha"],iframe[src*="challenges.cloudflare.com"],iframe[src*="arkoselabs"],#challenge-form,.cf-turnstile,#px-captcha');
    if (cap && cap.checkVisibility()) out.push("! captcha on page: ask the user to solve it");
    const ck = document.querySelector('#onetrust-banner-sdk,#didomi-host,#usercentrics-root,#CybotCookiebotDialog,.cc-window,[id*="cookie" i][role="dialog"],[class*="cookie-banner" i],[id*="cookie-banner" i],[class*="consent-banner" i],[id*="consent" i][role="dialog"]');
    if (ck && ck.checkVisibility({ opacityProperty: true, visibilityProperty: true })) out.push("! cookie banner: lighting dismiss");
    for (const f of frames.slice(0, 5)) {
      if (!f.checkVisibility()) continue;
      const r = f.getBoundingClientRect();
      if (r.width < 50 || r.height < 30) continue;
      let host = "";
      try {
        host = new URL(f.src, location.href).host;
      } catch (e) {}
      if (host) out.push("iframe " + host + " (cross-origin, lighting snap --frame " + host + ")");
    }
    return out;
  }

  function collectLines(opts) {
    const vw = innerWidth, vh = innerHeight;
    let scope = document.documentElement;
    const notesOut = [];
    if (opts.scope) {
      const s = /^e\d+$/.test(opts.scope) ? get(opts.scope) : document.querySelector(opts.scope);
      if (!s) return { error: "scope " + opts.scope + " not found" };
      scope = s;
    } else if (!opts.all) {
      const m = modalOf();
      if (m) {
        scope = m;
        notesOut.push('modal "' + q(trunc(nameOf(m), 50)) + '" open: page behind is hidden (press Escape to close)');
      }
    }
    flushAnimations();
    const items = [], frames = [];
    const cut = { below: 0, above: 0, layers: [], prune: !opts.all };
    walk(scope, { x: 0, y: 0 }, items, frames, cut);
    const layers = overlays(cut);
    let hitMs = 0;
    const filter = opts.filter ? opts.filter.toLowerCase().split("|").map((s) => s.trim()).filter(Boolean) : null;
    const starts = filter ? filter.map((f) => new RegExp("(^|[^\\p{L}\\p{N}])" + f.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "u")) : null;
    const matches = (s) => filter.some((f) => s.includes(f));
    const strength = (s) => (starts.some((r) => r.test(s)) ? 2 : matches(s) ? 1 : 0);
    const lines = [], seen = new Set();
    let below = 0, above = 0;
    const land = new Map();
    const groups = new Map();
    const collapseOk = !opts.all && !opts.scope && !filter;
    for (const it of items) {
      const el = it.el;
      const r = el.getBoundingClientRect();
      if (r.width <= 1 || r.height <= 1) continue;
      if (!it.heading && !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
      if (el.closest('[aria-hidden="true"]')) continue;
      const ax = r.left + it.off.x, ay = r.top + it.off.y;
      const pos = where(ax, ay, r.width, r.height);
      const inView = pos === "in";
      if (!inView && !opts.all) {
        if (!it.heading) {
          if (pos === "below") below++;
          else if (pos === "above") above++;
        }
        continue;
      }
      if (it.heading) {
        const full = clean(el.innerText);
        const txt = trunc(full, 80);
        const lvl = /^H([1-3])$/.exec(el.tagName);
        const hashes = "#".repeat(lvl ? Number(lvl[1]) : Math.min(3, Number(el.getAttribute("aria-level")) || 2));
        if (txt && !filter && !seen.has(txt)) {
          seen.add(txt);
          lines.push({ el, text: hashes + " " + txt, raw: full.toLowerCase(), heading: true });
        }
        continue;
      }
      if (inView && hitMs < HIT_BUDGET_MS && mayBeCovered(el, ax, ay, r.width, r.height, layers)) {
        const t0 = performance.now();
        const hit = covered(el, r);
        hitMs += performance.now() - t0;
        if (hit) continue;
      }
      const role = roleOf(el);
      const name = nameOf(el);
      if (role === "clickable" && !name) continue;
      const fs = filter ? Math.max(strength(name.toLowerCase()), strength(role), strength(String(el.value || "").toLowerCase())) : 0;
      if (filter && !fs) continue;
      const entry = { el, role, name, fs };
      if (collapseOk) {
        const lm = el.closest(LANDMARK);
        if (lm) {
          const box = land.get(lm) || { count: 0, entries: [] };
          box.count++;
          box.entries.push(entry);
          land.set(lm, box);
          entry.landmark = lm;
        }
        const item = el.closest(ITEM);
        if (item && item.parentElement) {
          const key = item.parentElement;
          const g = groups.get(key) || { items: new Set() };
          g.items.add(item);
          groups.set(key, g);
          entry.item = item;
          entry.list = key;
        }
      }
      lines.push(entry);
    }
    if (filter && lines.some((e) => e.fs === 2)) for (const e of lines) if (e.fs === 1) e.drop = true;
    const hrefOf = (e) => (e && !e.heading && e.role === "link" ? e.el.href || "" : null);
    for (let i = 0; i < lines.length; i++) {
      const e = lines[i], nx = lines[i + 1], pv = lines[i - 1];
      if (e.heading) {
        if (nx && !nx.heading && nx.name && (e.el.contains(nx.el) || nx.el.contains(e.el)) && nx.name.toLowerCase().startsWith(e.raw)) e.drop = true;
        continue;
      }
      const h = hrefOf(e);
      if (!h) continue;
      if (!e.name) {
        for (let j = Math.max(0, i - 3); j <= Math.min(lines.length - 1, i + 3); j++) {
          if (j !== i && hrefOf(lines[j]) === h && lines[j].name) e.drop = true;
        }
      } else if (hrefOf(pv) === h && !pv.drop && pv.name === e.name) e.drop = true;
    }
    const bigLand = new Set([...land.entries()].filter(([, b]) => b.count > 12).map(([lm]) => lm));
    const bigList = new Map();
    for (const [list, g] of groups) if (g.items.size > 8) bigList.set(list, [...g.items].slice(0, 5));
    const out = [], landDone = new Set(), listDone = new Set();
    const ids = new Set();
    for (const e of lines) {
      if (e.drop) continue;
      if (e.heading) {
        out.push(e.text);
        continue;
      }
      if (e.landmark && bigLand.has(e.landmark)) {
        if (!landDone.has(e.landmark)) {
          landDone.add(e.landmark);
          const lm = e.landmark;
          const kind = lm.tagName === "NAV" || lm.getAttribute("role") === "navigation" ? "nav" : lm.tagName === "FOOTER" || lm.getAttribute("role") === "contentinfo" ? "footer" : "aside";
          const nm = trunc(clean(lm.getAttribute("aria-label") || ""), 30);
          ids.add(ref(lm));
          out.push(ref(lm) + " " + kind + (nm ? ' "' + q(nm) + '"' : "") + ": " + land.get(lm).count + " items (lighting snap -s " + ref(lm) + ")");
        }
        continue;
      }
      if (e.list && bigList.has(e.list)) {
        const keep = bigList.get(e.list);
        if (!keep.includes(e.item)) {
          if (!listDone.has(e.list)) {
            listDone.add(e.list);
            const total = groups.get(e.list).items.size;
            ids.add(ref(e.list));
            out.push("... +" + (total - keep.length) + " similar items (lighting snap -s " + ref(e.list) + " --all)");
          }
          continue;
        }
      }
      ids.add(ref(e.el));
      out.push(line(e.el, e.role, e.name));
    }
    return { lines: out, ids, below: below + cut.below, above: above + cut.above, notes: notesOut.concat(notes(frames)) };
  }

  function snap(opts) {
    opts = opts || {};
    const sig = JSON.stringify([opts.all, opts.scope, opts.filter]);
    if (!opts.force && S.muts === S.lastMuts && location.href === S.lastHref && Math.round(scrollY) === S.lastY && sig === S.sig) {
      return { unchanged: true, title: document.title, url: location.href };
    }
    let res = collectLines(opts);
    if (res.error) return res;
    if (opts.filter && !opts.all && !res.lines.some((l) => /^e\d+ /.test(l))) {
      res = collectLines(Object.assign({}, opts, { all: true }));
      res.notes = res.notes.concat(res.lines.some((l) => /^e\d+ /.test(l)) ? "(no match in view, matches from the whole page; click scrolls to them)" : '(no match for "' + opts.filter + '" on the whole page)');
      res.searched = true;
    }
    const prev = S.last;
    S.last = res.ids;
    S.lastMuts = S.muts;
    S.lastHref = location.href;
    S.lastY = Math.round(scrollY);
    S.sig = sig;
    const out = { title: document.title, url: location.href, lines: res.lines, notes: res.notes, searched: res.searched, below: res.below, above: res.above };
    if (opts.diff) {
      out.lines = res.lines.filter((l) => /^e\d+ /.test(l) && !prev.has(l.split(" ")[0]));
      out.removed = [...prev].filter((r) => !res.ids.has(r)).length;
    }
    const sh = document.documentElement.scrollHeight;
    if (sh > innerHeight * 1.2) out.scroll = Math.round((scrollY / Math.max(1, sh - innerHeight)) * 100);
    return out;
  }

  function diff() {
    const res = collectLines({});
    const added = res.lines.filter((l) => /^e\d+ /.test(l) && !S.last.has(l.split(" ")[0]));
    const removed = [...S.last].filter((r) => !res.ids.has(r)).length;
    S.last = res.ids;
    S.lastMuts = -1;
    return { added, removed, total: res.ids.size };
  }

  function rect(r, opts) {
    const el = get(r);
    if (!el) return { gone: true };
    let b = el.getBoundingClientRect();
    const off = frameOffset(el.ownerDocument);
    const ax = b.left + off.x, ay = b.top + off.y;
    if (!opts || !opts.noScroll) {
      if (ay < 0 || ax < 0 || ay + b.height > innerHeight || ax + b.width > innerWidth) {
        el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
        b = el.getBoundingClientRect();
      }
    }
    const o2 = frameOffset(el.ownerDocument);
    const hit = covered(el, b);
    return {
      x: Math.round(b.left + o2.x + b.width / 2),
      y: Math.round(b.top + o2.y + b.height / 2),
      w: Math.round(b.width),
      h: Math.round(b.height),
      name: nameOf(el),
      role: roleOf(el),
      covered: hit ? describe(hit) : null,
      dpr: devicePixelRatio,
      href: el.tagName === "A" ? el.href : "",
      blank: el.tagName === "A" && el.target === "_blank",
      line: line(el, roleOf(el), nameOf(el)),
    };
  }

  function hay(el) {
    const text = el.textContent || "";
    let h = text;
    for (const a of ["aria-label", "title", "placeholder", "alt", "name"]) {
      const v = el.getAttribute(a);
      if (v) h += " " + v;
    }
    if (typeof el.value === "string") h += " " + el.value;
    const lb = el.getAttribute("aria-labelledby");
    if (lb) for (const id of lb.split(/\s+/)) h += " " + ((el.ownerDocument.getElementById(id) || {}).textContent || "");
    if (el.labels) for (const l of el.labels) h += " " + l.textContent;
    const inner = text.trim() ? null : el.querySelector("img[alt],[aria-label],[title]");
    if (inner) h += " " + (inner.getAttribute("alt") || "") + (inner.getAttribute("aria-label") || "") + (inner.getAttribute("title") || "");
    return h.toLowerCase().replace(/\s+/g, "");
  }

  function candidates(root, needle, viewOnly) {
    flushAnimations();
    const items = [], frames = [];
    walk(root || document.documentElement, { x: 0, y: 0 }, items, frames, viewOnly ? { below: 0, above: 0, layers: [], prune: true } : undefined);
    const vw = innerWidth, vh = innerHeight, out = [];
    for (const it of items) {
      if (it.heading) continue;
      const el = it.el;
      if (needle && !hay(el).includes(needle)) continue;
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) continue;
      if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
      if (el.closest('[aria-hidden="true"]')) continue;
      const ax = r.left + it.off.x, ay = r.top + it.off.y;
      const inView = ay + r.height > 0 && ax + r.width > 0 && ay < vh && ax < vw;
      if (viewOnly && !inView) continue;
      out.push({ el, role: roleOf(el), name: nameOf(el), inView });
    }
    return out;
  }

  globalThis.__lt = { S, get, ref, roleOf, nameOf, clean, trunc, q, describe, covered, frameOffset, snap, diff, rect, candidates, line, INTERACTIVE, CONTROL };
})();
