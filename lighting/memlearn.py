import json
import re
import sqlite3
import threading
import time
from pathlib import Path

from lighting import defaults as D
from lighting import games
from lighting import memmodel as MM
from lighting import memory as M
from lighting import vault

KINDS = ["decision", "preference", "bugfix", "feature", "fact", "discovery", "change", "open"]
SYSTEM = (
    "You keep the long-term memory of a coding assistant. From the session log, keep only what helps in a FUTURE "
    "session: decisions with their reason, the user's preferences and rules, bugs fixed with the root cause, "
    "non-obvious facts (paths, ports, commands, versions, accounts; never passwords, keys or tokens) and what is "
    "still open. Skip routine steps, file listings and what anyone sees in the code. Each note stands alone: name "
    "the project or tool it is about. Write English, short and concrete. At most %d notes; fewer is fine, none is "
    "fine. Also keep the session record up to date, merged with the record so far: summary = what changed and "
    "where, why = the reasons behind the decisions, rejected = what was tried and dropped and why, next = what is "
    "left open. Say what and why, never how; one short line per point." % D.MEM_NOTES_PER_CHUNK)
RECORD = ("summary", "why", "rejected", "next")
SCHEMA = {
    "type": "object",
    "properties": {
        "title": {"type": "string"},
        "summary": {"type": "string"},
        "why": {"type": "string"},
        "rejected": {"type": "string"},
        "next": {"type": "string"},
        "notes": {"type": "array", "maxItems": D.MEM_NOTES_PER_CHUNK, "items": {
            "type": "object",
            "properties": {"kind": {"type": "string", "enum": KINDS}, "title": {"type": "string"},
                           "body": {"type": "string"}, "files": {"type": "array", "items": {"type": "string"}}},
            "required": ["kind", "title", "body"]}},
    },
    "required": ["title", "summary", "why", "rejected", "next", "notes"],
}
REMINDER = re.compile(r"(?s)<(system-reminder|local-command-caveat)>.*?</\1>")
TAG = re.compile(r"</?[a-zA-Z][\w-]*(?:\s[^<>]*)?>")
_st = {"thread": None, "last": "", "learned": 0, "error": ""}
_lock = threading.Lock()


def cut(text, n):
    s = " ".join(str(text or "").split())
    return s if len(s) <= n else s[:n] + " …"


def clean(text):
    return TAG.sub("", REMINDER.sub("", text or "")).strip()


def tool_line(name, a):
    if name == "Bash" or name == "PowerShell":
        return "%s: %s" % (name, cut(a.get("command"), D.MEM_TOOL_TEXT))
    for k in ("file_path", "path", "pattern", "skill", "url", "query", "prompt"):
        if a.get(k):
            return "%s %s" % (name, cut(a[k], 160))
    return "%s %s" % (name, cut(json.dumps(a, ensure_ascii=False), 160))


def result_text(c):
    v = c.get("content")
    if isinstance(v, list):
        v = " ".join(x.get("text", "") for x in v if isinstance(x, dict))
    return str(v or "")


def condense(path, offset):
    with open(path, "rb") as f:
        f.seek(offset)
        data = f.read()
    end = data.rfind(b"\n") + 1
    parts, title, paths = [], "", []
    for raw in data[:end].splitlines():
        try:
            d = json.loads(raw)
        except ValueError:
            continue
        t = d.get("type")
        if t == "ai-title":
            title = d.get("aiTitle") or title
            continue
        if t not in ("user", "assistant") or d.get("isMeta") or d.get("isSidechain"):
            continue
        content = (d.get("message") or {}).get("content")
        if isinstance(content, str):
            content = [{"type": "text", "text": content}]
        for c in content or []:
            k = c.get("type")
            if k == "text":
                text = clean(c.get("text"))
                if text:
                    parts.append(("USER: " if t == "user" else "CLAUDE: ") + cut(text, D.MEM_TEXT))
            elif k == "tool_use":
                a = c.get("input") or {}
                parts.append("TOOL " + tool_line(c.get("name"), a))
                f = a.get("file_path") or a.get("notebook_path") or a.get("path")
                if isinstance(f, str) and re.match(r"^(?:[A-Za-z]:[\\/]|/)", f):
                    paths += [f] * (3 if c.get("name") in ("Edit", "Write", "NotebookEdit") else 1)
            elif k == "tool_result" and c.get("is_error"):
                parts.append("ERROR: " + cut(result_text(c), D.MEM_TOOL_TEXT))
    condense.paths = paths
    return M.scrub("\n".join(parts)), offset + end, title


def chunks(text):
    out, cur, size = [], [], 0
    for line in text.split("\n"):
        if size + len(line) > D.MEM_CHUNK_CHARS and cur:
            out.append("\n".join(cur))
            cur, size = [], 0
        cur.append(line)
        size += len(line) + 1
    if cur:
        out.append("\n".join(cur))
    return out[-8:]


def parse(text):
    try:
        return json.loads(text)
    except ValueError:
        m = re.search(r"(?s)\{.*\}", text or "")
        if not m:
            raise
        return json.loads(m.group(0))


