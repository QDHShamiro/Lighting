import json
import re
import threading
import time
from pathlib import Path

from lighting import defaults as D
from lighting import memory as M
from lighting import vault
from lighting.common import Fail

SAY = re.compile(r"(?is)^\s*(?:bitte\s+|please\s+)?(?:merk(?:e)?\s+dir|notier(?:e)?\s+dir|remember)\b[\s,:;-]*"
                 r"(?:(?:bitte|please)\s+)?(?:dass\s+|that\s+)?(.+)")
TODO_HELP = 'lighting todo | todo add "task" [--list name] | todo done|doing|drop|todo <id> | todo note <id> "text"'
SOURCE_TAGS = {"vault": "vault", "claude": "claude memory"}
MEMORY_HELP = "lighting memory [learn | import claude-mem | model | on | off]"


def project(ctx, flags=None):
    if flags and flags.get("global"):
        return ""
    if flags and flags.get("project"):
        return str(flags["project"])
    return M.project_of(getattr(ctx, "cwd", None) or "")


def ident(text):
    m = re.fullmatch(r"#?(\d+)", str(text or "").strip())
    if not m:
        raise Fail("'%s' is not a note or to-do number" % text, "the number after # in recall or todo")
    return int(m.group(1))


def line(r):
    body = M.oneline((r["body"] or "").split("\n")[0], 70) if r["body"] and r["kind"] != "session" else ""
    where = SOURCE_TAGS.get(r["source"], "")
    head = "#%d %s %s%s%s: %s" % (r["id"], M.day(r["updated"]), r["kind"], " [%s]" % r["project"] if r.get("other") and r["project"] else "",
                                  " (%s)" % where if where else "", M.oneline(r["title"], D.MEM_LINE))
    return head + (" | " + body if body and body.lower() not in head.lower() else "")


def embed_now(ids, texts):
    try:
        from lighting import memmodel as MM
        vecs = MM.embed(texts, start=False, timeout=10)
        if vecs is not None:
            M.save_vecs(zip(ids, vecs))
    except Exception:
        pass


def remember(ctx, text, flags=None, kind=None):
    text = M.scrub(" ".join(str(text or "").split()))
    if len(text) < 4:
        raise Fail("what should Lighting remember?", 'lighting remember "Brave is the only browser"')
    first = re.split(r"(?<=[.!?])\s", text, 1)[0]
    title = first if len(first) <= 160 else M.oneline(text, 160)
    nid = M.add_note(project(ctx, flags), kind or (flags or {}).get("kind") or "fact", title,
                     text if text != title else "", source="user", session=getattr(ctx, "sid", ""))
    threading.Thread(target=embed_now, args=([nid], [text]), daemon=True).start()
    return nid


def cmd_remember(ctx, pos, flags):
    nid = remember(ctx, " ".join(pos), flags)
    return "remembered #%d (lighting forget %d undoes it)" % (nid, nid)


def query_vec(text, start):
    try:
        from lighting import memmodel as MM
        v = MM.embed([text], query=True, start=start, timeout=15 if start else 3)
        return None if v is None else v[0]
    except Exception:
        return None


def cmd_recall(ctx, pos, flags):
    if not pos:
        raise Fail("recall needs words or a number", 'lighting recall "how did we deploy" | lighting recall 42')
    if len(pos) == 1 and re.fullmatch(r"#?\d+", pos[0]):
        r = M.note(ident(pos[0]))
        if not r:
            raise Fail("no note #%s" % pos[0].lstrip("#"), 'lighting recall "<words>"')
        M.touch([r["id"]])
        files = vault.path_of(r) or ", ".join(json.loads(r["files"] or "[]")[:8])
        return "#%d %s %s [%s, %s, %s]\n%s%s%s" % (
            r["id"], time.strftime("%Y-%m-%d", time.localtime(r["updated"])), r["kind"], r["project"] or "global",
            r["source"], r["session"][:8] or "-", r["title"], "\n" + r["body"] if r["body"] else "",
            "\nfiles: " + files if files else "")
    text = " ".join(pos)
    here = project(ctx, flags)
    found = M.search(text, project="" if flags.get("all") else here, limit=int(flags.get("max") or D.MEM_RECALL_MAX),
                     vec=query_vec(text, True))
    if not found:
        return 'nothing remembered about "%s"' % text
    M.touch([r["id"] for r in found])
    for r in found:
        r["other"] = bool(here) and r["project"] != here
    return "\n".join(line(r) for r in found) + "\n(lighting recall <id> shows one in full)"


