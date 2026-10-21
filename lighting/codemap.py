import hashlib
import os
import re
import sqlite3
import subprocess
import time
from pathlib import Path

from lighting import defaults as D
from lighting import memory as M
from lighting.common import Fail

LANGS = {".java": "java", ".py": "python", ".rs": "rust", ".ts": "typescript", ".tsx": "tsx", ".js": "javascript",
         ".mjs": "javascript", ".cjs": "javascript", ".jsx": "javascript", ".cs": "csharp", ".go": "go",
         ".kt": "kotlin", ".kts": "kotlin", ".cpp": "cpp", ".cc": "cpp", ".hpp": "cpp", ".h": "c", ".c": "c",
         ".php": "php", ".rb": "ruby", ".lua": "lua", ".swift": "swift", ".scala": "scala", ".dart": "dart"}
DEF = re.compile(r"^(?:function|method|class|interface|enum|struct|trait|record|module|constructor|type_alias|"
                 r"object|protocol|annotation_type|union|namespace|mod)_(?:definition|declaration|item|signature)$|"
                 r"^(?:function_item|struct_item|enum_item|trait_item|mod_item|type_item|macro_definition)$")
CALLS = {"call", "call_expression", "method_invocation", "invocation_expression", "macro_invocation",
         "object_creation_expression", "new_expression", "function_call", "method_call", "method_call_expression"}
CALLEE_FIELDS = ("name", "function", "type", "constructor", "method", "macro")
SKIP_DIRS = {".git", "node_modules", "target", "build", "out", "dist", ".gradle", ".idea", "venv", ".venv",
             "__pycache__", ".next", ".graphify", "vendor", "bin", "obj"}
SCHEMA = """
create table if not exists files(path text primary key, mtime real, lang text, lines integer);
create table if not exists symbols(id integer primary key, path text, name text, low text, kind text, line integer,
  end_line integer, parent text, sig text);
create index if not exists symbols_low on symbols(low);
create index if not exists symbols_path on symbols(path);
create table if not exists refs(path text, name text, low text, line integer, caller text);
create index if not exists refs_low on refs(low);
create index if not exists refs_path on refs(path);
create table if not exists vecs(id integer primary key, v blob);
"""
_parsers = {}
_dbs = {}


def root_for(cwd):
    p = Path(cwd)
    home = Path.home()
    for d in [p, *p.parents]:
        if d == home:
            break
        if (d / ".git").exists():
            if p != d and M.monorepo(d):
                child = d / p.relative_to(d).parts[0]
                if M.built(child):
                    return child
            return d
    found = next((d for d in [p, *p.parents] if d != home and M.built(d)), None)
    return found


def db(root):
    key = str(root).lower()
    if key not in _dbs:
        D.CODE.mkdir(parents=True, exist_ok=True)
        name = "%s-%s.db" % (re.sub(r"\W+", "_", root.name)[:40], hashlib.sha1(key.encode()).hexdigest()[:8])
        c = sqlite3.connect(str(D.CODE / name), check_same_thread=False, timeout=15)
        c.row_factory = sqlite3.Row
        c.execute("pragma journal_mode=wal")
        c.executescript(SCHEMA)
        _dbs[key] = c
    return _dbs[key]


def parser(lang):
    if lang not in _parsers:
        try:
            from tree_sitter_language_pack import get_parser
            _parsers[lang] = get_parser(lang)
        except Exception:
            _parsers[lang] = None
    return _parsers[lang]


def listing(root):
    try:
        out = subprocess.run(["git", "-C", str(root), "ls-files", "-co", "--exclude-standard", "."],
                             capture_output=True, timeout=20, creationflags=0x08000000 if D.WIN else 0)
        if out.returncode == 0:
            names = out.stdout.decode("utf-8", "replace").splitlines()
            return [root / n for n in names if Path(n).suffix.lower() in LANGS
                    and not any(part in SKIP_DIRS for part in n.split("/")[:-1])]
    except (OSError, subprocess.TimeoutExpired):
        pass
    found = []
    for base, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS and not d.startswith(".")]
        found += [Path(base) / n for n in names if Path(n).suffix.lower() in LANGS]
    return found


def text_of(node):
    return node.text.decode("utf-8", "replace") if node is not None and node.text else ""


def name_of(node):
    n = node.child_by_field_name("name")
    if n is None:
        n = next((c for c in node.children if c.type in ("identifier", "type_identifier", "simple_identifier",
                                                         "constant", "property_identifier")), None)
    return text_of(n)


def callee(node):
    for field in CALLEE_FIELDS:
        n = node.child_by_field_name(field)
        if n is not None:
            names = re.findall(r"[A-Za-z_]\w*", text_of(n))
            return names[-1] if names else ""
    return ""


