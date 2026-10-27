import fnmatch
import os
import re
import subprocess
import threading
import time
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from lighting import defaults as D
from lighting.common import Fail, Pending, cap, outfile, trimmed, win_path

LETTERS = {"i": "ignore", "w": "word", "x": "line", "F": "fixed", "E": "ere", "P": "perl", "G": None, "l": "files",
           "c": "files", "a": "text", "o": "only", "v": "invert", "r": None, "R": None, "n": None, "H": None, "I": None,
           "h": None, "s": None}
NUMBERED = re.compile(r"-([ABCm])(\d*)")
BARE_BAR = re.compile(r"(?<!\\)\|")
INTERVAL = re.compile(r"\{\d*(?:,\d*)?\}")
POSIX = {"alpha": "a-zA-Z", "digit": "0-9", "alnum": "a-zA-Z0-9", "upper": "A-Z", "lower": "a-z", "space": r"\s",
         "blank": r" \t", "punct": r"!-/:-@\[-`{-~", "xdigit": "0-9A-Fa-f", "word": r"\w", "cntrl": r"\x00-\x1f\x7f",
         "print": r" -~", "graph": "!-~"}
USAGE = ("lighting grep <pattern> [folder|file ...] [-i] [-w] [-x] [-F|-E|-P] [-l] [-o] [-v] [-m 1] [-C 2] "
         "[--glob '*.py'] [--include '*.py'] [--exclude-dir build] [--all]")


def number(val, flag="-C"):
    if not str(val or "").isdigit():
        raise Fail("%s needs a number (%s)" % (flag, val), "lighting grep %s 2 <pattern>" % flag)
    return int(val)


def options(pos, flags):
    o = {"ignore": bool(flags.get("ignore-case")), "word": bool(flags.get("word") or flags.get("word-regexp")),
         "fixed": bool(flags.get("fixed") or flags.get("fixed-strings")), "line": bool(flags.get("line-regexp")),
         "ere": bool(flags.get("extended-regexp")), "perl": bool(flags.get("perl-regexp")),
         "files": bool(flags.get("files") or flags.get("files-with-matches") or flags.get("count")),
         "hidden": bool(flags.get("hidden")), "all": bool(flags.get("all")), "text": bool(flags.get("text")), "before": 0,
         "after": 0, "patterns": [], "only": bool(flags.get("only-matching")),
         "invert": bool(flags.get("invert-match")), "max": None}
    if flags.get("context") not in (None, True):
        o["before"] = o["after"] = number(flags["context"])
    if flags.get("max-count") not in (None, True):
        o["max"] = number(flags["max-count"], "-m")
    rest, i, done = [], 0, False
    while i < len(pos):
        a = pos[i]
        m = NUMBERED.fullmatch(a)
        if done or len(a) < 2 or not a.startswith("-"):
            rest.append(a)
        elif a == "--":
            done = True
        elif a == "-e":
            if i + 1 >= len(pos):
                raise Fail("-e needs a pattern after it", USAGE)
            i += 1
            o["patterns"].append(pos[i])
        elif m:
            val = m.group(2)
            if not val and i + 1 < len(pos):
                i += 1
                val = pos[i]
            n = number(val, "-" + m.group(1))
            o["max"] = n if m.group(1) == "m" else o["max"]
            o["before"] = n if m.group(1) in "BC" else o["before"]
            o["after"] = n if m.group(1) in "AC" else o["after"]
        elif re.fullmatch(r"-[A-Za-z]+", a):
            unknown = [c for c in a[1:] if c not in LETTERS]
            if unknown:
                raise Fail("grep option -%s is not supported" % unknown[0], "%s (a pattern that starts with - goes after --)" % USAGE)
            for c in a[1:]:
                if LETTERS[c]:
                    o[LETTERS[c]] = True
        else:
            rest.append(a)
        i += 1
    if o["only"] and o["invert"]:
        raise Fail("-o with -v shows nothing: the lines -v picks have no match", USAGE)
    if o["only"]:
        o["before"] = o["after"] = 0
    return o, rest


def bracket(p, i):
    j = i + 1 + p.startswith("^", i + 1)
    j += p.startswith("]", j)
    while j < len(p) and p[j] != "]":
        if p.startswith(("[:", "[.", "[="), j):
            end = p.find(p[j + 1] + "]", j + 2)
            if end < 0:
                return None, i + 1
            j = end + 2
        else:
            j += 2 if p[j] == "\\" else 1
    if j >= len(p):
        return None, i + 1
    body, out, k = p[i + 1:j], "", 0
    if body.startswith("^"):
        out, k = "^", 1
    if body.startswith("]", k):
        out, k = out + r"\]", k + 1
    while k < len(body):
        if body.startswith(("[:", "[.", "[="), k):
            end = body.find(body[k + 1] + "]", k + 2)
            name = body[k + 2:end]
            out += POSIX.get(name, re.escape(name)) if body[k + 1] == ":" else re.escape(name)
            k = end + 2
        elif body[k] == "\\" and k + 1 < len(body):
            out += body[k:k + 2]
            k += 2
        else:
            out += "\\" + body[k] if body[k] in "[&~|" else body[k]
            k += 1
    return "[" + out + "]", j + 1


