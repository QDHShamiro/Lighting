import datetime
import json

from lighting import defaults as D

GROUPS = (("look at pages and apps", {"open", "snap", "back", "forward", "reload", "tab", "search", "shot", "frames",
                                      "inbox", "unread"}),
          ("clicks and typing", {"click", "type", "press", "fill", "select", "check", "scroll", "hover", "drag",
                                 "dismiss", "focus", "upload", "launch", "wait", "expect", "reply", "dialog"}),
          ("sequences and routines", {"do", "run"}),
          ("page text and data", {"text", "read", "table", "fetch", "js", "net", "console", "listen", "watch", "clip"}),
          ("memory and code", {"recall", "remember", "forget", "todo", "wiki", "memory", "code", "hook", "grep",
                               "squeeze"}))
CLAIMED = {"look at pages and apps", "clicks and typing", "sequences and routines"}
SKIP = {"selftest", "bench", "demo", "ext-reload", "done", "ping", "status", "log", "tokens", "suggest", "version",
        "help", "doctor-info", "access-prompt", "stop", "autoload"}


def group_of(cmd):
    return next((name for name, cmds in GROUPS if cmd in cmds), "other")


def without(cmd):
    if cmd == "open":
        return D.TOKENS_PAGE
    if cmd == "frames":
        return D.TOKENS_VIDEO
    return D.TOKENS_SCREENSHOT if group_of(cmd) in CLAIMED else None


def short(n):
    for size, unit in ((1_000_000_000, "B"), (1_000_000, "M"), (1_000, "k")):
        if n >= size:
            return ("%.0f%s" if n >= 10 * size else "%.1f%s") % (n / size, unit)
    return str(n)


def read_rows(since):
    out = []
    for path in (D.LOG.with_suffix(".old.jsonl"), D.LOG):
        try:
            text = path.read_text("utf-8", errors="replace")
        except OSError:
            continue
        for line in text.splitlines():
            if line[7:17] < since:
                continue
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def summarize(rows):
    s = {"groups": {name: [0, 0, 0] for name in [g for g, _ in GROUPS] + ["other"]}, "cmds": {}, "days": {},
         "trim": 0, "trim_from": None, "runs": 0}
    for r in rows:
        cmd = r.get("cmd") or ""
        if r.get("x") or cmd in SKIP or not r.get("t"):
            continue
        ok = r.get("ok", True)
        used = (r.get("n") or 0) // 4 + (D.TOKENS_IMAGE.get(cmd, 0) if ok else 0)
        base = without(cmd) if ok else None
        alt = max(base, used) if base is not None else used
        g = s["groups"][group_of(cmd)]
        g[0], g[1], g[2] = g[0] + 1, g[1] + used, g[2] + alt
        s["cmds"][cmd] = s["cmds"].get(cmd, 0) + alt - used
        day = s["days"].setdefault(r["t"][:10], [0, 0])
        day[0], day[1] = day[0] + used, day[1] + alt - used
        s["runs"] += 1
        if r.get("c"):
            s["trim"] += r["c"] // 4
            s["trim_from"] = s["trim_from"] or r["t"][:10]
    return s


def buckets(days, daily):
    out = {}
    for day, (used, saved) in days.items():
        d = datetime.date.fromisoformat(day)
        key = d if daily else d - datetime.timedelta(days=d.weekday())
        b = out.setdefault(key, [0, 0])
        b[0], b[1] = b[0] + used, b[1] + saved
    keys = sorted(out)[-(14 if daily else D.TOKENS_WEEKS):]
    return [(k, out[k][0], out[k][1]) for k in keys]


