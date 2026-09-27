import json
import re
import time

from lighting import defaults as D
from lighting.common import Fail, cap, terms, tiers

BROWSER = {"ctrl+l": "address bar", "ctrl+t": "new tab", "ctrl+w": "close tab", "ctrl+f": "find on page"}
SEED = {
    "discord.exe": {"ctrl+k": "quick switcher (find a channel or DM)", "ctrl+/": "list of all shortcuts",
                    "alt+up": "channel above", "alt+down": "channel below"},
    "obsidian.exe": {"ctrl+o": "quick switcher (open a note)", "ctrl+p": "command palette", "ctrl+n": "new note",
                     "ctrl+shift+f": "search all files"},
    "explorer.exe": {"ctrl+l": "address bar", "f2": "rename", "alt+up": "parent folder", "ctrl+f": "search"},
    "spotify.exe": {"space": "play/pause", "ctrl+l": "search"},
    "brave.exe": BROWSER, "chrome.exe": BROWSER, "msedge.exe": BROWSER,
    "*": {"win+e": "Explorer", "win+d": "show desktop", "win+v": "clipboard history", "win+shift+s": "screen snip",
          "playpause": "media play/pause (any app)"},
}
ALIAS = {"control": "ctrl", "strg": "ctrl", "umschalt": "shift", "umsch": "shift", "option": "alt", "cmd": "win",
         "command": "win", "meta": "win", "escape": "esc", "return": "enter", "eingabe": "enter", "entf": "delete",
         "del": "delete", "pfeil nach oben": "up", "arrowup": "up", "arrowdown": "down", "arrowleft": "left",
         "arrowright": "right"}
HINT_RE = re.compile(r"\s*\(((?:ctrl|strg|alt|shift|umschalt|win|cmd)[ +-][^)]{1,20}|[a-z0-9] then [a-z0-9])\)\s*$", re.I)
RANK = {"learned": 3, "user": 3, "ui": 2, "seed": 1}
_shown = {}


def norm(combo):
    s = " ".join(str(combo).strip().lower().replace(" then ", " ").replace(", ", " ").split())
    if "+" not in s and "-" not in s and " " in s and all(len(p) == 1 for p in s.split()):
        return s
    parts = [p for p in re.split(r"\s*[+-]\s*|\s+", s) if p]
    return "+".join(ALIAS.get(p, p) for p in parts) if parts else s


