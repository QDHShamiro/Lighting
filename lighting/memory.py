import functools
import json
import re
import sqlite3
import threading
import time
from pathlib import Path

from lighting import defaults as D

SCHEMA = """
create table if not exists vecs(id integer primary key, v blob);
create table if not exists notes(id integer primary key, project text, kind text, title text, body text,
  files text default '', source text, session text default '', created real, updated real, hits integer default 0,
  origin text default '');
create index if not exists notes_project on notes(project, updated);
create index if not exists notes_origin on notes(origin);
create virtual table if not exists notes_fts using fts5(title, body, content='notes', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2', prefix='3');
create trigger if not exists notes_ai after insert on notes begin
  insert into notes_fts(rowid, title, body) values (new.id, new.title, new.body); end;
create trigger if not exists notes_ad after delete on notes begin
  insert into notes_fts(notes_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
  delete from vecs where id = old.id; end;
create trigger if not exists notes_au after update of title, body on notes begin
  insert into notes_fts(notes_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
  insert into notes_fts(rowid, title, body) values (new.id, new.title, new.body);
  delete from vecs where id = old.id; end;
create table if not exists sessions(id text primary key, project text, cwd text, transcript text default '',
  started real, last real, ended integer default 0, offset integer default 0, learned real default 0,
  title text default '', summary text default '', next text default '', status text default 'open',
  tries integer default 0, retry_at real default 0, source text default 'live', why text default '',
  rejected text default '');
create index if not exists sessions_project on sessions(project, last);
create table if not exists todos(id integer primary key, project text, list text default '', task text,
  status text default 'todo', note text default '', created real, updated real, origin text default '');
create index if not exists todos_project on todos(project, status);
create table if not exists meta(k text primary key, v text);
"""
MIGRATE = ["alter table sessions add column why text default ''",
           "alter table sessions add column rejected text default ''"]
OPEN = ("todo", "doing")
TOKEN = re.compile(r"[^\W_][\w.-]*", re.U)
SECRET = [
    re.compile(r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}"),
    re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}"),
    re.compile(r"\bgsk_[A-Za-z0-9]{20,}"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    re.compile(r"\bxox[abpr]-[A-Za-z0-9-]{10,}"),
    re.compile(r"\beyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
    re.compile(r"(?i)((?:password|passwort|passwd|secret|token|api[_-]?key|license[_-]?key)\s*[:=]\s*)\S{6,}"),
]
_lock = threading.RLock()
_db = {"conn": None, "ids": None, "mat": None}


def conn():
    if _db["conn"] is None:
        D.MEMORY.mkdir(parents=True, exist_ok=True)
        c = sqlite3.connect(str(D.MEMORY_DB), check_same_thread=False, timeout=15)
        c.row_factory = sqlite3.Row
        c.execute("pragma journal_mode=wal")
        c.execute("pragma synchronous=normal")
        c.executescript(SCHEMA)
        for sql in MIGRATE:
            try:
                c.execute(sql)
            except sqlite3.OperationalError:
                pass
        _db["conn"] = c
    return _db["conn"]


def rows(sql, args=()):
    with _lock:
        return conn().execute(sql, args).fetchall()


def one(sql, args=()):
    found = rows(sql, args)
    return found[0] if found else None


def write(sql, args=()):
    with _lock:
        c = conn()
        cur = c.execute(sql, args)
        c.commit()
        return cur.lastrowid


def many(sql, seq):
    with _lock:
        c = conn()
        c.executemany(sql, seq)
        c.commit()


def meta(key, value=None):
    if value is None:
        r = one("select v from meta where k = ?", (key,))
        return r["v"] if r else None
    write("insert into meta(k, v) values (?, ?) on conflict(k) do update set v = excluded.v", (key, str(value)))
    return value


_roots = {}


def built(d):
    try:
        return any((d / f).exists() for f in D.BUILD_FILES)
    except OSError:
        return False


def monorepo(root):
    if root not in _roots:
        try:
            _roots[root] = sum(1 for c in root.iterdir() if c.is_dir() and built(c)) >= D.MONOREPO_MIN
        except OSError:
            _roots[root] = False
    return _roots[root]


@functools.lru_cache(maxsize=8192)
def project_of(cwd):
    if not cwd:
        return ""
    p = Path(cwd)
    home = Path.home()
    chain = [p, *p.parents]
    root = None
    for d in chain:
        if d == home:
            break
        try:
            if (d / ".git").exists():
                root = d
                break
        except OSError:
            break
    if root is None:
        if p == home:
            return ""
        found = next((d for d in chain if d != home and built(d)), p)
        return found.name or str(found)
    if p != root and monorepo(root):
        child = root / p.relative_to(root).parts[0]
        if built(child):
            return child.name
    return root.name or str(root)


def project_of_files(paths):
    seen = {}
    for f in paths:
        name = project_of(str(Path(f).parent)) if f else ""
        if name:
            seen[name] = seen.get(name, 0) + 1
    return max(seen, key=seen.get) if seen else ""


def scrub(text):
    out = text or ""
    for rx in SECRET:
        out = rx.sub(lambda m: (m.group(1) if m.groups() else "") + "[secret]", out)
    return out


def oneline(text, n=D.MEM_LINE):
    s = " ".join(str(text or "").split())
    return s if len(s) <= n else s[: n - 1].rstrip() + "…"


def day(t):
    return time.strftime("%m-%d", time.localtime(t or 0))


def norm_title(text):
    return {w.strip(".-") for w in TOKEN.findall((text or "").lower()) if len(w.strip(".-")) > 2}


def similar(a, b):
    x, y = norm_title(a), norm_title(b)
    return len(x & y) / max(1, len(x | y))


def add_note(project, kind, title, body="", files=(), source="user", session="", when=None, origin=""):
    now = when or time.time()
    title = oneline(scrub(title), 200)
    body = scrub(body or "").strip()
    if not title:
        return None
    return write("insert into notes(project, kind, title, body, files, source, session, created, updated, origin) "
                 "values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                 (project, kind or "note", title, body, json.dumps(list(files or []))[:2000], source, session, now, now,
                  origin))