def translate(p, ere):
    out, i = [], 0
    while i < len(p):
        c = p[i]
        start = not out or out[-1] in ("(", "|", "^")
        if c == "\\" and i + 1 < len(p):
            d, i = p[i + 1], i + 2
            if d in "(){}|+?" and not ere:
                out.append(d)
            elif d in "<>":
                out.append(r"\b")
            elif d in "`'":
                out.append("^" if d == "`" else "$")
            elif d in "wWsSbBdD123456789":
                out.append("\\" + d)
            else:
                out.append(re.escape(d))
            continue
        i += 1
        if c == "[":
            cls, i = bracket(p, i - 1)
            out.append(cls or r"\[")
        elif c == "*":
            out.append(r"\*" if start else "*")
        elif c in "+?":
            out.append(c if ere and not start else "\\" + c)
        elif c == "{":
            m = INTERVAL.match(p, i - 1)
            if ere and m and not start and m.group() not in ("{}", "{,}"):
                out.append(m.group())
                i += len(m.group()) - 1
            else:
                out.append(r"\{")
        elif c in "()|}":
            out.append(c if ere else "\\" + c)
        elif c == "^":
            out.append("^" if ere or start else r"\^")
        elif c == "$":
            out.append("$" if ere or i == len(p) or p.startswith(("\\)", "\\|"), i) else r"\$")
        else:
            out.append(c)
    return "".join(out)


def matcher(patterns, o):
    texts = [re.escape(p) if o["fixed"] else p if o["perl"] else translate(p, o["ere"]) for p in patterns]
    text = texts[0] if len(texts) == 1 else "|".join("(?:%s)" % t for t in texts)
    if o["word"]:
        text = r"(?<!\w)(?:%s)(?!\w)" % text
    if o["line"]:
        text = r"^(?:%s)$" % text
    flags = re.I if o["ignore"] else 0
    try:
        rx = re.compile(text, flags)
    except re.error as e:
        if o["fixed"]:
            raise Fail("cannot read the pattern: %s" % e, USAGE)
        rx, pre, _ = matcher(patterns, dict(o, fixed=True))
        return rx, pre, "not a regex (%s), searched as plain text" % e
    return rx, None if "\\A" in text or "\\Z" in text else re.compile(text, flags | re.M), ""


def braces(glob):
    m = re.search(r"\{([^{}]*,[^{}]*)\}", glob)
    if not m:
        return [glob]
    return [g for part in m.group(1).split(",") for g in braces(glob[:m.start()] + part + glob[m.end():])]


def split_list(val):
    if not isinstance(val, str):
        return []
    parts, cur, depth = [], "", 0
    for ch in val:
        if ch == "," and not depth:
            parts.append(cur)
            cur = ""
            continue
        depth = max(0, depth + {"{": 1, "}": -1}.get(ch, 0))
        cur += ch
    return [g for part in parts + [cur] if part.strip() for g in braces(part.strip())]


def git_files(root):
    try:
        out = subprocess.run(["git", "-c", "core.quotePath=false", "-C", str(root), "ls-files", "-z", "-co",
                              "--exclude-standard", "."], capture_output=True, timeout=20,
                             creationflags=0x08000000 if D.WIN else 0)
    except (OSError, subprocess.TimeoutExpired):
        return None
    names = out.stdout.decode("utf-8", "replace").split("\0") if out.returncode == 0 else []
    names = sorted(set(n for n in names if n))
    nested = [n for n in names if n.endswith("/")]
    inner = {}
    if nested:
        with ThreadPoolExecutor(min(D.GREP_GIT_WORKERS, len(nested))) as pool:
            inner = dict(zip(nested, pool.map(lambda n: git_files(root / n), nested)))
    found = []
    for n in names:
        found += (inner[n] or []) if n.endswith("/") else [root / n]
    return found or None


def walk_files(root, hidden, skipdirs):
    for base, dirs, names in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in D.GREP_SKIP and (hidden or not d.startswith("."))
                         and not any(fnmatch.fnmatch(d, g) for g in skipdirs))
        for n in sorted(names):
            if hidden or not n.startswith("."):
                yield Path(base) / n


