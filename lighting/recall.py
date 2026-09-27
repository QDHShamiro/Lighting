import json
import re
import time

from lighting import defaults as D
from lighting.common import Fail

STOP = set("""
a an and are auf aus bei bin bis bitte can da dann das dass dem den der des die dir do doch du ein eine einen einem
einer er es for from für geht gib hab habe hast hat he hey hi i ich ihr im in is ist it ja kann kannst mach mache
machen mal me mein meine mich mir mit my nach nicht noch nur oder of on or please sag so soll the then to um und uns
unter vom von vor was we wie will wir with wo you zu zum zur über lighting claude okay ok jetzt schnell einfach
""".split())
TOKEN = re.compile(r"[@#]|[^\W\d_][\w-]*", re.U)


def words(text):
    out = []
    for t in TOKEN.findall((text or "").lower()):
        if t in ("@", "#") or (len(t) > 2 and t not in STOP):
            out.append(t)
    return out


def stem(w):
    return w[:5] if len(w) > 5 else w


def app_word(where):
    if not where:
        return ""
    if where.get("kind") == "app":
        return re.sub(r"\.exe$", "", (where.get("exe") or "").lower())
    m = re.match(r"^[a-z]+://(?:www\.)?([^/.]+)", where.get("url") or "", re.I)
    return m.group(1).lower() if m else ""


def routine_words(r):
    first = (r.get("steps") or [{}])[0]
    app = app_word(first.get("where"))
    if not app and first.get("args"):
        app = re.sub(r"^(?:[a-z]+://)?(?:www\.)?", "", first["args"][0].lower()).split("/")[0].split(".")[0].split(":")[0]
    ws = set(words(r.get("name", "").replace("-", " ")) + list(r.get("tags") or []))
    return {stem(w) for w in ws if w}, stem(app) if app else ""


def score(prompt_stems, r):
    ws, app = routine_words(r)
    shared = prompt_stems & ws
    app_hit = bool(app) and app in prompt_stems
    other = shared - {app}
    return app_hit, len(other)


def fits(prompt_stems, r):
    app_hit, other = score(prompt_stems, r)
    return (app_hit and other >= 1) or other >= 2


def note_prompt(prompts, text):
    ws = words(text)
    if ws:
        prompts.append((time.time(), ws))
        del prompts[:-5]


def tags_for(prompts, t0, params):
    vals = {w for v in (params or {}).values() for w in words(str(v))}
    best = None
    for t, ws in prompts:
        if t <= t0 + 5 and t0 - t < 1800:
            best = ws
    if not best:
        return []
    seen, out = set(), []
    for w in best:
        if w not in vals and w not in seen:
            seen.add(w)
            out.append(w)
    return out[:12]


def cmd_suggest(ctx, pos, flags):
    from lighting import routines
    raw = " ".join(pos)
    try:
        prompt = json.loads(raw).get("prompt") or ""
    except (ValueError, AttributeError):
        prompt = raw
    note_prompt(ctx.prompts, prompt)
    stems = {stem(w) for w in words(prompt)}
    if len(stems) < 2:
        return ""
    now = time.time()
    hits = []
    for r in routines.load_all():
        if r.get("flaky") or now - ctx.told.get(r["name"], 0) < D.SUGGEST_EVERY_S or not fits(stems, r):
            continue
        app_hit, other = score(stems, r)
        hits.append((app_hit, other, r.get("stats", {}).get("ok", 0), r))
    if not hits:
        return ""
    hits.sort(key=lambda h: (h[0], h[1], h[2]), reverse=True)
    lines = []
    for *_, r in hits[:2]:
        ctx.told[r["name"]] = now
        lines.append("Lighting routine fits this request (one call, swap the values): %s" % routines.usage(r))
    return "\n".join(lines)


def find(query):
    from lighting import routines
    stems = {stem(w) for w in words(query)}
    ranked = []
    for r in routines.load_all():
        app_hit, other = score(stems, r)
        if app_hit or other:
            ranked.append((other + (1 if app_hit else 0), r))
    ranked.sort(key=lambda x: -x[0])
    if not ranked:
        raise Fail('no routine fits "%s"' % query, "lighting routines")
    if len(ranked) > 1 and ranked[1][0] == ranked[0][0]:
        raise Fail('"%s" fits several routines: %s' % (query, ", ".join(r["name"] for _, r in ranked[:3])),
                   "use the full name")
    return ranked[0][1]
