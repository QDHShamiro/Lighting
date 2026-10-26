import json
import re
import shutil
import sqlite3
import threading
import time
from pathlib import Path

from lighting import defaults as D
from lighting import games
from lighting import memmodel as MM
from lighting import memory as M
from lighting import vault

KINDS = ["decision", "preference", "bugfix", "feature", "fact", "discovery", "change"]
SYSTEM = (
    "You keep the long-term memory of a coding assistant. From the session log, keep only what helps in a FUTURE "
    "session: decisions with their reason, the user's preferences and rules, bugs fixed with the root cause, "
    "non-obvious facts (paths, ports, commands, versions, accounts; never passwords, keys or tokens). Skip routine "
    "steps, file listings, what anyone sees in the code, progress and status (counts, timings, test or CI runs, "
    "what is running right now) and plans; what is still open goes into next, never into a note. Each note stands "
    "alone: its title starts with the project or tool, like 'Lighting: hints need words and meaning'. Write "
    "English, short and concrete. At most %d notes; fewer is fine, none is fine. For the session record write only "
    "what THIS part of the log adds, one short line per point, what and why, never how: summary = what changed "
    "and where, why = the reasons behind the decisions, rejected = what was tried and dropped and why, next = what "
    "is left open at the end of this part. lessons = at most 2 things worth a permanent page in the user's "
    "knowledge base: mistake = an error with its root cause and the fix, rule = a fixed rule the user gave, "
    "pattern = a reusable way of doing something; none is fine." % D.MEM_NOTES_PER_CHUNK)
DRAFT_SYSTEM = (
    "Write one page for the user's knowledge base from the note and the session context. Fill each section with "
    "1 to 4 short lines in the language of the section headings; leave a section empty when the note says nothing "
    "for it. Concrete names, versions and commands, never passwords, keys or tokens. The title says the lesson in "
    "a few words.")
RECORD = ("summary", "why", "rejected", "next")
GENERIC = {"assets", "config", "commands", "claude", "desktop", "content", "bilder", "scripts", "src", "data", "docs",
           "tools", "test", "tests", "temp", "plugins", "server", "build", "public", "notes", "mod", "host", "shim",
           "readme", "pluginname", "store", "stores", "reel", "index"}
EMPTY = {"none", "nothing", "n/a", "na", "keine", "keiner", "nichts", "-", "--"}
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
        "lessons": {"type": "array", "maxItems": 2, "items": {
            "type": "object",
            "properties": {"kind": {"type": "string", "enum": list(vault.WIKI_KINDS)}, "title": {"type": "string"},
                           "body": {"type": "string"}},
            "required": ["kind", "title", "body"]}},
    },
    "required": ["title", "summary", "why", "rejected", "next", "notes", "lessons"],
}
REMINDER = re.compile(r"(?s)<(system-reminder|local-command-caveat)>.*?</\1>")
STILL_OPEN = "case when last > ? then 'open' else 'done' end"
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


def condense(path, offset, limit=None):
    limit = limit or D.MEM_CHUNK_CHARS * D.MEM_CHUNKS
    parts, title, paths, size, pos, more = [], "", [], 0, offset, False
    with open(path, "rb") as fh:
        fh.seek(offset)
        for raw in fh:
            if not raw.endswith(b"\n"):
                break
            if size >= limit:
                more = True
                break
            pos += len(raw)
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
                k, piece = c.get("type"), ""
                if k == "text":
                    text = clean(c.get("text"))
                    if text:
                        piece = ("USER: " if t == "user" else "CLAUDE: ") + cut(text, D.MEM_TEXT)
                elif k == "tool_use":
                    a = c.get("input") or {}
                    piece = "TOOL " + tool_line(c.get("name"), a)
                    f = a.get("file_path") or a.get("notebook_path") or a.get("path")
                    if isinstance(f, str) and re.match(r"^(?:[A-Za-z]:[\\/]|/)", f):
                        paths += [f] * (3 if c.get("name") in ("Edit", "Write", "NotebookEdit") else 1)
                elif k == "tool_result" and c.get("is_error"):
                    piece = "ERROR: " + cut(result_text(c), D.MEM_TOOL_TEXT)
                if piece:
                    parts.append(piece)
                    size += len(piece) + 1
    condense.paths, condense.more = paths, more
    return M.scrub("\n".join(parts)), pos, title


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
    return out


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
    began = time.time()
    text, end, title = condense(row["transcript"], row["offset"])
    more = condense.more
    sid, project = row["id"], M.project_of_files(condense.paths) or row["project"]
    if project != row["project"]:
        M.write("update sessions set project = ? where id = ?", (project, sid))
    if len(text) < D.MEM_MIN_CHARS:
        M.write("update sessions set offset = ?, learned = ?, status = " + STILL_OPEN + ", "
                "title = coalesce(nullif(?, ''), title) where id = ?", (end, time.time(), began, title, sid))
        return 0
    summary = dict({k: row[k] or "" for k in RECORD}, title=title or row["title"])
    added, wiki = 0, []
    for part in chunks(text):
        if games.running():
            raise MM.NotReady("a game started")
        user = "Project: %s\nSession title: %s\n\nSession log:\n%s" % (project or "-", summary["title"] or "-", part)
        out = parse(MM.chat(SYSTEM, user, SCHEMA))
        for n in out.get("notes") or []:
            if n.get("title"):
                kind = n.get("kind") if n.get("kind") in KINDS else "fact"
                _, new = M.merge_note(project, kind, titled(project, n["title"]), n.get("body", ""),
                                      n.get("files") or [], sid)
                added += new
        wiki += [n for n in out.get("lessons") or [] if n.get("kind") in vault.WIKI_KINDS and n.get("title")]
        for k in ("summary", "why", "rejected"):
            summary[k] = points(summary[k], out.get(k))
        summary["next"] = points("", out.get("next")) or summary["next"]
        summary["title"] = title or summary["title"] or out.get("title") or ""
    for n in wiki[: D.WIKI_PER_SESSION]:
        draft(n, project, sid, summary)
    M.write("update sessions set offset = ?, learned = ?, status = " + STILL_OPEN + ", tries = 0, title = ?, summary = ?, "
            "why = ?, rejected = ?, next = ? where id = ?", (end, time.time(), 0 if more else began,
                                                             M.oneline(summary["title"], 120),
                                                             *[summary[k] for k in RECORD], sid))
    if not vault.write_session(sid, project, summary, row["started"] or time.time()):
        M.session_note(sid, project, summary["title"], summary["summary"], summary["next"])
    _st["learned"] += added
    return added