def learn(row):
    if not Path(row["transcript"]).exists():
        M.write("update sessions set status = 'done' where id = ?", (row["id"],))
        return 0
    text, end, title = condense(row["transcript"], row["offset"])
    sid, project = row["id"], M.project_of_files(condense.paths) or row["project"]
    if project != row["project"]:
        M.write("update sessions set project = ? where id = ?", (project, sid))
    if len(text) < D.MEM_MIN_CHARS:
        M.write("update sessions set offset = ?, learned = ?, status = 'done', title = coalesce(nullif(?, ''), title) "
                "where id = ?", (end, time.time(), title, sid))
        return 0
    summary = dict({k: row[k] or "" for k in RECORD}, title=title or row["title"])
    added = 0
    for part in chunks(text):
        if games.running():
            raise MM.NotReady("a game started")
        so_far = "\n".join("%s: %s" % (k, summary[k]) for k in RECORD if summary[k]) or "-"
        user = "Project: %s\nSession title: %s\nRecord so far:\n%s\n\nSession log:\n%s" % (
            project or "-", summary["title"] or "-", so_far, part)
        out = parse(MM.chat(SYSTEM, user, SCHEMA))
        for n in out.get("notes") or []:
            if n.get("title"):
                kind = n.get("kind") if n.get("kind") in KINDS else "fact"
                _, new = M.merge_note(project, kind, n["title"], n.get("body", ""), n.get("files") or [], sid)
                added += new
        summary = dict({k: M.scrub(str(out.get(k) or "")) for k in RECORD},
                       title=title or out.get("title") or summary["title"])
    M.write("update sessions set offset = ?, learned = ?, status = 'done', tries = 0, title = ?, summary = ?, why = ?, "
            "rejected = ?, next = ? where id = ?", (end, time.time(), M.oneline(summary["title"], 120),
                                                    *[summary[k] for k in RECORD], sid))
    if not vault.write_session(sid, project, summary, row["started"] or time.time()):
        M.session_note(sid, project, summary["title"], summary["summary"], summary["next"])
    _st["learned"] += added
    return added


def embed_some(seconds=50):
    end, n = time.time() + seconds, 0
    while time.time() < end:
        todo = M.unembedded(D.MEM_EMBED_BATCH)
        if not todo or games.running():
            break
        vecs = MM.embed([r["title"] + "\n" + (r["body"] or "") for r in todo])
        if vecs is None:
            break
        M.save_vecs(zip([r["id"] for r in todo], vecs))
        n += len(todo)
    return n


def tick(now=False):
    if not _lock.acquire(blocking=False):
        return
    try:
        work(now)
    finally:
        _lock.release()


def work(now):
    c = D.config()
    if not c.get("memory") or MM.reap():
        return
    try:
        reproject()
        vault.index()
    except (OSError, ValueError) as e:
        _st["error"] = "vault: %s" % e
    due = M.due_sessions(time.time() + D.MEM_QUIET_S if now else None) if c.get("memory_learn") or now else []
    if not due and not M.unembedded(1):
        return
    if not (MM.api("chat") or MM.ready("chat")) or not (MM.api("embed") or MM.ready("embed")):
        if c.get("memory_learn"):
            MM.download_bg()
        return
    for row in due:
        try:
            learn(row)
            _st["last"] = "%s learned %s" % (time.strftime("%H:%M"), row["id"][:8])
        except MM.NotReady as e:
            M.write("update sessions set retry_at = ? where id = ?", (time.time() + D.MEM_POLL_S, row["id"]))
            _st["error"] = str(e)
            return
        except (OSError, ValueError, KeyError, TypeError) as e:
            M.write("update sessions set tries = tries + 1, retry_at = ? where id = ?",
                    (time.time() + D.MEM_RETRY_S, row["id"]))
            _st["error"] = "%s: %s" % (row["id"][:8], e)
    try:
        embed_some()
    except MM.NotReady as e:
        _st["error"] = str(e)


def loop():
    while True:
        time.sleep(D.MEM_POLL_S)
        try:
            tick()
        except Exception as e:
            _st["error"] = str(e)


def start():
    if _st["thread"] is None:
        _st["thread"] = threading.Thread(target=loop, daemon=True)
        _st["thread"].start()


def fold_work_state(entries):
    lists = {}
    for project, name, fields, t in entries:
        try:
            f = json.loads(fields)
        except ValueError:
            continue
        lst = lists.setdefault((project, name), {"closed": False, "items": {}})
        if "task" in f:
            item = lst["items"].setdefault(f["task"], {"t": t})
            item.update({k: v for k, v in f.items() if v is not None})
            item["t"] = t
        elif f.get("status") == "done":
            lst["closed"] = True
    return lists