def cmd_forget(ctx, pos, flags):
    if not pos:
        raise Fail("forget needs a note number", "lighting forget 42")
    try:
        n = M.forget(ident(pos[0]))
    except ValueError as e:
        raise Fail(str(e))
    return "forgot #%s" % pos[0].lstrip("#") if n else "no note #%s" % pos[0].lstrip("#")


def todo_line(t, short=False):
    note_len = 0 if short else 80
    return "#%d %s%s%s" % (t["id"], "[doing] " if t["status"] == "doing" else "",
                           M.oneline(t["task"], 80 if short else D.MEM_LINE),
                           " (%s)" % M.oneline(t["note"], note_len) if t["note"] and note_len else "")


def todo_list(rows, short=False):
    out, last = [], None
    for t in rows:
        if t["list"] != last:
            out.append("%s:" % (t["list"] or "to-dos"))
            last = t["list"]
        out.append("  " + todo_line(t, short))
    return out


def cmd_todo(ctx, pos, flags):
    sub = pos[0].lower() if pos else ""
    here = project(ctx, flags)
    if sub == "add":
        task = " ".join(pos[1:])
        if not task:
            raise Fail("todo add needs the task", TODO_HELP)
        tid = M.todo_add(here, task, flags.get("list") or "", "doing" if flags.get("doing") else "todo")
        return "to-do #%d added%s" % (tid, " to " + flags["list"] if flags.get("list") else "")
    if sub in ("done", "doing", "drop", "dropped", "todo", "open"):
        if len(pos) < 2:
            raise Fail("which to-do?", TODO_HELP)
        status = {"drop": "dropped", "open": "todo"}.get(sub, sub)
        t = M.todo_set(ident(pos[1]), status, " ".join(pos[2:]) or None)
        if not t:
            raise Fail("no to-do #%s" % pos[1], "lighting todo")
        return "#%d is %s: %s" % (t["id"], t["status"], M.oneline(t["task"], 80))
    if sub == "note":
        if len(pos) < 3:
            raise Fail("todo note needs a number and the text", TODO_HELP)
        t = M.todo_set(ident(pos[1]), None, " ".join(pos[2:]))
        if not t:
            raise Fail("no to-do #%s" % pos[1], "lighting todo")
        return "#%d noted" % t["id"]
    if sub:
        raise Fail("unknown todo command '%s'" % sub, TODO_HELP)
    rows = M.todos(None if flags.get("all") else here)
    if flags.get("list"):
        rows = [t for t in rows if t["list"] == flags["list"]]
    return "\n".join(todo_list(rows)) if rows else "no open to-dos (lighting todo add \"task\")"


def project_lines(here):
    r = M.one("select body from notes where source = 'vault' and kind = 'project' and project = ? "
              "order by updated desc limit 1", (here,)) if here else None
    text = [x.strip() for x in (r["body"] if r else "").splitlines() if x.strip() and not x.startswith(("#", "---"))]
    return ["About: " + M.oneline(" ".join(text[:3]), 220)] if text else []


def commits(cwd):
    import subprocess
    try:
        out = subprocess.run(["git", "-C", cwd, "log", "-%d" % D.MEM_START_COMMITS, "--format=%h %ad %s", "--date=format:%m-%d"],
                             capture_output=True, timeout=3, creationflags=0x08000000 if D.WIN else 0)
    except (OSError, subprocess.TimeoutExpired):
        return []
    rows = out.stdout.decode("utf-8", "replace").splitlines() if out.returncode == 0 else []
    return ["Last commits:"] + ["  " + M.oneline(x, 100) for x in rows] if rows else []


