import fnmatch
import os
import re
import subprocess
import time
from collections import deque
from pathlib import Path

from lighting import defaults as D
from lighting.common import Fail, cap, outfile, trimmed, win_path

LETTERS = {"i": "ignore", "w": "word", "F": "fixed", "l": "files", "c": "files", "r": None, "R": None, "n": None,
           "E": None, "H": None, "I": None, "P": None, "h": None, "s": None}
CONTEXT = re.compile(r"-([ABC])(\d*)")
BARE_BAR = re.compile(r"(?<!\\)\|")
USAGE = "lighting grep <pattern> [folder|file ...] [-i] [-w] [-F] [-l] [-C 2] [--glob '*.py'] [--all]"


def number(val):
    if not str(val or "").isdigit():
        raise Fail("which number of context lines? (%s)" % val, "lighting grep -C 2 <pattern>")
    return int(val)


def options(pos, flags):
    o = {"ignore": bool(flags.get("ignore-case")), "word": bool(flags.get("word")), "fixed": bool(flags.get("fixed")),
         "files": bool(flags.get("files") or flags.get("count")), "hidden": bool(flags.get("hidden")),
         "all": bool(flags.get("all")), "before": 0, "after": 0}
    if flags.get("context") not in (None, True):
        o["before"] = o["after"] = number(flags["context"])
    rest, i, done = [], 0, False
    while i < len(pos):
        a = pos[i]
        m = CONTEXT.fullmatch(a)
        if done or len(a) < 2 or not a.startswith("-"):
            rest.append(a)
        elif a == "--":
            done = True
        elif m:
            val = m.group(2)
            if not val and i + 1 < len(pos):
                i += 1
                val = pos[i]
            n = number(val)
            o["before"] = n if m.group(1) in "BC" else o["before"]
            o["after"] = n if m.group(1) in "AC" else o["after"]
        elif re.fullmatch(r"-[A-Za-z]+", a) and all(c in LETTERS for c in a[1:]):
            for c in a[1:]:
                if LETTERS[c]:
                    o[LETTERS[c]] = True
        else:
            rest.append(a)
        i += 1
    return o, rest


def matcher(pattern, o):
    text = re.escape(pattern) if o["fixed"] else pattern
    if not o["fixed"] and "\\|" in text and not BARE_BAR.search(text):
        text = text.replace("\\|", "|")
    if o["word"]:
        text = r"\b(?:%s)\b" % text
    flags = re.I if o["ignore"] else 0
    try:
        rx = re.compile(text, flags)
    except re.error as e:
        if o["fixed"]:
            raise Fail("cannot read the pattern: %s" % e, USAGE)
        rx, pre, _ = matcher(pattern, dict(o, fixed=True))
        return rx, pre, "not a regex (%s), searched as plain text" % e
    return rx, None if "\\A" in text or "\\Z" in text else re.compile(text, flags | re.M), ""


def git_files(root):
    try:
        out = subprocess.run(["git", "-c", "core.quotePath=false", "-C", str(root), "ls-files", "-z", "-co",
                              "--exclude-standard", "."], capture_output=True, timeout=20,
                             creationflags=0x08000000 if D.WIN else 0)
    except (OSError, subprocess.TimeoutExpired):
        return None
    names = out.stdout.decode("utf-8", "replace").split("\0") if out.returncode == 0 else []
    return [root / n for n in sorted(set(n for n in names if n))] or None


def walk_files(root, hidden):
    for base, dirs, names in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in D.GREP_SKIP and (hidden or not d.startswith(".")))
        for n in sorted(names):
            if hidden or not n.startswith("."):
                yield Path(base) / n


def nice(p, base):
    try:
        return p.relative_to(base).as_posix()
    except ValueError:
        return p.as_posix()


def candidates(root, base, single, o, globs):
    if root.is_file():
        yield root, nice(root, base)
        return
    listed = git_files(root)
    for p in listed if listed is not None else walk_files(root, o["hidden"]):
        rel = p.relative_to(root).as_posix()
        if globs and not any(fnmatch.fnmatch(p.name, g) or fnmatch.fnmatch(rel, g) for g in globs):
            continue
        yield p, rel if single else nice(p, base)


def snippet(line, m, rx, keep):
    text = line.rstrip() if keep else line.strip()
    if len(text) <= D.GREP_LONG:
        return text
    if m is None:
        return text[:D.GREP_LONG] + "..."
    shift = 0 if keep else len(line) - len(line.lstrip())
    a = max(0, m.start() - shift - D.GREP_SIDE)
    b = min(len(text), max(a, m.end() - shift) + D.GREP_SIDE, a + 2 * D.GREP_LONG)
    more = sum(1 for _ in rx.finditer(line)) - 1
    return ("..." if a else "") + text[a:b] + ("..." if b < len(text) else "") + (
        " (+%d more in this line)" % more if more > 0 else "")


def hits(lines, rx, o):
    before, after, keep = o["before"], o["after"], bool(o["before"] or o["after"])
    buf, left, shown, count, last = deque(maxlen=before or 1), 0, [], 0, 0
    for no, line in enumerate(lines, 1):
        m = rx.search(line)
        if m:
            count += 1
            if len(shown) < D.GREP_KEEP:
                if before:
                    shown += [(n, snippet(t, None, rx, True), "-") for n, t in buf if n > last]
                shown.append((no, snippet(line, m, rx, keep), ":"))
                last = no
            buf.clear()
            left = after
        elif left:
            left -= 1
            if len(shown) < D.GREP_KEEP:
                shown.append((no, snippet(line, None, rx, True), "-"))
                last = no
        elif before:
            buf.append((no, line))
    return count, shown