def titled(project, title):
    title = title.strip()
    if not project or title.lower().startswith(project.lower()):
        return title
    return "%s: %s" % (project, title)


def points(old, new):
    lines = [x.strip().lstrip("-* ").strip() for x in (old or "").splitlines() if x.strip()]
    seen = {frozenset(M.norm_title(x)) for x in lines}
    for x in str(new or "").splitlines():
        x = M.scrub(x.strip().lstrip("-* ").strip())
        key = frozenset(M.norm_title(x))
        if x and x.rstrip(".!").lower() not in EMPTY and key not in seen:
            seen.add(key)
            lines.append(x)
    return "\n".join(lines[-D.RECORD_LINES:])


def draft(n, project, sid, record):
    for r in M.search(n["title"], project=project, limit=5, fts_only=True):
        if r["source"] == "vault" and M.similar(r["title"], n["title"]) >= D.MEM_DUP_SIM:
            return None
    heads = vault.template(n["kind"])[1]
    schema = {"type": "object", "required": ["title", "sections"], "properties": {
        "title": {"type": "string"},
        "sections": {"type": "object", "required": heads, "properties": {h: {"type": "string"} for h in heads}}}}
    user = "Project: %s\nKind: %s\nLesson: %s\n%s\n\nSession: %s\n%s" % (
        project or "-", n["kind"], n["title"], n.get("body", ""), record.get("title", ""), record.get("summary", ""))
    try:
        out = parse(MM.chat(DRAFT_SYSTEM, user, schema))
    except (ValueError, KeyError, TypeError):
        return None
    sections = {h: M.scrub(str(v)) for h, v in (out.get("sections") or {}).items()}
    return vault.write_draft(n["kind"], M.oneline(M.scrub(out.get("title") or n["title"]), 100), sections, project, sid)


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
    from lighting import codemap
    if not due and not M.unembedded(1) and not codemap.active_roots():
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
        if not embed_some():
            code_some()
    except MM.NotReady as e:
        _st["error"] = str(e)


def code_some():
    from lighting import codemap
    for root in codemap.active_roots():
        if games.running():
            return
        codemap.refresh(root, D.CODE_BG_REFRESH_S)
        if codemap.embed_some(root, 40):
            return


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


def claude_mem_projects():
    src = Path.home() / ".claude-mem" / "claude-mem.db"
    if not src.exists():
        return {}
    db = sqlite3.connect("file:%s?mode=ro" % src.as_posix(), uri=True)
    try:
        return {"cm:o:%d" % i: p or "" for i, p in db.execute("select id, project from observations")}
    except sqlite3.Error:
        return {}
    finally:
        db.close()


def reproject():
    if M.meta("projects_v4"):
        return 0
    home, fixes, first = Path.home().name, [], claude_mem_projects()
    for r in M.rows("select id, project, files, origin from notes where source in ('import', 'session', 'user')"):
        try:
            files = json.loads(r["files"] or "[]")
        except ValueError:
            files = []
        was = first.get(r["origin"], r["project"])
        project = M.project_of_files(files) or ("" if was == home else was)
        if project != r["project"]:
            fixes.append((project, r["id"]))
    M.many("update notes set project = ? where id = ?", fixes)
    M.write("update sessions set project = '' where project = ?", (home,))
    M.write("update todos set project = '' where project = ?", (home,))
    M.meta("projects_v4", time.time())
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
    pages = {r["project"] for r in M.rows("select distinct project from notes where source = 'vault' and kind = 'project'")}
    busy = {r["project"] for r in M.rows("select project from notes where project != '' group by project "
                                         "having count(*) >= ?", (D.KNOWN_PROJECT_NOTES,))}
    names = {n for n in pages | busy if len(n) >= 4 and re.search(r"[A-Za-z]{3}", n) and n.lower() not in GENERIC}
    return sorted(names, key=len, reverse=True)


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
            shutil.copy2(f, dest)
            copied += 1
    return copied


def state():
    return _st