def merge_note(project, kind, title, body, files=(), session=""):
    for r in search(title, project=project, limit=3, fts_only=True):
        if r["project"] == project and r["source"] not in ("vault", "claude") and similar(r["title"], title) >= D.MEM_DUP_SIM:
            text = body if len(body or "") > len(r["body"] or "") else r["body"]
            write("update notes set title = ?, body = ?, kind = ?, updated = ?, session = ? where id = ?",
                  (oneline(scrub(title), 200), scrub(text), kind or r["kind"], time.time(), session, r["id"]))
            return r["id"], False
    return add_note(project, kind, title, body, files, "session", session), True


def session_note(sid, project, title, summary, nxt="", when=None, origin=None):
    origin = origin or "s:" + sid
    body = (summary or "") + ("\nNext: " + nxt if nxt else "")
    r = one("select id from notes where origin = ?", (origin,))
    if r:
        write("update notes set title = ?, body = ?, updated = ? where id = ?",
              (oneline(title or "session", 200), scrub(body), when or time.time(), r["id"]))
        return r["id"]
    return add_note(project, "session", title or "session", body, source="session", session=sid, when=when,
                    origin=origin)


def note(nid):
    return one("select * from notes where id = ?", (nid,))


def drop_cache():
    _db["ids"] = None


def forget(nid):
    r = note(nid)
    if r and r["source"] in ("vault", "claude"):
        raise ValueError("#%d is the file %s: edit or delete the file itself" % (nid, (r["origin"] or "").split(":", 1)[-1]))
    with _lock:
        c = conn()
        n = c.execute("delete from notes where id = ?", (nid,)).rowcount
        c.commit()
    _db["ids"] = None
    return n


def fts_query(text):
    from lighting.recall import STOP
    toks, seen = [], set()
    for t in TOKEN.findall((text or "").lower()):
        t = t.strip(".-")
        if len(t) < 2 or t in STOP or t in seen:
            continue
        seen.add(t)
        toks.append('"%s"*' % t.replace('"', "") if len(t) >= 4 else '"%s"' % t.replace('"', ""))
    return " OR ".join(toks[:16])


def save_vecs(pairs):
    import numpy as np
    many("insert or replace into vecs(id, v) values (?, ?)",
         [(i, np.asarray(v, dtype=np.float16).tobytes()) for i, v in pairs])
    _db["ids"] = None


def matrix():
    import numpy as np
    if _db["ids"] is None:
        found = rows("select id, v from vecs")
        _db["ids"] = np.array([r["id"] for r in found], dtype=np.int64)
        _db["mat"] = (np.frombuffer(b"".join(r["v"] for r in found), dtype=np.float16).astype(np.float32)
                      .reshape(-1, D.MEM_VEC_DIM)) if found else np.zeros((0, D.MEM_VEC_DIM), dtype=np.float32)
    return _db["ids"], _db["mat"]