def nice(p, base):
    try:
        return p.relative_to(base).as_posix()
    except ValueError:
        return p.as_posix()


def candidates(root, base, single, o, globs, skipdirs):
    if root.is_file():
        yield root, nice(root, base)
        return
    listed = git_files(root)
    take = [g for g in globs if not g.startswith("!")]
    drop = [g[1:] for g in globs if g.startswith("!")]
    top = str(root)
    cut = len(top) if top.endswith(os.sep) else len(top) + 1
    for p in listed if listed is not None else walk_files(root, o["hidden"], skipdirs):
        rel = str(p)[cut:].replace(os.sep, "/")
        if skipdirs and any(fnmatch.fnmatch(part, g) for part in rel.split("/")[:-1] for g in skipdirs):
            continue
        if take and not any(fnmatch.fnmatch(p.name, g) or fnmatch.fnmatch(rel, g) for g in take):
            continue
        if any(fnmatch.fnmatch(p.name, g) or fnmatch.fnmatch(rel, g) for g in drop):
            continue
        yield p, rel if single else nice(p, base)


def snippet(line, m, rx, keep):
    text = (line.rstrip() if keep else line.strip()).replace("\r", " ").replace("\0", " ")
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
    limit = float("inf") if o["max"] is None else o["max"]
    buf, left, shown, count, picked, last = deque(maxlen=before or 1), 0, [], 0, 0, 0
    for no, line in enumerate(lines, 1):
        m = rx.search(line)
        if (m is None) == o["invert"] and picked < limit:
            picked += 1
            parts = [x for x in rx.finditer(line) if x.group()] if o["only"] else [m]
            count += len(parts)
            if len(shown) < D.GREP_KEEP:
                if before:
                    shown += [(n, snippet(t, None, rx, True), "-") for n, t in buf if n > last]
                if o["only"]:
                    shown += [(no, snippet(x.group(), None, rx, True), ":") for x in parts]
                else:
                    shown.append((no, snippet(line, m, rx, keep), ":"))
                last = no
            buf.clear()
            left = after
        elif left:
            left -= 1
            if len(shown) < D.GREP_KEEP:
                shown.append((no, snippet(line, None, rx, True), "-"))
                last = no
        elif picked >= limit:
            break
        elif before:
            buf.append((no, line))
    return count, shown


def encoding_of(head):
    if head.startswith((b"\xff\xfe", b"\xfe\xff")):
        return "utf-16"
    if b"\0" in head:
        return None
    return "utf-8-sig" if head.startswith(b"\xef\xbb\xbf") else "utf-8"


def stream(path, enc):
    with open(path, "r", encoding=enc, errors="replace", newline="\n") as fh:
        for line in fh:
            line = line[:-1] if line.endswith("\n") else line
            yield line[:-1] if line.endswith("\r") else line


def scan(path, rx, pre, o):
    try:
        with open(path, "rb") as fh:
            size = os.fstat(fh.fileno()).st_size
            if size > D.GREP_MAX_BYTES:
                return "big"
            raw = fh.read(8000 if size > D.GREP_WHOLE else -1)
        enc = encoding_of(raw[:8000])
        if enc is None and not o["text"]:
            return "binary"
        enc = enc or "utf-8"
        if size > D.GREP_WHOLE:
            return hits(stream(path, enc), rx, o)
        text = raw.decode(enc, "replace").replace("\r\n", "\n")
        if pre is not None and not o["invert"] and not pre.search(text):
            return 0, []
        return hits((text[:-1] if text.endswith("\n") else text).split("\n"), rx, o)
    except OSError:
        return "gone"


