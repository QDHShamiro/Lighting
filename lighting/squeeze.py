import json
import math
import os
import re
import shutil
import stat
import subprocess
import sys
from collections import Counter
from pathlib import Path

from lighting import defaults as D
from lighting.common import Fail, outfile, trimmed, win_path

ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Z0-9]")
IMPORTANT = re.compile(D.SQUEEZE_IMPORTANT, re.I)
TRACE = re.compile(D.SQUEEZE_TRACE_LINE)
PREFIX = re.compile(D.SQUEEZE_PREFIX)
NUMS = re.compile(r"0x[0-9a-fA-F]+|\b[0-9a-fA-F]{8,}\b|\d+")
QUIET = re.compile(r'"[\w-]*(?:error|fail|exception|warn)[\w-]*"\s*:\s*(?:null|false|0|\[\]|\{\}|"")', re.I)
USAGE = "lighting squeeze <file> | <command> 2>&1 | lighting squeeze | lighting squeeze -- <command>"


def clean(text):
    out = []
    for line in ANSI.sub("", text.replace("\r\n", "\n")).split("\n"):
        if "\r" in line:
            parts = [p for p in line.split("\r") if p.strip()]
            line = parts[-1] if parts else ""
        out.append(line.rstrip())
    while out and not out[-1]:
        out.pop()
    return out


def body(line):
    return PREFIX.sub("", line, count=1)


def template(line):
    return " ".join(NUMS.sub("#", body(line)).split())


def kind(line):
    return " ".join(template(line).split()[:3])[:60]


def tracey(line):
    b = body(line)
    return bool(TRACE.search(b)) or b[:1] in (" ", "\t")


def short_line(line):
    extra = len(line) - D.SQUEEZE_LINE_CHARS
    return line if extra <= 0 else line[:D.SQUEEZE_LINE_CHARS] + "... (+%d chars)" % extra


def marked(lines, after, trace):
    n = len(lines)
    keep = [i < D.SQUEEZE_HEAD or i >= n - D.SQUEEZE_TAIL for i in range(n)]
    flow = [tracey(line) for line in lines]
    seen = Counter()
    for i, line in enumerate(lines):
        if flow[i] or not IMPORTANT.search(line):
            continue
        t = template(line)
        seen[t] += 1
        if seen[t] > D.SQUEEZE_SAME:
            continue
        keep[i] = True
        j = i + 1
        while j < n and j - i <= trace and flow[j]:
            keep[j] = True
            j += 1
        for k in range(j, min(n, j + after)):
            keep[k] = True
    return keep


def gap(run):
    kinds = Counter(kind(line) for line in run if line.strip()).most_common(2)
    what = ", ".join("%d x %s" % (c, k) for k, c in kinds if k and c > 1)
    return "... %d lines%s" % (len(run), " (%s)" % what if what else "")


def assemble(lines, keep):
    out, run, prev, same = [], [], None, 1
    for line, k in list(zip(lines, keep)) + [(None, True)]:
        if line is not None and not k:
            run.append(line)
            continue
        t = template(line) if line is not None else None
        if line is not None and not run and t and t == prev:
            same += 1
            continue
        if same > 1:
            out[-1] += " (x%d)" % same
        same = 1
        if run:
            out += [short_line(r) for r in run] if len(run) <= D.SQUEEZE_GAP_KEEP else [gap(run)]
            run = []
        if line is not None:
            out.append(short_line(line))
        prev = t
    return out


def squeeze_log(lines):
    out = assemble(lines, marked(lines, D.SQUEEZE_AFTER, D.SQUEEZE_TRACE))
    if len(out) > D.SQUEEZE_LINES:
        out = assemble(lines, marked(lines, 0, D.SQUEEZE_TRACE_TIGHT))
    if len(out) > D.SQUEEZE_LINES:
        top = D.SQUEEZE_LINES - D.SQUEEZE_TAIL
        out = out[:top] + ["... %d more squeezed lines (see the full file)" % (len(out) - D.SQUEEZE_LINES)] + out[-D.SQUEEZE_TAIL:]
    return out


def as_json(text):
    s = text.strip()
    if s[:1] in ("[", "{"):
        try:
            return json.loads(s), False
        except ValueError:
            pass
    rows = [r for r in s.split("\n") if r.strip()]
    if len(rows) < 2 or not all(r.lstrip()[:1] == "{" for r in rows[:50]):
        return None, False
    items, bad = [], 0
    for r in rows:
        try:
            items.append(json.loads(r))
        except ValueError:
            bad += 1
    return (items, True) if bad <= len(rows) // 10 else (None, False)