def load():
    try:
        return json.loads(D.KEYS.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def save(data):
    D.HOME.mkdir(parents=True, exist_ok=True)
    D.KEYS.write_text(json.dumps(data, indent=1, ensure_ascii=False), "utf-8")


def put(app, combo, what, src):
    if not app or not combo or not what:
        return
    data = load()
    keys = data.setdefault(app.lower(), {})
    k = norm(combo)
    cur = keys.get(k)
    if cur and RANK.get(cur.get("src"), 0) > RANK[src]:
        return
    if cur and cur.get("what") == what[:60] and cur.get("src") == src and src != "learned":
        return
    keys[k] = {"what": what[:60], "src": src, "n": (cur or {}).get("n", 0) + (1 if src == "learned" else 0)}
    save(data)


def learn(app, combo, line):
    m = re.match(r'^[de]\d+\s+(\w+)(?:\s+"([^"]*)")?', line or "")
    if m:
        put(app, combo, ('%s "%s"' % (m.group(1), m.group(2)) if m.group(2) else m.group(1)) + " opens", "learned")


def harvest(app, pairs):
    if not app or not pairs:
        return
    data = load()
    keys = data.setdefault(app.lower(), {})
    changed = False
    for combo, what in pairs[:40]:
        k, what = norm(combo), " ".join(str(what).split())[:60]
        cur = keys.get(k) or {}
        if not k or not what or RANK.get(cur.get("src"), 0) > RANK["ui"] or cur.get("what") == what:
            continue
        keys[k] = {"what": what, "src": "ui", "n": 0}
        changed = True
    if changed:
        save(data)


def from_name(name):
    m = HINT_RE.search(name or "")
    return (m.group(1), HINT_RE.sub("", name)) if m else None


def known(app):
    app = (app or "").lower()
    app = "*" if app in ("*", "windows", "windows.exe", "desktop") else app
    out = {}
    for src_app in {app}:
        for k, what in SEED.get(src_app, {}).items():
            out[k] = {"what": what, "src": "seed", "n": 0}
    for k, v in load().get(app, {}).items():
        if RANK.get(v.get("src"), 0) >= RANK.get(out.get(k, {}).get("src"), 0):
            out[k] = v
    return out


def ranked(app):
    return sorted(known(app).items(), key=lambda kv: (-RANK.get(kv[1].get("src"), 0), -kv[1].get("n", 0), kv[0]))


def hint(app):
    app = (app or "").lower()
    if not app or time.time() - _shown.get(app, 0) < D.KEY_HINT_EVERY_S:
        return ""
    own = SEED.get(app, {})
    top = [(k, v) for k, v in ranked(app) if v.get("src") != "seed" or k in own][:3]
    if not top:
        return ""
    _shown[app] = time.time()
    return "\nkeys: %s (lighting keys)" % " | ".join("%s %s" % (k, v["what"]) for k, v in top)


def app_of(ctx):
    from lighting import win
    if ctx.kind() == "app" and ctx.target and win.alive(ctx.target[1]):
        return win.exe_of(win.pid_of(ctx.target[1])).lower()
    w = getattr(ctx, "where", None) or {}
    m = re.match(r"^[a-z]+://(?:www\.)?([^/:]+)", w.get("url") or "", re.I)
    return m.group(1).lower() if m else ""


def cmd_keys(ctx, pos, flags):
    if pos and pos[0].lower() in ("add", "rm"):
        if len(pos) < 3:
            raise Fail("keys %s needs an app and a key" % pos[0], 'lighting keys add discord.exe "ctrl+k=quick switcher"')
        app = pos[1].lower()
        app = app if "." in app else app + ".exe"
        if pos[0].lower() == "add":
            combo, _, what = " ".join(pos[2:]).partition("=")
            if not what:
                raise Fail("write it as key=what it does", 'lighting keys add %s "ctrl+k=quick switcher"' % app)
            put(app, combo, what.strip(), "user")
            return "saved %s %s for %s" % (norm(combo), what.strip(), app)
        data = load()
        k = norm(" ".join(pos[2:]))
        if not data.get(app, {}).pop(k, None):
            raise Fail("no saved key %s for %s" % (k, app), "lighting keys %s" % app)
        save(data)
        return "removed %s for %s" % (k, app)
    app = (pos[0].lower() if pos else app_of(ctx))
    if pos and "." not in app:
        app += ".exe"
    if not app:
        apps = sorted(set(load()) | set(k for k in SEED if k != "*"))
        return "known apps and sites: " + ", ".join(apps) + ", windows -> lighting keys <app>"
    app = "*" if app in ("windows", "windows.exe", "desktop") else app
    rows = ranked(app)
    if flags.get("f"):
        ws = terms(flags["f"])
        rows = [(k, v) for k, v in rows if any(tiers(ws, k, v.get("what")))]
    if not rows:
        return "no keys known for %s -> use it once (lighting press ...), or lighting keys add %s \"ctrl+k=...\"" % (app, app)
    lines = ["keys for %s (%d)" % ("windows (any app)" if app == "*" else app, len(rows))]
    for k, v in rows:
        src = v.get("src")
        note = " (learned %dx)" % v.get("n", 0) if src == "learned" else {"ui": " (from the app)", "user": " (saved)"}.get(src, "")
        lines.append("%-14s %s%s" % (k, v.get("what", ""), note))
    return cap("\n".join(lines), "keys", lines=60)