def extras():
    lines = []
    try:
        from lighting import routines
        rs = [r.get("stats") or {} for r in routines.load_all()]
        runs, saved = sum(x.get("runs", 0) for x in rs), sum(x.get("saved", 0) for x in rs)
        if runs:
            lines.append("routines: %d runs, ~%s tokens saved against doing their steps by hand (all time)"
                         % (runs, short(saved)))
    except Exception:
        pass
    try:
        from lighting import brain
        st = brain.data()["stats"]
        if st.get("pruned"):
            lines.append("brain filter: hid ~%s tokens of controls you never use, on %d snaps (all time)"
                         % (short(st.get("chars", 0) // 4), st["pruned"]))
    except Exception:
        pass
    return lines


def report(s, title, daily):
    if not s["runs"]:
        return "%s: no Lighting commands logged" % title
    head = "%-24s %6s %8s %17s %8s" % ("", "runs", "used", "without Lighting", "saved")
    out = ["%s: %s commands" % (title, "{:,}".format(s["runs"])), "", head]
    total = [0, 0, 0]
    for name, (runs, used, alt) in s["groups"].items():
        if not runs:
            continue
        total = [total[0] + runs, total[1] + used, total[2] + alt]
        claim = name in CLAIMED and alt > used
        out.append("%-24s %6s %8s %17s %8s%s" % (name, "{:,}".format(runs), short(used), short(alt) if claim else "-",
                                                 short(alt - used) if claim else "-",
                                                 "  %d%%" % (100 * (alt - used) // alt) if claim else ""))
    saved = total[2] - total[1]
    out.append("%-24s %6s %8s %17s %8s  %d%%" % ("total", "{:,}".format(total[0]), short(total[1]), short(total[2]),
                                                 short(saved), 100 * saved // max(1, total[2])))
    rows = buckets(s["days"], daily)
    top = max([b[2] for b in rows] + [1])
    out += ["", "%-10s %8s %8s" % ("day" if daily else "week", "used", "saved")]
    out += ["%-10s %8s %8s  %s" % (k.strftime("%m-%d"), short(u), short(v), "█" * max(1 if v else 0, round(D.TOKENS_BAR * v / top)))
            for k, u, v in rows]
    best = sorted(((v, c) for c, v in s["cmds"].items() if v > 0), reverse=True)[:5]
    if best:
        out += ["", "most saved: " + ", ".join("%s %s" % (c, short(v)) for v, c in best)]
    out += extras()
    if s["trim"]:
        out.append("exact: Lighting cut ~%s tokens from its own answers since %s (long answers to a file, app "
                   "deltas, brain filter, code outlines, squeeze, grep)" % (short(s["trim"]), s["trim_from"][5:]))
    out += ["", "used = what your agent read from Lighting (characters / 4, images as measured).",
            "without Lighting = the same steps by a screenshot agent, measured (README, The numbers): ~%s tokens"
            % "{:,}".format(D.TOKENS_PAGE),
            "to open a page (screenshot + page text), ~%s per other look or action (one screenshot), ~%s per"
            % ("{:,}".format(D.TOKENS_SCREENSHOT), "{:,}".format(D.TOKENS_VIDEO)),
            "video. Page text, memory and code count only as used, nothing is claimed for them."]
    return "\n".join(out)


def cmd_tokens(ctx, pos, flags):
    word = (pos[0].lower() if pos else "") or str(D.TOKENS_DAYS)
    today = datetime.date.today()
    if word == "claude":
        from lighting import claudeuse
        return claudeuse.cmd(pos[1:])
    if word == "all":
        since, title, daily = "", "Lighting tokens, all logged days", False
    elif word == "today":
        since, title, daily = today.isoformat(), "Lighting tokens, today", True
    elif word.isdigit() and int(word) > 0:
        first = today - datetime.timedelta(days=int(word) - 1)
        since = first.isoformat()
        title, daily = "Lighting tokens, last %s days (%s to %s)" % (word, first.strftime("%m-%d"), today.strftime("%m-%d")), int(word) <= 14
    else:
        from lighting.common import Fail
        raise Fail("which days?", "lighting tokens | lighting tokens 7 | lighting tokens today | lighting tokens all | lighting tokens claude")
    return report(summarize(read_rows(since)), title, daily)