def outliers(dicts):
    found = []
    for key, c in Counter(k for it in dicts for k in it).items():
        if c < 0.8 * len(dicts):
            continue
        vals = [(i, it.get(key)) for i, it in enumerate(dicts)]
        nums = [(i, v) for i, v in vals if isinstance(v, (int, float)) and not isinstance(v, bool)]
        if len(nums) >= 10:
            mean = sum(v for _, v in nums) / len(nums)
            sd = math.sqrt(sum((v - mean) ** 2 for _, v in nums) / len(nums))
            found += [i for i, v in nums if sd and abs(v - mean) > 3 * sd]
        words = Counter(v for _, v in vals if isinstance(v, (str, bool)))
        if len(dicts) >= 20 and 1 < len(words) <= 5:
            found += [i for i, v in vals if isinstance(v, (str, bool)) and words[v] == 1]
    return found


def brief(x):
    if isinstance(x, dict):
        return {k: brief(v) for k, v in x.items() if not (isinstance(v, str) and len(v) > D.SQUEEZE_SIGNAL)}
    if isinstance(x, list):
        return [brief(v) for v in x[:D.SQUEEZE_ITEMS]]
    return x


def picks(items):
    found, kinds, bad = [], set(), 0
    for i, it in enumerate(items):
        text = QUIET.sub("", json.dumps(brief(it), ensure_ascii=False)[:4000])
        if not IMPORTANT.search(text):
            continue
        bad += 1
        sig = NUMS.sub("#", text)
        if sig not in kinds:
            kinds.add(sig)
            found.append(i)
    if len(items) >= 10 and all(isinstance(it, dict) for it in items):
        found += outliers(items)
    return list(dict.fromkeys(found))[:D.SQUEEZE_PICKS], bad


def constant(items):
    if not 2 <= len(items) <= D.SQUEEZE_SAME_MAX or not all(isinstance(it, dict) for it in items):
        return {}
    first = items[0]
    return {k: v for k, v in first.items()
            if all(k in it for it in items) and len({json.dumps(it[k], sort_keys=True) for it in items}) == 1}


def crush(obj):
    if isinstance(obj, list) and len(obj) > D.SQUEEZE_ITEMS:
        n, same = len(obj), constant(obj)
        chosen, bad = picks(obj)
        keep = sorted(set(range(D.SQUEEZE_FIRST)) | set(range(n - D.SQUEEZE_LAST, n)) | set(chosen))
        out, last = [], -1
        for i in keep:
            if i - last > 1:
                out.append("<... %d more items>" % (i - last - 1))
            item = {k: v for k, v in obj[i].items() if k not in same} if same else obj[i]
            out.append(crush(item))
            last = i
        head = "<%d items%s" % (n, ", %d with error words" % bad if bad else "")
        if same:
            shown = ["%s=%s" % (k, json.dumps(v, ensure_ascii=False)) if len(json.dumps(v)) <= D.SQUEEZE_SHORT else k
                     for k, v in same.items()]
            head += ", the same in all: " + ", ".join(shown)
        out.insert(0, compact(head + ">"))
        return out
    if isinstance(obj, list):
        return [crush(x) for x in obj]
    if isinstance(obj, dict):
        return {k: crush(v) for k, v in obj.items()}
    if isinstance(obj, str) and len(obj) > D.SQUEEZE_STR:
        return obj[:D.SQUEEZE_STR] + "... (+%d chars)" % (len(obj) - D.SQUEEZE_STR)
    return obj


def weight(item):
    s = json.dumps(item[1], ensure_ascii=False)
    return s.startswith('"http'), len(s)


def compact(x):
    if isinstance(x, str) and x.startswith("<") and x.endswith(">"):
        return x if len(x) <= D.SQUEEZE_ITEM_CHARS else x[:D.SQUEEZE_ITEM_CHARS] + "...>"
    if isinstance(x, dict):
        x = dict(sorted(x.items(), key=weight))
    s = json.dumps(x, ensure_ascii=False, separators=(",", ":"))
    extra = len(s) - D.SQUEEZE_ITEM_CHARS
    return s if extra <= 0 else s[:D.SQUEEZE_ITEM_CHARS] + "... (+%d chars)" % extra