def extract(data, lang):
    p = parser(lang)
    if p is None:
        return [], []
    tree = p.parse(data)
    defs, refs, stack = [], [], [(tree.root_node, "")]
    while stack:
        node, owner = stack.pop()
        here = owner
        if DEF.match(node.type):
            name = name_of(node)
            if name:
                first = text_of(node).split("\n", 1)[0].strip()
                defs.append((name, node.type.split("_")[0], node.start_point[0] + 1, node.end_point[0] + 1, owner,
                             first[:160]))
                here = name
        elif node.type in CALLS:
            name = callee(node)
            if name:
                refs.append((name, node.start_point[0] + 1, owner))
        stack += [(c, here) for c in reversed(node.children)]
    return defs, refs


def refresh(root, budget_s=None):
    c = db(root)
    have = {r["path"]: r["mtime"] for r in c.execute("select path, mtime from files")}
    seen, changed, t0 = set(), 0, time.time()
    for f in listing(root):
        rel = f.relative_to(root).as_posix()
        seen.add(rel)
        try:
            st = f.stat()
        except OSError:
            continue
        if have.get(rel) == st.st_mtime or st.st_size > D.CODE_MAX_BYTES:
            continue
        try:
            data = f.read_bytes()
        except OSError:
            continue
        defs, refs = extract(data, LANGS[f.suffix.lower()])
        c.execute("delete from symbols where path = ?", (rel,))
        c.execute("delete from refs where path = ?", (rel,))
        c.executemany("insert into symbols(path, name, low, kind, line, end_line, parent, sig) values (?, ?, ?, ?, ?, ?, ?, ?)",
                      [(rel, n, n.lower(), k, a, b, o, s) for n, k, a, b, o, s in defs])
        c.executemany("insert into refs(path, name, low, line, caller) values (?, ?, ?, ?, ?)",
                      [(rel, n, n.lower(), a, o) for n, a, o in refs])
        c.execute("insert or replace into files(path, mtime, lang, lines) values (?, ?, ?, ?)",
                  (rel, st.st_mtime, LANGS[f.suffix.lower()], data.count(b"\n") + 1))
        changed += 1
        if budget_s and time.time() - t0 > budget_s:
            break
    gone = [p for p in have if p not in seen]
    for p in gone:
        for table in ("files", "symbols", "refs"):
            c.execute("delete from %s where path = ?" % table, (p,))
    c.execute("delete from vecs where id not in (select id from symbols)")
    c.commit()
    return changed + len(gone)


def overview(root):
    c = db(root)
    langs = c.execute("select lang, count(*) n, sum(lines) l from files group by lang order by l desc").fetchall()
    if not langs:
        return "no code files in %s" % root.as_posix()
    lines = ["%s: %s" % (root.as_posix(), ", ".join("%d %s files (%d lines)" % (r["n"], r["lang"], r["l"] or 0)
                                                    for r in langs[:5]))]
    paths = [r["path"].split("/")[:-1] for r in c.execute("select path from files")]
    common = 0
    while paths and all(len(p) > common for p in paths) and len({p[common] for p in paths}) == 1:
        common += 1
    tops = {}
    for p in paths:
        key = "/".join(p[common:common + 2]) or "."
        tops[key] = tops.get(key, 0) + 1
    base = "/".join(paths[0][:common]) + "/" if common else ""
    lines.append("folders%s: %s" % (" under " + base if base else "", ", ".join(
        "%s (%d)" % kv for kv in sorted(tops.items(), key=lambda kv: -kv[1])[:D.CODE_FOLDERS])))
    top = c.execute("select s.name, s.path, s.line, count(*) n, (select count(*) from symbols x where x.low = s.low "
                    "and x.kind = s.kind) same from symbols s join refs r on r.low = s.low "
                    "where s.kind in ('class', 'interface', 'struct', 'enum', 'trait', 'record', 'object') "
                    "and length(s.name) > 2 group by s.low order by n desc limit ?", (D.CODE_TOP,)).fetchall()
    if top:
        lines.append("most used types: " + ", ".join(
            "%s (%s)" % (r["name"], "%d classes with this name" % r["same"] if r["same"] > 1 else "%s:%d" % (r["path"], r["line"]))
            for r in top))
    mains = c.execute("select path, line from symbols where low = 'main' limit 5").fetchall()
    if mains:
        lines.append("entry points: " + ", ".join("%s:%d" % (r["path"], r["line"]) for r in mains))
    return "\n".join(lines)


def lookup(root, name, limit=D.CODE_LIMIT):
    c = db(root)
    low = name.lower()
    defs = c.execute("select * from symbols where low = ? order by kind != 'class', path limit ?", (low, limit)).fetchall()
    fuzzy = False
    if not defs:
        defs = c.execute("select * from symbols where low like ? order by length(name), path limit ?",
                         ("%" + low.replace("%", "") + "%", limit)).fetchall()
        fuzzy = bool(defs)
    if not defs:
        return 'nothing named "%s" in %s (lighting code "<what it does>" searches by meaning)' % (name, root.name)
    lines = [("no exact match, close names:" if fuzzy else "defined:")]
    lines += ["  %s:%d %s %s%s" % (r["path"], r["line"], r["kind"], r["name"], " in " + r["parent"] if r["parent"] else "")
              for r in defs]
    if not fuzzy:
        lines += callers_lines(c, low, D.CODE_CALLERS)
    return "\n".join(lines)