def search(text, project="", limit=D.MEM_RECALL_MAX, vec=None, fts_only=False, min_sim=0.0):
    score, sims = {}, {}
    fq = fts_query(text)
    if fq:
        try:
            found = rows("select rowid from notes_fts where notes_fts match ? order by bm25(notes_fts, 3.0, 1.0) "
                         "limit 60", (fq,))
        except sqlite3.OperationalError:
            found = []
        for i, r in enumerate(found):
            score[r[0]] = score.get(r[0], 0.0) + 1.0 / (D.MEM_RRF_K + i + 1)
    if vec is not None and not fts_only:
        import numpy as np
        ids, mat = matrix()
        if len(ids):
            sim = mat @ vec
            for i, j in enumerate(np.argsort(-sim)[:60]):
                if sim[j] < min_sim:
                    break
                nid = int(ids[j])
                sims[nid] = float(sim[j])
                score[nid] = score.get(nid, 0.0) + 1.0 / (D.MEM_RRF_K + i + 1)
    if not score:
        return []
    marks = ",".join("?" * len(score))
    found = rows("select * from notes where id in (%s)" % marks, list(score))
    now, out = time.time(), []
    for r in found:
        s = score[r["id"]]
        if project and r["project"] == project:
            s *= D.MEM_PROJECT_BOOST
        if r["source"] == "user":
            s *= 1.2
        s *= 1.0 + 0.25 * 0.5 ** (max(0.0, now - (r["updated"] or 0)) / (90 * 86400))
        out.append((s, sims.get(r["id"]), r))
    out.sort(key=lambda x: -x[0])
    return [dict(r, score=s, sim=sim) for s, sim, r in out[:limit]]


def touch(ids):
    if ids:
        many("update notes set hits = hits + 1 where id = ?", [(i,) for i in ids])


def unembedded(limit):
    return rows("select n.id, n.title, n.body from notes n left join vecs v on v.id = n.id where v.id is null "
                "order by n.source = 'user' desc, n.updated desc limit ?", (limit,))


def todo_add(project, task, lst="", status="todo", note_text="", when=None, origin=""):
    now = when or time.time()
    return write("insert into todos(project, list, task, status, note, created, updated, origin) "
                 "values (?, ?, ?, ?, ?, ?, ?, ?)", (project, lst, oneline(task, 300), status, note_text, now, now, origin))


def todo_set(tid, status=None, note_text=None):
    r = one("select * from todos where id = ?", (tid,))
    if not r:
        return None
    write("update todos set status = ?, note = ?, updated = ? where id = ?",
          (status or r["status"], r["note"] if note_text is None else note_text, time.time(), tid))
    return one("select * from todos where id = ?", (tid,))


def todos(project=None, open_only=True):
    sql, args = "select * from todos where 1 = 1", []
    if project:
        sql += " and project = ?"
        args.append(project)
    if open_only:
        sql += " and status in ('todo', 'doing')"
    return rows(sql + " order by list, status = 'doing' desc, id", args)


def session_seen(sid, project, cwd, transcript, ended=False):
    if not sid:
        return
    now = time.time()
    write("insert into sessions(id, project, cwd, transcript, started, last) values (?, ?, ?, ?, ?, ?) "
          "on conflict(id) do update set last = excluded.last, transcript = coalesce(nullif(excluded.transcript, ''), "
          "sessions.transcript), status = case when sessions.status = 'done' then 'open' else sessions.status end",
          (sid, project, cwd, transcript or "", now, now))
    if ended:
        write("update sessions set ended = 1 where id = ?", (sid,))


def due_sessions(now=None):
    now = now or time.time()
    return rows("select * from sessions where status = 'open' and transcript != '' and tries < ? and retry_at <= ? "
                "and (ended = 1 or last < ?) order by last", (D.MEM_MAX_TRIES, now, now - D.MEM_QUIET_S))


def sessions(project=None, limit=5):
    sql, args = "select * from sessions where summary != ''", []
    if project:
        sql += " and project = ?"
        args.append(project)
    return rows(sql + " order by last desc limit ?", args + [limit])


def counts():
    r = one("select (select count(*) from notes) n, (select count(*) from vecs) v, "
            "(select count(*) from todos where status in ('todo', 'doing')) t, "
            "(select count(*) from sessions where summary != '') s, "
            "(select count(*) from sessions where status = 'open' and transcript != '') q")
    return dict(r)