def render(results, o, budget, bare=False):
    if o["files"]:
        groups = {}
        for show, count, _ in results:
            folder, _, name = show.rpartition("/")
            groups.setdefault(folder, []).append("%s (%d)" % (name, count))
        return ["%s/  %s" % (f, ", ".join(names)) if f else ", ".join(names) for f, names in groups.items()], 0
    per_file = max(D.GREP_PER_FILE, budget // max(1, len(results)))
    out, used, rest, context = [], 0, [], bool(o["before"] or o["after"])
    pad = "" if bare else "  "
    for show, count, shown in results:
        if used >= budget:
            rest.append("%s (%d)" % (show, count))
            continue
        allow, n, prev = min(per_file, budget - used), 0, None
        if not bare:
            out.append("%s (%d)" % (show, count) if count > 1 else show)
        for no, text, kind in shown:
            if kind == ":":
                if n >= allow:
                    break
                n += 1
            if context and prev is not None and no > prev + 1:
                out.append(pad + "--")
            out.append(("%s%d%s %s" % (pad, no, kind, text)).rstrip())
            prev = no
        if count > n:
            out.append("%s... +%d more in this file" % (pad, count - n))
        used += n
    if rest:
        out.append("more files: " + ", ".join(rest[:D.GREP_MORE_FILES]) + (
            ", +%d more" % (len(rest) - D.GREP_MORE_FILES) if len(rest) > D.GREP_MORE_FILES else ""))
    return out, sum(c for _, c, _ in results) - used


def search(job, roots, base, o, globs, rx, pre, skipdirs=()):
    single = len(roots) == 1 and roots[0].is_dir()
    deadline = time.time() + D.GREP_BUDGET_S
    results, seen, skipped, searched, stopped = [], set(), {"binary": 0, "big": 0, "gone": 0}, 0, False
    try:
        for root in roots:
            for path, show in candidates(root, base, single, o, globs, skipdirs):
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
    except Exception as e:
        job["error"] = e
    job.update(results=results, skipped=skipped, searched=searched, stopped=stopped)


def cmd_grep(ctx, pos, flags):
    o, rest = options(pos, flags)
    pats = o["patterns"] or rest[:1]
    if not pats:
        raise Fail("what should I search for?", USAGE)
    pattern, names = '" or "'.join(pats), rest if o["patterns"] else rest[1:]
    rx, pre, warn = matcher(pats, o)
    globs = split_list(flags.get("glob")) + split_list(flags.get("include")) + [
        "!" + g for g in split_list(flags.get("exclude"))]
    skipdirs = split_list(flags.get("exclude-dir"))
    base = Path(getattr(ctx, "cwd", None) or os.getcwd())
    roots = []
    for name in names or ["."]:
        p = Path(win_path(name)).expanduser()
        p = p if p.is_absolute() else base / p
        if not p.exists():
            raise Fail("no file or folder %s" % name, USAGE)
        roots.append(p)
    job = {}
    worker = threading.Thread(target=search, args=(job, roots, base, o, globs, rx, pre, skipdirs), daemon=True)
    worker.start()
    worker.join(D.GREP_INLINE_S)
    if not worker.is_alive():
        return answer(job, pattern, roots, o, flags, warn)

    def poll(last):
        if worker.is_alive():
            if last:
                raise Fail("the search did not end in time", "narrow the folder or add --glob")
            return None
        return answer(job, pattern, roots, o, flags, warn)

    return Pending(poll, D.GREP_BUDGET_S + D.GREP_INLINE_S + 10, D.GREP_POLL_S)


def answer(job, pattern, roots, o, flags, warn):
    if "error" in job:
        raise Fail("search failed: %s: %s" % (type(job["error"]).__name__, str(job["error"])[:200]), USAGE)
    results, skipped, searched, stopped = job["results"], job["skipped"], job["searched"], job["stopped"]
    notes = ["%d %s" % (n, {"binary": "binary", "big": "over %d MB" % (D.GREP_MAX_BYTES // 1_000_000),
                            "gone": "unreadable"}[k]) for k, n in skipped.items() if n]
    tail = ([", skipped " + ", ".join(notes)] if notes else []) + (
        ["; stopped after %d s, narrow the folder or add --glob" % D.GREP_BUDGET_S] if stopped else []) + (
        ["; " + warn] if warn else [])
    if not results:
        if not (o["fixed"] or o["ere"] or o["perl"] or o["invert"]) and BARE_BAR.search(pattern):
            tail.append('; | is a plain character here, use \\| or -E for "or"')
        return "%s \"%s\" in %d file%s%s" % ("every line matches" if o["invert"] else "no line matches", pattern,
                                             searched, "" if searched == 1 else "s", "".join(tail))
    total = sum(c for _, c, _ in results)
    budget = 10 ** 9 if o["all"] else int(flags["max"]) if str(flags.get("max") or "").isdigit() else D.GREP_LINES
    bare = len(roots) == 1 and roots[0].is_file()
    lines, hidden = render(results, o, budget, bare)
    if hidden or tail:
        lines.append("%d line%s" % (total, "" if total == 1 else "s") + (
            " in %d files" % len(results) if len(results) > 1 else "") + (
            " (%d searched)" % searched if searched > 1 else "") + "".join(tail))
    text = "\n".join(lines)
    if o["all"] or not hidden:
        return cap(text, "grep")
    full = "\n".join(render(results, dict(o, files=False), 10 ** 9, bare)[0])
    path = outfile("grep", "txt")
    path.write_text(full, "utf-8")
    trimmed(len(full) - len(text))
    return text + "\nfull list: %s" % path.as_posix()