def stream(path):
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        for line in fh:
            yield line.rstrip("\n")


def scan(path, rx, pre, o):
    try:
        size = path.stat().st_size
        if size > D.GREP_MAX_BYTES:
            return "big"
        with open(path, "rb") as fh:
            if b"\0" in fh.read(8000):
                return "binary"
        if size > D.GREP_WHOLE:
            return hits(stream(path), rx, o)
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        if pre is not None and not pre.search(text):
            return 0, []
        return hits((text[:-1] if text.endswith("\n") else text).split("\n"), rx, o)
    except OSError:
        return "gone"


def render(results, o, budget):
    if o["files"]:
        groups = {}
        for show, count, _ in results:
            folder, _, name = show.rpartition("/")
            groups.setdefault(folder, []).append("%s (%d)" % (name, count))
        return ["%s/  %s" % (f, ", ".join(names)) if f else ", ".join(names) for f, names in groups.items()], 0
    per_file = max(D.GREP_PER_FILE, budget // max(1, len(results)))
    out, used, rest, context = [], 0, [], bool(o["before"] or o["after"])
    for show, count, shown in results:
        if used >= budget:
            rest.append("%s (%d)" % (show, count))
            continue
        allow, n, prev = min(per_file, budget - used), 0, None
        out.append("%s (%d)" % (show, count) if count > 1 else show)
        for no, text, kind in shown:
            if kind == ":":
                if n >= allow:
                    break
                n += 1
            if context and prev is not None and no > prev + 1:
                out.append("  --")
            out.append(("  %d%s %s" % (no, kind, text)).rstrip())
            prev = no
        if count > n:
            out.append("  ... +%d more in this file" % (count - n))
        used += n
    if rest:
        out.append("more files: " + ", ".join(rest[:D.GREP_MORE_FILES]) + (
            ", +%d more" % (len(rest) - D.GREP_MORE_FILES) if len(rest) > D.GREP_MORE_FILES else ""))
    return out, sum(c for _, c, _ in results) - used


def cmd_grep(ctx, pos, flags):
    o, rest = options(pos, flags)
    if not rest:
        raise Fail("what should I search for?", USAGE)
    pattern, names = rest[0], rest[1:]
    rx, pre, warn = matcher(pattern, o)
    globs = [g.strip() for g in str(flags.get("glob") or "").split(",") if g.strip() and flags.get("glob") is not True]
    base = Path(getattr(ctx, "cwd", None) or os.getcwd())
    roots = []
    for name in names or ["."]:
        p = Path(win_path(name)).expanduser()
        p = p if p.is_absolute() else base / p
        if not p.exists():
            raise Fail("no file or folder %s" % name, USAGE)
        roots.append(p)
    single = len(roots) == 1 and roots[0].is_dir()
    deadline = time.time() + D.GREP_BUDGET_S
    results, seen, skipped, searched, stopped = [], set(), {"binary": 0, "big": 0, "gone": 0}, 0, False
    for root in roots:
        for path, show in candidates(root, base, single, o, globs):
            if time.time() > deadline:
                stopped = True
                break
            key = os.path.normcase(str(path))
            if key in seen:
                continue
            seen.add(key)
            res = scan(path, rx, pre, o)
            if isinstance(res, str):
                skipped[res] += 1
                continue
            searched += 1
            if res[0]:
                results.append((show, res[0], res[1]))
        if stopped:
            break
    where = nice(roots[0], base) if len(roots) == 1 else "%d places" % len(roots)
    notes = ["%d %s" % (n, {"binary": "binary", "big": "over %d MB" % (D.GREP_MAX_BYTES // 1_000_000),
                            "gone": "unreadable"}[k]) for k, n in skipped.items() if n]
    tail = ([", skipped " + ", ".join(notes)] if notes else []) + (
        ["; stopped after %d s, narrow the folder or add --glob" % D.GREP_BUDGET_S] if stopped else []) + (
        ["; " + warn] if warn else [])
    if not results:
        return "no line matches \"%s\" in %d files under %s%s" % (pattern, searched, where, "".join(tail))
    total = sum(c for _, c, _ in results)
    budget = 10 ** 9 if o["all"] else int(flags["max"]) if str(flags.get("max") or "").isdigit() else D.GREP_LINES
    lines, hidden = render(results, o, budget)
    summary = "%d line%s" % (total, "" if total == 1 else "s") + (
        " in %d files" % len(results) if len(results) > 1 else "") + (" (%d searched)" % searched if searched > 1 else "")
    text = "\n".join(lines + [summary + "".join(tail)])
    if o["all"] or not hidden:
        return cap(text, "grep")
    full = "\n".join(render(results, dict(o, files=False), 10 ** 9)[0])
    path = outfile("grep", "txt")
    path.write_text(full, "utf-8")
    trimmed(len(full) - len(text))
    return text + "\nfull list: %s" % path.as_posix()
