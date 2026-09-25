import re
import time
from pathlib import Path

from lighting import defaults as D

REF = re.compile(r"^(?:(f\d+)\.)?(e|d|o|w|t)(\d+)$")


class Fail(Exception):
    def __init__(self, msg, hint=None):
        super().__init__(msg)
        self.hint = hint

    def text(self):
        return "err: " + str(self) + (" -> try: " + self.hint if self.hint else "")


def ref_kind(token):
    m = REF.match(token or "")
    return m.group(2) if m else None


def is_url(token):
    return bool(re.match(r"^(https?|file)://", token or "", re.I))


def win_path(p):
    m = re.match(r"^/([a-zA-Z])/(.*)$", p or "")
    return (m.group(1).upper() + ":/" + m.group(2)) if m else p


def outfile(name, ext):
    D.OUT.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%H%M%S") + "-%03d" % (int(time.time() * 1000) % 1000)
    return D.OUT / ("%s-%s.%s" % (stamp, re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:30] or "out", ext))


def cap(text, name, lines=None, chars=D.OUT_CHARS):
    rows = text.split("\n")
    if (lines is None or len(rows) <= lines) and len(text) <= chars:
        return text
    path = outfile(name, "txt")
    path.write_text(text, "utf-8")
    keep = rows[:lines] if lines else rows
    out = "\n".join(keep)
    if len(out) > chars:
        out = out[:chars].rsplit("\n", 1)[0]
    shown = out.count("\n") + 1
    return out + "\n... +%d more lines (full: %s)" % (len(rows) - shown, Path(path).as_posix())


def tokens(text):
    return len(text) // 4 + 1


CODE_RE = re.compile(r"(```[\s\S]*?```|`[^`\n]*`)")
LINK_RE = re.compile(r"!?\[([^\]]*)\]\((?:[^()]|\([^()]*\))*\)")
FOOT_RE = re.compile(r"\[\^[^\]]+\](?!:)")


def plain_links(md):
    parts = CODE_RE.split(md)
    return "".join(p if i % 2 else FOOT_RE.sub("", LINK_RE.sub(r"\1", p)) for i, p in enumerate(parts))


def terms(f):
    return [t.strip() for t in (f or "").lower().split("|") if t.strip()]


def hit(words, *texts):
    return any(w in (x or "").lower() for w in words for x in texts)
