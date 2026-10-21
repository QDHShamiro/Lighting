import re
import time
from pathlib import Path

from lighting import defaults as D
from lighting import memory as M

FRONT = re.compile(r"(?s)\A---\s*\n(.*?)\n---\s*\n")
HEAD = re.compile(r"(?m)^#\s+(.+)$")
KIND_DIRS = [("knowledge/mistakes", "mistake"), ("knowledge/patterns", "pattern"), ("rules", "rule"),
             ("projects", "project"), ("sessions", "session"), ("eingang", "draft"), ("resources", "reference"),
             ("archive", "archive"), ("notes", "note")]
SKIP = {".obsidian", ".obsidian-mcp", ".trash", "templates", ".git"}
FILE_SOURCES = ("vault", "claude")


def root():
    own = D.config().get("vault")
    return Path(own) if own else D.VAULT


def claude_dirs():
    base = Path.home() / ".claude" / "projects"
    try:
        return [d / "memory" for d in base.iterdir() if (d / "memory").is_dir()]
    except OSError:
        return []


def front(text):
    m = FRONT.match(text)
    if not m:
        return {}, text
    meta = {}
    for line in m.group(1).splitlines():
        k, sep, v = line.partition(":")
        if sep and not line.startswith((" ", "\t")):
            meta[k.strip().lower()] = v.strip().strip("\"'")
    return meta, text[m.end():]


def vault_kind(rel):
    low = rel.lower()
    return next((kind for prefix, kind in KIND_DIRS if low.startswith(prefix + "/")), "note")


def vault_project(rel, meta):
    if meta.get("project"):
        return meta["project"].strip("[]").split("/")[-1]
    parts = rel.split("/")
    if parts[0].lower() in ("projects", "sessions") and len(parts) >= 2:
        return Path(parts[1]).stem if len(parts) == 2 else parts[1]
    return ""


def parse(path, text, rel, claude):
    meta, body = front(text)
    head = HEAD.search(body)
    title = meta.get("title") or meta.get("name") or (head.group(1).strip() if head else Path(rel).stem)
    if claude:
        kind = (meta.get("type") or "").strip() or "rule"
        if meta.get("description"):
            body = meta["description"] + "\n" + body
        return "", kind, title, body
    return vault_project(rel, meta), vault_kind(rel), title, body


def files(base, claude):
    if claude:
        return [(p, p.name) for p in base.glob("*.md") if p.name != "MEMORY.md"]
    out = []
    for p in base.rglob("*.md"):
        rel = p.relative_to(base).as_posix()
        if not any(part.lower() in SKIP for part in rel.split("/")[:-1]):
            out.append((p, rel))
    return out


def index_source(source, bases, claude):
    have = {r["origin"]: (r["id"], r["updated"]) for r in
            M.rows("select id, origin, updated from notes where source = ?", (source,))}
    seen, changed = set(), 0
    for base in bases:
        if not base.is_dir():
            continue
        for p, rel in files(base, claude):
            origin = "%s:%s" % (source, p.as_posix())
            seen.add(origin)
            try:
                mtime = p.stat().st_mtime
            except OSError:
                continue
            old = have.get(origin)
            if old and abs((old[1] or 0) - mtime) < 1:
                continue
            try:
                text = p.read_text("utf-8", errors="replace")
            except OSError:
                continue
            project, kind, title, body = parse(p, text, rel, claude)
            title, body = M.oneline(M.scrub(title), 200), M.scrub(body.strip())[: D.VAULT_BODY]
            if old:
                M.write("update notes set project = ?, kind = ?, title = ?, body = ?, updated = ? where id = ?",
                        (project, kind, title, body, mtime, old[0]))
            else:
                M.write("insert into notes(project, kind, title, body, files, source, created, updated, origin) "
                        "values (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        (project, kind, title, body, "[]", source, mtime, mtime, origin))
            changed += 1
    gone = [(v[0],) for k, v in have.items() if k not in seen]
    if gone:
        M.many("delete from notes where id = ?", gone)
        M.drop_cache()
    return changed + len(gone)


def index():
    t = time.time()
    n = index_source("vault", [root()], False) + index_source("claude", claude_dirs(), True)
    M.meta("vault_indexed", t)
    return n


def path_of(row):
    origin = row["origin"] or ""
    return origin.split(":", 1)[1] if row["source"] in FILE_SOURCES and ":" in origin else ""