def start_text(here, cwd=""):
    c = M.counts()
    lines = ['Lighting memory (%s): lighting recall "<words>" searches %d notes (vault, session notes and Claude '
             'memory included), lighting remember "<fact>" keeps one, lighting todo tracks work.'
             % (here or "no project", c["n"])]
    lines += project_lines(here)
    rows = M.todos(here)[: D.MEM_START_TODOS]
    if rows:
        lines.append("Open to-dos:")
        lines += ["  " + x for x in todo_list(rows, True)]
    recent = M.sessions(here, D.MEM_START_SESSIONS)
    if recent:
        lines.append("Last sessions:")
        for s in recent:
            nxt = " Open: " + M.oneline(s["next"], 90) if s["next"] else ""
            lines.append("  %s %s: %s%s" % (M.day(s["last"]), M.oneline(s["title"] or "-", 50),
                                             M.oneline(s["summary"], 100), nxt))
    if here and cwd:
        lines += commits(cwd)
    rules = M.rows("select id, title from notes where kind = 'rule' and source = 'vault' and project = ? "
                   "order by updated desc limit ?", (here, D.MEM_START_RULES)) if here else []
    if rules:
        lines.append("Project rules:")
        lines += ["  #%d %s" % (r["id"], M.oneline(r["title"], D.MEM_LINE)) for r in rules]
    mine = M.rows("select * from notes where source = 'user' and project in (?, '') order by updated desc limit ?",
                  (here, D.MEM_START_NOTES))
    if mine:
        lines.append("Remembered:")
        lines += ["  #%d %s" % (r["id"], M.oneline(r["title"], D.MEM_LINE)) for r in mine]
    waiting = len(vault.drafts())
    if waiting:
        lines.append("%d wiki drafts wait for a yes or no: lighting wiki" % waiting)
    if c["q"]:
        from lighting import memmodel as MM
        if not (MM.api("chat") or MM.ready("chat")):
            lines.append("(%d sessions wait to be learned: %s)" % (c["q"], MM.status()))
    return "\n".join(lines)


def hook_input(pos):
    try:
        return json.loads(pos[1]) if len(pos) > 1 else {}
    except ValueError:
        return {}


def cmd_hook(ctx, pos, flags):
    if not D.config().get("memory"):
        return ""
    event = pos[0].lower() if pos else ""
    data = hook_input(pos)
    cwd = data.get("cwd") or getattr(ctx, "cwd", None) or ""
    here = M.project_of(cwd)
    sid = data.get("session_id") or getattr(ctx, "sid", "")
    M.session_seen(sid, here, cwd, data.get("transcript_path") or "", ended=event == "end")
    if event == "start":
        from lighting import memlearn
        memlearn.start()
        return start_text(here, cwd)
    if event == "compact":
        from lighting import memlearn
        threading.Thread(target=memlearn.tick, args=(True,), daemon=True).start()
    return ""


def on_prompt(ctx, prompt):
    if not D.config().get("memory") or not prompt:
        return []
    m = SAY.match(prompt)
    if m and len(m.group(1).strip()) >= 6:
        nid = remember(ctx, m.group(1).strip().rstrip("."), kind="preference")
        return ['Lighting memory saved #%d: "%s" (lighting forget %d if that was not meant to be kept)'
                % (nid, M.oneline(m.group(1), 90), nid)]
    from lighting.recall import stem, words
    stems = {stem(w) for w in words(prompt)}
    if len(stems) < 3:
        return []
    here = M.project_of(getattr(ctx, "cwd", None) or "")
    qv = query_vec(prompt, False)
    out = []
    for r in M.search(prompt, project=here, limit=8, vec=qv, min_sim=D.MEM_HINT_SIM):
        key = "m:%d" % r["id"]
        if key in ctx.told or r["kind"] == "session" or r["project"] not in (here, ""):
            continue
        have = {stem(w) for w in words(r["title"] + " " + (r["body"] or ""))}
        hit = len(stems & have)
        words_ok = hit >= D.MEM_HINT_TERMS and hit / len(stems) >= D.MEM_HINT_SHARE
        if words_ok and (qv is None or (r["sim"] or 0) >= D.MEM_HINT_SIM):
            ctx.told[key] = time.time()
            out.append("#%d %s" % (r["id"], M.oneline(r["title"], 100)))
        if len(out) >= D.MEM_HINTS:
            break
    return ["Lighting memory may help (lighting recall <id>): " + " | ".join(out)] if out else []