def callers_lines(c, low, limit):
    rows = c.execute("select path, line, caller from refs where low = ? order by path, line limit ?",
                     (low, limit + 1)).fetchall()
    total = c.execute("select count(*) from refs where low = ?", (low,)).fetchone()[0]
    if not rows:
        return ["  no calls found"]
    out = ["called from (%d):" % total]
    out += ["  %s:%d%s" % (r["path"], r["line"], " in " + r["caller"] if r["caller"] else "") for r in rows[:limit]]
    if total > limit:
        out.append("  ... lighting code callers <name> lists all")
    return out


def callers(root, name):
    return "\n".join(callers_lines(db(root), name.lower(), D.CODE_ALL_CALLERS))


def source_text(root, r):
    try:
        lines = (root / r["path"]).read_text("utf-8", errors="replace").splitlines()
    except OSError:
        return r["sig"] or r["name"]
    body = "\n".join(lines[r["line"] - 1: min(r["end_line"], r["line"] - 1 + D.CODE_EMBED_LINES)])
    return "%s %s in %s (%s)\n%s" % (r["kind"], r["name"], r["parent"] or "-", r["path"], body)


def unembedded(root, limit):
    return db(root).execute("select s.* from symbols s left join vecs v on v.id = s.id where v.id is null and "
                            "s.kind in ('function', 'method', 'constructor', 'class', 'struct', 'enum', 'trait', "
                            "'interface', 'record', 'macro') limit ?", (limit,)).fetchall()


def embed_some(root, seconds=50):
    import numpy as np
    from lighting import games, memmodel as MM
    c, end, n = db(root), time.time() + seconds, 0
    while time.time() < end and not games.running():
        todo = unembedded(root, D.MEM_EMBED_BATCH)
        if not todo:
            break
        vecs = MM.embed([source_text(root, r) for r in todo])
        if vecs is None:
            break
        c.executemany("insert or replace into vecs(id, v) values (?, ?)",
                      [(r["id"], np.asarray(v, dtype=np.float16).tobytes()) for r, v in zip(todo, vecs)])
        c.commit()
        n += len(todo)
    return n


def meaning(root, text, limit=D.CODE_LIMIT):
    import numpy as np
    from lighting import memmodel as MM
    c = db(root)
    rows = c.execute("select id, v from vecs").fetchall()
    total = c.execute("select count(*) from symbols where kind in ('function', 'method', 'constructor', 'class', "
                      "'struct', 'enum', 'trait', 'interface', 'record', 'macro')").fetchone()[0]
    if not rows:
        return "the meaning index of %s is not built yet (%d symbols wait; it builds in the background)" % (
            root.name, total)
    q = MM.embed([text], query=True, timeout=30)
    if q is None:
        raise Fail("the local model is not running", "lighting memory model")
    ids = np.array([r["id"] for r in rows])
    mat = np.frombuffer(b"".join(r["v"] for r in rows), dtype=np.float16).astype(np.float32).reshape(-1, D.MEM_VEC_DIM)
    sim = mat @ q[0]
    order = np.argsort(-sim)[:limit]
    best, score = [int(ids[j]) for j in order], {int(ids[j]): float(sim[j]) for j in order}
    found = {r["id"]: r for r in c.execute("select * from symbols where id in (%s)" % ",".join("?" * len(best)), best)}
    lines = ["  %s:%d %s %s%s (%.2f)" % (found[i]["path"], found[i]["line"], found[i]["kind"], found[i]["name"],
                                         " in " + found[i]["parent"] if found[i]["parent"] else "",
                                         score[i]) for i in best if i in found]
    head = "closest by meaning:" + ("" if len(rows) >= total else " (%d of %d symbols indexed so far)" % (len(rows), total))
    return "\n".join([head] + lines)


def used(root):
    seen = {k: v for k, v in (M.meta_json("code_roots") or {}).items() if time.time() - v < D.CODE_ACTIVE_DAYS * 86400}
    seen[str(root)] = time.time()
    M.meta_json("code_roots", seen)


def active_roots():
    return [Path(k) for k, v in (M.meta_json("code_roots") or {}).items()
            if time.time() - v < D.CODE_ACTIVE_DAYS * 86400 and Path(k).is_dir()]


def cmd_code(ctx, pos, flags):
    where = flags.get("project") or getattr(ctx, "cwd", None) or os.getcwd()
    root = root_for(where)
    if root is None:
        raise Fail("no project here (%s)" % where, "run it inside a project folder, or lighting code ... --project <path>")
    refresh(root)
    used(root)
    if not pos:
        return overview(root)
    if pos[0].lower() == "callers" and len(pos) == 2:
        return callers(root, pos[1])
    text = " ".join(pos)
    if " " in text.strip():
        return meaning(root, text)
    return lookup(root, text)
