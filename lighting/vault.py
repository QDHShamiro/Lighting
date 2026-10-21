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
SECTIONS = [("summary", "Done"), ("why", "Why"), ("rejected", "Tried and rejected"), ("next", "Left open")]
WIKI_KINDS = ("mistake", "rule", "pattern")
BUILTIN = {
    "mistake": "## Problem\n## Cause\n## Fix\n## Rule\n",
    "rule": "## Rule\n## Why\n## When it applies\n",
    "pattern": "## Pattern\n## When to use\n## Example\n",
}
HOMES = {"mistake": "Knowledge/Mistakes", "pattern": "Knowledge/Patterns", "rule": "Rules"}
INDEXES = {"mistake": "Knowledge/INDEX.md", "pattern": "Knowledge/INDEX.md", "rule": "Rules/INDEX.md"}
SECTION_OF = {"mistake": "Mistakes", "pattern": "Patterns"}


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
        name = Path(parts[1]).stem if len(parts) == 2 else parts[1]
        return "" if name == "general" else name
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


def safe_name(text, n=60):
    s = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', " ", text or "").strip(" .")
    return " ".join(s.split())[:n].strip(" .") or "session"


def session_file(sid, project):
    folder = root() / "Sessions" / safe_name(project or "general", 80)
    old = next(iter(folder.glob("* %s.md" % sid[:8])), None) if folder.is_dir() else None
    return folder, old


def write_session(sid, project, record, started):
    folder, old = session_file(sid, project)
    title = M.oneline(record.get("title") or "session", 120)
    lines = ["# " + title, "Session: %s   Date: %s   Project: %s" % (
        sid[:8], time.strftime("%Y-%m-%d", time.localtime(started)), project or "-"), ""]
    for key, head in SECTIONS:
        text = (record.get(key) or "").strip()
        if text:
            lines += ["## " + head] + [("- " + x.lstrip("-* ").strip()) for x in text.splitlines() if x.strip()] + [""]
    path = folder / ("%s %s %s.md" % (time.strftime("%Y-%m-%d", time.localtime(started)), safe_name(title), sid[:8]))
    try:
        folder.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines), "utf-8", newline="\n")
        if old and old != path:
            old.unlink()
    except OSError:
        return None
    return path


def inbox():
    return root() / (D.config().get("vault_inbox") or "Inbox")


def template(kind):
    p = root() / "Templates" / ("%s.md" % kind)
    try:
        text = p.read_text("utf-8")
    except OSError:
        return BUILTIN[kind], [h for h in re.findall(r"(?m)^##\s+(.+)$", BUILTIN[kind])]
    return text, [h.strip() for h in re.findall(r"(?m)^##\s+(.+)$", text)]


def render(kind, title, sections, project, sid):
    heads = template(kind)[1]
    lines = ["---", "type: %s" % kind, "date: %s" % time.strftime("%Y-%m-%d"), "project: %s" % (project or "-"),
             "draft: true", "session: %s" % sid[:8], "---", "# " + title, ""]
    for h in heads:
        value = (sections.get(h) or "").strip()
        if value:
            lines += ["## " + h, value, ""]
    if project:
        lines.append("[[Projects/%s]]" % project)
    return "\n".join(lines) + "\n"


def write_draft(kind, title, sections, project, sid):
    folder = inbox()
    path = folder / ("%s %s %s.md" % (time.strftime("%Y-%m-%d"), kind, safe_name(title)))
    if path.exists():
        return None
    try:
        folder.mkdir(parents=True, exist_ok=True)
        path.write_text(render(kind, title, sections, project, sid), "utf-8", newline="\n")
    except OSError:
        return None
    return path


def drafts():
    folder = inbox()
    return sorted(folder.glob("*.md")) if folder.is_dir() else []


def accept(path, folder=None):
    text = path.read_text("utf-8")
    meta, _ = front(text)
    kind = meta.get("type") or "note"
    dest_dir = root() / (folder or HOMES.get(kind, "Notes"))
    dest = dest_dir / (re.sub(r"^\d{4}-\d{2}-\d{2} \w+ ", "", path.stem) + ".md")
    if dest.exists():
        raise ValueError("%s already exists" % dest.relative_to(root()).as_posix())
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest.write_text(re.sub(r"(?m)^draft: true\n", "", text, count=1), "utf-8")
    path.unlink()
    register(dest, kind, meta.get("project", "-"))
    return dest


def register(dest, kind, project):
    rel = dest.relative_to(root()).with_suffix("").as_posix()
    head = HEAD.search(dest.read_text("utf-8"))
    line = "- [[%s]] — %s (%s%s)" % (rel, head.group(1).strip() if head else dest.stem, kind,
                                    ", " + project if project and project != "-" else "")
    index = root() / INDEXES.get(kind, "Knowledge/INDEX.md")
    if not index.exists():
        return
    text = index.read_text("utf-8")
    section = SECTION_OF.get(kind)
    m = re.search(r"(?m)^## %s\s*$" % re.escape(section), text) if section else None
    if m:
        end = text.find("\n## ", m.end())
        end = len(text) if end < 0 else end
        text = text[:end].rstrip("\n") + "\n" + line + "\n" + text[end:]
    else:
        text = text.rstrip("\n") + "\n" + line + "\n"
    index.write_text(text, "utf-8")


def drop(path):
    trash = root() / ".trash"
    trash.mkdir(exist_ok=True)
    path.replace(trash / path.name)


def path_of(row):
    origin = row["origin"] or ""
    return origin.split(":", 1)[1] if row["source"] in FILE_SOURCES and ":" in origin else ""