WIKI_HELP = "lighting wiki | wiki accept <n> [folder] | wiki drop <n> | wiki show <n>"


def cmd_code(ctx, pos, flags):
    from lighting import codemap
    return codemap.cmd_code(ctx, pos, flags)


def cmd_wiki(ctx, pos, flags):
    sub = pos[0].lower() if pos else ""
    found = vault.drafts()
    if not sub:
        if not found:
            return "no drafts wait in %s" % vault.inbox().as_posix()
        lines = ["%d drafts wait in %s (lighting wiki accept <n> | drop <n> | show <n>):"
                 % (len(found), vault.inbox().as_posix())]
        for i, p in enumerate(found, 1):
            meta = vault.front(p.read_text("utf-8", errors="replace"))[0]
            lines.append("  %d %s [%s]: %s" % (i, meta.get("type", "note"), meta.get("project", "-"), p.stem))
        return "\n".join(lines)
    if sub not in ("accept", "drop", "show"):
        raise Fail("unknown wiki command '%s'" % sub, WIKI_HELP)
    if len(pos) < 2 or not pos[1].isdigit() or not 1 <= int(pos[1]) <= len(found):
        raise Fail("which draft? 1 to %d" % len(found) if found else "no drafts wait", WIKI_HELP)
    path = found[int(pos[1]) - 1]
    if sub == "show":
        return path.read_text("utf-8", errors="replace")
    if sub == "drop":
        vault.drop(path)
        return "dropped %s (in the vault's .trash)" % path.name
    try:
        dest = vault.accept(path, " ".join(pos[2:]) or None)
    except (OSError, ValueError) as e:
        raise Fail(str(e), WIKI_HELP)
    return "accepted -> %s (registered in its INDEX)" % dest.relative_to(vault.root()).as_posix()


def cmd_memory(ctx, pos, flags):
    from lighting import memlearn, memmodel as MM
    sub = pos[0].lower() if pos else ""
    if sub in ("on", "off"):
        cfg = D.config()
        cfg["memory"] = sub == "on"
        D.save_config(cfg)
        return "memory %s" % sub
    if sub == "import" and len(pos) >= 2 and pos[1].lower() == "context-engine":
        n = memlearn.import_context_engine([Path(flags["dir"])] if flags.get("dir") else None)
        return "copied %d Context Engine session notes into %s/Sessions (the originals stay)" % (
            n, vault.root().as_posix())
    if sub == "import":
        if len(pos) < 2 or pos[1].lower() != "claude-mem":
            raise Fail("import from where?", "lighting memory import claude-mem [--file path] | import context-engine")
        n, s, t = memlearn.import_claude_mem(flags.get("file"))
        return "imported %d notes, %d session summaries, %d open to-dos from claude-mem (vectors follow in the " \
               "background)" % (n, s, t)
    if sub == "model":
        started = MM.download_bg() if not (MM.ready("chat") and MM.ready("embed")) else False
        return MM.status() + (" | download started" if started else "")
    if sub == "learn":
        M.write("update sessions set retry_at = 0, tries = 0 where status = 'open' and transcript != ''")
        memlearn.start()
        threading.Thread(target=memlearn.tick, args=(True,), daemon=True).start()
        return "learning %d sessions in the background (lighting memory shows the state)" % M.counts()["q"]
    if sub:
        raise Fail("unknown memory command '%s'" % sub, MEMORY_HELP)
    c, st = M.counts(), memlearn.state()
    return "\n".join(x for x in [
        "memory %s: %d notes (%d with vectors), %d open to-dos, %d session summaries, %d sessions to learn" % (
            "on" if D.config().get("memory") else "off", c["n"], c["v"], c["t"], c["s"], c["q"]),
        "model: " + MM.status(),
        "last: %s" % st["last"] if st["last"] else "",
        "problem: %s" % st["error"] if st["error"] else "",
        "kept on this PC only (%s) | %s" % (D.MEMORY_DB.as_posix(), MEMORY_HELP)] if x)