def squeeze_json(obj, lines_mode):
    small = crush(obj)
    if isinstance(small, list):
        rows = [compact(x) for x in small]
        return rows if lines_mode else ["["] + rows + ["]"]
    if isinstance(small, dict):
        out = ["{"]
        for k, v in small.items():
            key = json.dumps(k, ensure_ascii=False)
            if isinstance(v, list) and len(v) > 1:
                out += ["%s: [" % key] + ["  " + compact(x) for x in v] + ["]"]
            else:
                out.append("%s: %s" % (key, compact(v)))
        return out + ["}"]
    return [compact(small)]


def squeeze(text, name="output"):
    lines = clean(text)
    plain = "\n".join(lines)
    if len(lines) <= D.SQUEEZE_MIN_LINES and len(plain) <= D.SQUEEZE_MIN_CHARS:
        return plain
    obj, lines_mode = as_json(plain)
    out = squeeze_json(obj, lines_mode) if obj is not None else squeeze_log(lines)
    small = "\n".join(out)
    if len(small) >= len(plain):
        return plain
    path = outfile("squeeze-" + name, "txt")
    path.write_text(plain, "utf-8")
    trimmed(len(plain) - len(small))
    return small + "\nsqueezed %s lines (~%s tokens) to %d lines (~%s tokens), all of it: %s" % (
        "{:,}".format(len(lines)), "{:,}".format(len(plain) // 4), len(out), "{:,}".format(len(small) // 4),
        path.as_posix())


def read_limited(fh):
    keep, head, data, dropped = D.SQUEEZE_MAX_BYTES // 2, None, bytearray(), 0
    while True:
        chunk = fh.read(1 << 20)
        if not chunk:
            break
        data += chunk
        if head is None and len(data) > keep:
            head, data = bytes(data[:keep]), bytearray(data[keep:])
        elif head is not None and len(data) > keep:
            dropped += len(data) - keep
            del data[:len(data) - keep]
    if head is None:
        return bytes(data)
    return head + (b"\n... %d MB in the middle not read\n" % (dropped // 1_000_000) if dropped else b"") + bytes(data)


def cmd_squeeze(ctx, pos, flags):
    text, name = getattr(ctx, "stdin", None), str(flags.get("name") or "output")
    if pos and pos[0] == "--":
        raise Fail("this lighting client cannot run commands yet", "<command> > out.txt 2>&1; lighting squeeze out.txt")
    if pos:
        base, parts = Path(getattr(ctx, "cwd", None) or os.getcwd()), []
        for p in pos:
            path = Path(win_path(p)).expanduser()
            path = path if path.is_absolute() else base / path
            try:
                with open(path, "rb") as fh:
                    parts.append(read_limited(fh).decode("utf-8", "replace"))
            except OSError as e:
                raise Fail("cannot read %s (%s)" % (p, e.strerror or e), USAGE)
        text, name = "\n".join(parts), Path(pos[0]).name
    if text is None:
        raise Fail("nothing to squeeze", USAGE)
    return squeeze(text, name)


def piped():
    try:
        mode = os.fstat(sys.stdin.fileno()).st_mode
    except (OSError, ValueError, AttributeError):
        return False
    return stat.S_ISFIFO(mode) or stat.S_ISREG(mode)


def run_command(args):
    exe = shutil.which(args[0]) or args[0]
    try:
        p = subprocess.Popen([exe, *args[1:]], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError as e:
        return 127, "cannot start %s: %s" % (args[0], e.strerror or e)
    data = read_limited(p.stdout)
    return p.wait(), data.decode("utf-8", "replace")


def client(argv):
    from lighting.cli import execute
    rest, code, text = list(argv[1:]), None, None
    if "--" in rest:
        cmd, rest = rest[rest.index("--") + 1:], rest[:rest.index("--")]
        if not cmd:
            sys.stdout.write("err: which command? -> try: %s\n" % USAGE)
            return 2
        code, text = run_command(cmd)
        rest.append("--name=" + Path(cmd[0]).stem)
    elif not [a for a in rest if not a.startswith("-")] and piped():
        text = read_limited(sys.stdin.buffer).decode("utf-8", "replace")
    try:
        res = execute(["squeeze", *rest], {"stdin": text} if text is not None else None)
        out = res.get("out", "")
        code = res.get("code", 0) if code is None else code
    except Exception:
        if text is None:
            raise
        out = squeeze(text)
    sys.stdout.write(out + "\n")
    sys.stdout.flush()
    return code or 0