def reproject():
    if M.meta("projects_v2"):
        return 0
    home, fixes = Path.home().name, []
    for r in M.rows("select id, project, files from notes where source in ('import', 'session', 'user')"):
        try:
            files = json.loads(r["files"] or "[]")
        except ValueError:
            files = []
        project = M.project_of_files(files) or ("" if r["project"] == home else r["project"])
        if project != r["project"]:
            fixes.append((project, r["id"]))
    M.many("update notes set project = ? where id = ?", fixes)
    M.write("update sessions set project = '' where project = ?", (home,))
    M.write("update todos set project = '' where project = ?", (home,))
    M.meta("projects_v2", time.time())
    return len(fixes)


def import_claude_mem(path=None):
    src = Path(path) if path else Path.home() / ".claude-mem" / "claude-mem.db"
    if not src.exists():
        raise MM.NotReady("no claude-mem database at %s" % src)
    db = sqlite3.connect("file:%s?mode=ro" % src.as_posix(), uri=True)
    have = {r[0] for r in M.rows("select origin from notes where origin like 'cm:%' "
                                 "union select origin from todos where origin like 'cm:%'")}
    home = Path.home().name
    notes = []
    for oid, project, kind, title, sub, facts, narr, fr, fm, t in db.execute(
            "select id, project, type, title, subtitle, facts, narrative, files_read, files_modified, created_at_epoch "
            "from observations where title is not null"):
        if "cm:o:%d" % oid in have:
            continue
        try:
            fl = json.loads(facts or "[]")
        except ValueError:
            fl = []
        body = "\n".join([sub or ""] + ["- " + str(x) for x in fl]) if fl else "\n".join([sub or "", narr or ""])
        try:
            files = (json.loads(fm or "[]") + json.loads(fr or "[]"))[:12]
        except ValueError:
            files = []
        when = (t or 0) / 1000
        project = M.project_of_files(files) or ("" if project == home else project)
        notes.append((project or "", kind or "note", M.oneline(M.scrub(title), 200), M.scrub(body.strip()),
                      json.dumps(files)[:2000], "import", "", when, when, "cm:o:%d" % oid))
    sess = []
    for sid, project, req, done, learned, nxt, t in db.execute(
            "select id, project, request, completed, learned, next_steps, created_at_epoch from session_summaries"):
        when = (t or 0) / 1000
        project = "" if project == home else project
        summary = M.scrub(" ".join(x for x in (done, learned) if x))
        if summary and "cm:s:%d" % sid not in have:
            notes.append((project or "", "session", M.oneline(req or "session", 200),
                          summary + ("\nNext: " + M.scrub(nxt) if nxt else ""), "[]", "import", "cm:s:%d" % sid, when,
                          when, "cm:s:%d" % sid))
        if summary:
            sess.append(("cm:s:%d" % sid, project or "", "", "", when, when, M.oneline(req, 120), summary,
                         M.scrub(nxt or ""), "done", "import"))
    M.many("insert into notes(project, kind, title, body, files, source, session, created, updated, origin) "
           "values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", notes)
    M.many("insert or ignore into sessions(id, project, cwd, transcript, started, last, title, summary, next, status, "
           "source) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", sess)
    lists = fold_work_state(db.execute("select project, list_name, fields, created_at_epoch from work_state_entries "
                                       "order by id"))
    todos = 0
    for (project, name), lst in lists.items():
        if lst["closed"]:
            continue
        for task, item in lst["items"].items():
            origin = "cm:w:%s:%s" % (name, task)
            if item.get("status", "todo") in M.OPEN and origin not in have:
                M.todo_add(project or "", task, name, item.get("status", "todo"), M.scrub(str(item.get("note") or "")),
                           (item["t"] or 0) / 1000, origin)
                todos += 1
    db.close()
    M.meta("imported_claude_mem", time.time())
    return len(notes), len(sess), todos


def context_dirs():
    import string
    found = []
    for letter in string.ascii_uppercase:
        drive = Path("%s:/" % letter)
        if D.WIN and drive.exists():
            for pattern in ("*/.claude/memory/sessions", "*/*/.claude/memory/sessions", "*/*/*/.claude/memory/sessions"):
                try:
                    found += [p for p in drive.glob(pattern) if p.is_dir()]
                except OSError:
                    continue
    home = Path.home() / ".claude" / "memory" / "sessions"
    if home.is_dir() and home not in found:
        found.append(home)
    return found


def known_projects():
    names = {r["project"] for r in M.rows("select distinct project from notes where project != ''")}
    return sorted((n for n in names if len(n) >= 4), key=len, reverse=True)


def import_context_engine(dirs=None):
    names, copied = known_projects(), 0
    for d in dirs or context_dirs():
        owner = d.parent.parent.parent
        project = "" if owner == Path.home() else M.project_of(str(owner))
        for f in d.glob("*.md"):
            try:
                text = f.read_text("utf-8", errors="replace")
            except OSError:
                continue
            name = project or next((n for n in names if re.search(r"(?i)(?<![\w-])%s(?![\w-])" % re.escape(n),
                                                                   text[:400])), "")
            dest = vault.root() / "Sessions" / vault.safe_name(name or "general", 80) / f.name
            if dest.exists():
                continue
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(text, "utf-8")
            copied += 1
    return copied


def state():
    return _st
