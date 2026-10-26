import json
import os
import re
import statistics
import time
from datetime import date, datetime, timedelta
from pathlib import Path

from lighting import defaults as D
from lighting.tokens import short

EFFORT = re.compile(r'"effort":\s*"(\w+)"')
KEEP_DAYS = 90


def claude_home():
    return Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude")


def seconds(stamp):
    try:
        return datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp()
    except (AttributeError, ValueError):
        return 0.0


def visible(content):
    n = 0
    for b in content if isinstance(content, list) else []:
        if isinstance(b, dict) and b.get("type") == "text":
            n += len(b.get("text") or "")
        elif isinstance(b, dict) and b.get("type") == "tool_use":
            n += len(json.dumps(b.get("input") or {}))
    return n


def parse(path):
    reqs, index, effort = [], {}, ""
    with path.open(encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if '"type":"api-request"' in line:
                m = EFFORT.search(line)
                effort = m.group(1) if m else effort
                continue
            if '"type":"assistant"' not in line or '"usage"' not in line:
                continue
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if e.get("isSidechain"):
                continue
            m = e.get("message") or {}
            u = m.get("usage") or {}
            key = m.get("id") or e.get("uuid")
            r = index.get(key)
            if r is None:
                write = u.get("cache_creation_input_tokens") or 0
                ctx = (u.get("input_tokens") or 0) + write + (u.get("cache_read_input_tokens") or 0)
                if not ctx:
                    continue
                r = index[key] = [str(e.get("timestamp") or "")[:19], ctx, write, 0, 0, effort]
                reqs.append(r)
            r[3] = max(r[3], u.get("output_tokens") or 0)
            r[4] += visible(m.get("content"))
    return reqs


def load(since):
    try:
        cache = json.loads(D.CLAUDE_CACHE.read_text("utf-8"))
    except (OSError, ValueError):
        cache = {}
    first = time.mktime(since.timetuple())
    keep_from = time.time() - KEEP_DAYS * 86400
    fresh, sessions = {}, []
    for path in sorted((claude_home() / "projects").glob("*/*.jsonl")):
        try:
            st = path.stat()
        except OSError:
            continue
        key, sig = str(path), "%d:%d" % (st.st_size, int(st.st_mtime))
        hit = cache.get(key)
        if st.st_mtime < first:
            if hit and st.st_mtime >= keep_from:
                fresh[key] = hit
            continue
        try:
            reqs = hit["r"] if hit and hit.get("s") == sig else parse(path)
        except OSError:
            continue
        fresh[key] = {"s": sig, "r": reqs}
        reqs = [r for r in reqs if r[0][:10] >= since.isoformat()]
        if reqs:
            sessions.append(reqs)
    try:
        D.CLAUDE_CACHE.write_text(json.dumps(fresh, separators=(",", ":")), "utf-8")
    except OSError:
        pass
    return sessions


def compacted(sessions, window):
    total, count = 0, 0
    for reqs in sessions:
        offset, prev = 0, None
        for r in reqs:
            if prev is not None and r[1] < prev * 0.5:
                offset = 0
            sim = r[1] - offset
            if sim > window:
                total, count = total + sim, count + 1
                offset += sim - D.CLAUDE_SUMMARY
                sim = D.CLAUDE_SUMMARY
            total += sim
            prev = r[1]
    return total, count


def window_set():
    val = os.environ.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
    if not val:
        try:
            env = json.loads((claude_home() / "settings.json").read_text("utf-8")).get("env") or {}
            val = env.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        except (OSError, ValueError, AttributeError):
            val = None
    return int(val) if str(val or "").isdigit() else None


def summarize(sessions):
    s = {"sessions": len(sessions), "requests": 0, "ctx": 0, "write": 0, "out": 0, "think": 0, "effort": {},
         "rebuilds": 0, "breaks": 0, "rebuilt": 0, "ctxs": [], "starts": []}
    for reqs in sessions:
        s["starts"].append((reqs[0][0], min(r[1] for r in reqs), len(reqs)))
        prev = None
        for r in reqs:
            t, ctx, write, out, vis, effort = r
            think = max(0, out - vis // 4)
            s["requests"] += 1
            s["ctx"] += ctx
            s["write"] += write
            s["out"] += out
            s["think"] += think
            s["ctxs"].append(ctx)
            e = s["effort"].setdefault(effort or "?", [0, 0])
            e[0], e[1] = e[0] + 1, e[1] + think
            if prev and write > D.CLAUDE_REBUILD and ctx >= prev[1] * 0.5:
                s["rebuilds"] += 1
                s["breaks"] += seconds(t) - seconds(prev[0]) > D.CLAUDE_BREAK_S
                s["rebuilt"] += write
            prev = r
    return s


def tips(s, sessions, weighted):
    out = []
    window = window_set()
    sim, count = compacted(sessions, D.CLAUDE_COMPACT_AT)
    cut = 100 * (s["ctx"] - sim) / max(1, s["ctx"])
    if cut >= D.CLAUDE_TIP_PCT:
        done = " (you set %s; older sessions in this window ran without it)" % short(window) if window and window <= D.CLAUDE_COMPACT_AT else ""
        out.append((cut, "auto-compact at %s: ~%d%% fewer context tokens, +%d compactions. Set CLAUDE_CODE_AUTO_COMPACT_WINDOW="
                         "%d in ~/.claude/settings.json env%s" % (short(D.CLAUDE_COMPACT_AT), cut, count, D.CLAUDE_COMPACT_AT, done)))
    mx, hi = s["effort"].get("max"), s["effort"].get("high")
    if mx and hi and hi[1]:
        share, ratio = 100 * mx[0] / s["requests"], (mx[1] / mx[0]) / (hi[1] / hi[0])
        if share >= D.CLAUDE_TIP_PCT and ratio >= D.CLAUDE_MAX_RATIO:
            out.append((share, "effort: max ran %d%% of requests and thinks %.1fx as much as high. /effort high for routine work, "
                               "max for hard problems" % (share, ratio)))
    fixed = sum(c * n for _, c, n in s["starts"])
    start = fixed / max(1, s["requests"])
    if start > D.CLAUDE_START_BIG:
        latest = statistics.median([c for _, c, _ in sorted(s["starts"])[-D.CLAUDE_START_SESSIONS:]])
        part = 100 * fixed / max(1, s["ctx"])
        out.append((part, "start context ~%s on every request (%d%% of all context, last %d sessions ~%s): /context shows it; "
                          "deny tools you never use, skillOverrides \"name-only\" for rare skills, turn off unused MCP servers"
                    % (short(int(start)), part, D.CLAUDE_START_SESSIONS, short(int(latest)))))
    rebuilt = 100 * s["rebuilt"] * D.CLAUDE_WEIGHTS["write"] / max(1, weighted)
    if rebuilt >= D.CLAUDE_TIP_PCT / 2:
        out.append((rebuilt, "cache rebuilds: %d full rewrites (%d after a break over %d min), %d%% of the weighted total. "
                             "/compact before long breaks" % (s["rebuilds"], s["breaks"], D.CLAUDE_BREAK_S // 60, rebuilt)))
    return [text for _, text in sorted(out, reverse=True)]


def report(sessions, days):
    s = summarize(sessions)
    title = "Claude Code tokens, last %d days" % days
    if not s["requests"]:
        return "%s: no sessions found in %s" % (title, claude_home() / "projects")
    w = D.CLAUDE_WEIGHTS
    read = s["ctx"] - s["write"]
    weighted = read * w["read"] + s["write"] * w["write"] + s["out"] * w["output"]
    pct = lambda part: 100 * part / max(1, weighted)
    s["ctxs"].sort()
    lines = ["%s: %s requests in %d sessions (subagents not counted)" % (title, "{:,}".format(s["requests"]), s["sessions"]),
             "context read: %s tokens, ~%s per request (half of all requests above %s)" % (
                 short(s["ctx"]), short(s["ctx"] // s["requests"]), short(s["ctxs"][len(s["ctxs"]) // 2])),
             "weighted like API prices: cache reads %d%%, cache writes %d%%, output %d%%" % (
                 pct(read * w["read"]), pct(s["write"] * w["write"]), pct(s["out"] * w["output"])),
             "output %s tokens: ~%d%% thinking. Claude Code keeps thinking in the context, so later requests read it again" % (
                 short(s["out"]), 100 * s["think"] // max(1, s["out"]))]
    known = sorted(((k, v) for k, v in s["effort"].items() if k != "?"), key=lambda kv: -kv[1][0])
    if known:
        lines += ["", "%-8s %9s %22s" % ("effort", "requests", "thinking per request")]
        lines += ["%-8s %9s %22s" % (k, "{:,}".format(n), "~" + "{:,}".format(t // n)) for k, (n, t) in known]
    found = tips(s, sessions, weighted)
    lines += [""] + (["fixes, biggest first:"] + ["- " + t for t in found] if found else ["no big fix found"])
    lines += ["", "read from your local Claude Code transcripts; weights: cache read %.1fx, cache write %.0fx (1 h cache), output %.0fx "
                  "the input price. Thinking = output minus visible text and tool calls (characters / 4)." % (w["read"], w["write"], w["output"])]
    return "\n".join(lines)


def cmd(pos):
    word = pos[0] if pos else str(D.TOKENS_DAYS)
    if not word.isdigit() or int(word) < 1:
        from lighting.common import Fail
        raise Fail("which days?", "lighting tokens claude | lighting tokens claude 7")
    days = int(word)
    return report(load(date.today() - timedelta(days=days - 1)), days)
