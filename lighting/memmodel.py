import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import tarfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

from lighting import defaults as D
from lighting import games
from lighting import memory as M

LOCAL_HOSTS = ("127.0.0.1", "localhost", "::1")


class NotReady(Exception):
    pass


_st = {"chat": None, "embed": None, "dl": None, "note": "", "api_ok": (0.0, False), "lock": threading.RLock(),
       "down": (0.0, ""), "probing": None, "warming": None}


def cfg():
    return D.config()


def models_dir():
    return Path(cfg().get("models_dir") or D.MODELS)


def ram_gb():
    try:
        if D.WIN:
            import ctypes

            class Status(ctypes.Structure):
                _fields_ = [("len", ctypes.c_ulong), ("load", ctypes.c_ulong), ("total", ctypes.c_ulonglong),
                            ("avail", ctypes.c_ulonglong), ("pt", ctypes.c_ulonglong), ("pa", ctypes.c_ulonglong),
                            ("vt", ctypes.c_ulonglong), ("va", ctypes.c_ulonglong), ("ve", ctypes.c_ulonglong)]
            s = Status()
            s.len = ctypes.sizeof(Status)
            ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(s))
            return s.total / 2 ** 30
        return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2 ** 30
    except (OSError, ValueError, AttributeError):
        return 8.0


def chat_key():
    return "4b" if ram_gb() >= D.MEM_BIG_RAM_GB else "2b"


def model_file(kind):
    own = cfg().get("memory_model" if kind == "chat" else "memory_embed")
    if own:
        return Path(own)
    spec = D.MEM_MODELS[chat_key() if kind == "chat" else "embed"]
    return models_dir() / spec["file"]


def platform_key():
    return "%s-%s" % (D.OS, D.ARCH)


def server_file():
    own = cfg().get("memory_server")
    if own:
        return Path(own)
    name = "llama-server.exe" if D.WIN else "llama-server"
    bad = (M.meta("llama_bad") or "").split(",")
    for p in sorted((models_dir() / "llama.cpp").glob("*/**/" + name), reverse=True):
        if not any(b and b in str(p) for b in bad):
            return p
    return None


def ready(kind):
    m, s = model_file(kind), server_file()
    return bool(s and s.exists() and m.exists() and m.stat().st_size > 1_000_000)


def reachable(base):
    u = urllib.parse.urlsplit(base)
    if (u.hostname or "") not in LOCAL_HOSTS:
        return True
    try:
        socket.create_connection((u.hostname, u.port or 80), timeout=D.MEM_LOCAL_CONNECT_S).close()
        return True
    except (OSError, ValueError):
        return False


def probe(base):
    ok = False
    if reachable(base):
        local = (urllib.parse.urlsplit(base).hostname or "") in LOCAL_HOSTS
        try:
            with urllib.request.urlopen(base + "/v1/models", timeout=D.MEM_API_PROBE_S if local else D.MEM_API_REMOTE_S) as r:
                ok = r.status == 200
        except (OSError, urllib.error.URLError, ValueError):
            ok = False
    _st["api_ok"] = (time.time(), ok)
    return ok


def api(kind, wait=True):
    c = cfg()
    base = (c.get("memory_api") or "").rstrip("/")
    model = c.get("memory_api_model" if kind == "chat" else "memory_api_embed")
    if not base or not model:
        return None
    if base.endswith("/v1"):
        base = base[:-3]
    t, ok = _st["api_ok"]
    if time.time() - t > D.MEM_API_RECHECK_S:
        if wait or not t:
            ok = probe(base)
        elif not (_st["probing"] and _st["probing"].is_alive()):
            _st["probing"] = threading.Thread(target=probe, args=(base,), daemon=True)
            _st["probing"].start()
    return (base, model) if ok else None


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def get(url, timeout=3):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.status


class Server:
    def __init__(self, kind):
        self.kind = kind
        self.proc = None
        self.port = 0
        self.used = 0.0
        self.log = None
        self.up = False

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def base(self):
        return "http://127.0.0.1:%d" % self.port

    def args(self, exe, model, ngl):
        a = [str(exe), "-m", str(model), "--host", "127.0.0.1", "--port", str(self.port), "-ngl", str(ngl),
             "-t", str(max(1, (os.cpu_count() or 4) // 2))]
        if self.kind == "embed":
            n = str(D.MEM_EMBED_CTX)
            return a + ["--embedding", "--pooling", "last", "-c", n, "-b", n, "-ub", n]
        return a + ["-c", str(D.MEM_CTX), "--jinja"]

    def start(self):
        exe, model = server_file(), model_file(self.kind)
        if not ready(self.kind):
            raise NotReady("the local model is not downloaded yet (lighting memory model)")
        D.MEMORY.mkdir(parents=True, exist_ok=True)
        path = D.MEMORY / ("server-%s.log" % self.kind)
        try:
            if path.stat().st_size > D.SERVER_LOG_MAX:
                path.write_bytes(path.read_bytes()[-D.SERVER_LOG_KEEP:])
        except OSError:
            pass
        for ngl in (99, 0):
            self.port = free_port()
            self.log = open(path, "ab")
            kw = {"creationflags": D.NO_WINDOW | 0x4000} if D.WIN else {"preexec_fn": lambda: os.nice(10)}
            self.proc = subprocess.Popen(self.args(exe, model, ngl), stdout=self.log, stderr=self.log,
                                         stdin=subprocess.DEVNULL, cwd=str(Path(exe).parent), **kw)
            end = time.time() + D.MEM_SERVER_BOOT_S
            while time.time() < end and self.alive():
                try:
                    if get(self.base() + "/health", 2) == 200:
                        self.used = time.time()
                        self.up = True
                        return self.base()
                except (OSError, urllib.error.URLError):
                    pass
                time.sleep(0.5)
            self.stop()
        if not cfg().get("memory_server"):
            try:
                top = Path(exe).relative_to(models_dir() / "llama.cpp").parts[0]
            except ValueError:
                top = Path(exe).parent.name
            M.meta("llama_bad", ",".join([b for b in (M.meta("llama_bad") or "").split(",") if b] + [top]))
        raise NotReady("the local model server did not start (see %s)" % (D.MEMORY / ("server-%s.log" % self.kind)))

    def stop(self):
        self.up = False
        p, self.proc = self.proc, None
        if p and p.poll() is None:
            p.terminate()
            try:
                p.wait(5)
            except subprocess.TimeoutExpired:
                p.kill()
        if self.log:
            self.log.close()
            self.log = None


def base_for(kind, start=True, wait=True):
    found = api(kind, wait)
    if found:
        return found
    if not start:
        s = _st[kind]
        if s and s.up and s.alive():
            s.used = time.time()
            return s.base(), "local"
        return None
    with _st["lock"]:
        s = _st[kind] or Server(kind)
        _st[kind] = s
        if not s.alive():
            if games.running():
                raise NotReady("a game is running")
            s.start()
        s.used = time.time()
        return s.base(), "local"


def post(base, path, body, timeout):
    req = urllib.request.Request(base + path, json.dumps(body).encode("utf-8"), {"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        try:
            detail = e.read()[:200].decode("utf-8", "replace")
        except OSError:
            detail = ""
        raise NotReady("model answered %d: %s" % (e.code, detail)) from None
    except (OSError, urllib.error.URLError) as e:
        raise NotReady("model not reachable: %s" % e) from None


def chat(system, user, schema=None):
    base, model = base_for("chat")
    body = {"model": model, "temperature": 0.2, "max_tokens": D.MEM_ANSWER_TOKENS,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "chat_template_kwargs": {"enable_thinking": False}}
    if schema:
        body["response_format"] = {"type": "json_schema", "json_schema": {"name": "memory", "schema": schema}}
    data = post(base, "/v1/chat/completions", body, D.MEM_CALL_S)
    text = data["choices"][0]["message"].get("content") or ""
    return re.sub(r"(?s)<think>.*?</think>", "", text).strip()


def down():
    until, why = _st["down"]
    return why if time.time() < until else ""


def embed(texts, query=False, start=True, timeout=120, wait=True):
    import numpy as np
    why = down()
    if why:
        raise NotReady(why)
    found = base_for("embed", start, wait)
    if not found:
        return None
    base, model = found
    items = [(D.MEM_QUERY_PREFIX if query else "") + t[: D.MEM_EMBED_CHARS] for t in texts]
    try:
        data = post(base, "/v1/embeddings", {"model": model, "input": items}, timeout)
    except NotReady as e:
        _st["down"] = (time.time() + D.MEM_EMBED_BACKOFF_S, str(e))
        raise
    vecs = np.array([d["embedding"] for d in sorted(data["data"], key=lambda d: d["index"])],
                    dtype=np.float32)[:, : D.MEM_VEC_DIM]
    return vecs / np.maximum(np.linalg.norm(vecs, axis=1, keepdims=True), 1e-9)


def reason(text):
    m = re.match(r"model answered (\d+)", text)
    if m:
        return "the model is paused" if m.group(1) == "503" else "the model answered %s" % m.group(1)
    if "timed out" in text:
        return "the model did not answer in time"
    if text.startswith("model not reachable"):
        return "the model is not reachable"
    return text


def query(text, budget, start=False):
    try:
        v = embed([text], query=True, start=False, timeout=budget, wait=False)
    except NotReady as e:
        return None, reason(str(e))
    if v is not None:
        return v[0], ""
    if not ready("embed"):
        return None, "no local model yet (lighting memory model)"
    if start:
        warm("embed")
        return None, "the model is starting"
    return None, "the model is not running"


def warm(kind):
    t = _st["warming"]
    if (t and t.is_alive()) or not ready(kind):
        return

    def run():
        try:
            base_for(kind, True)
        except (NotReady, OSError, ValueError):
            pass

    _st["warming"] = threading.Thread(target=run, daemon=True)
    _st["warming"].start()


def reap():
    gaming = games.running()
    for kind in ("chat", "embed"):
        s = _st[kind]
        if s and s.alive() and (gaming or time.time() - s.used > D.MEM_SERVER_IDLE_S):
            s.stop()
    return gaming


def stop_all():
    for kind in ("chat", "embed"):
        if _st[kind]:
            _st[kind].stop()


def tls():
    from lighting import browser
    return browser.tls()


def fetch(url, dest, size=None, sha=None):
    dest.parent.mkdir(parents=True, exist_ok=True)
    need = (size or 0) + 2 ** 30
    if shutil.disk_usage(dest.parent).free < need:
        raise NotReady("not enough space on %s for %s (lighting config models_dir <folder on a bigger drive>)"
                       % (dest.parent, dest.name))
    part = dest.with_name(dest.name + ".part")
    h = hashlib.sha256()
    have = part.stat().st_size if part.exists() else 0
    if size and have >= size:
        have = 0
    headers = {"User-Agent": "lighting"}
    if have:
        headers["Range"] = "bytes=%d-" % have
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60, context=tls()) as r:
            if r.status != 206:
                have = 0
            with open(part, "ab" if have else "wb") as f:
                if have:
                    with open(part, "rb") as old:
                        for block in iter(lambda: old.read(1 << 20), b""):
                            h.update(block)
                total, done = have + int(r.headers.get("Content-Length") or (size or 0) - have), have
                while True:
                    if games.running():
                        raise NotReady("paused: a game started, it goes on later")
                    chunk = r.read(1 << 20)
                    if not chunk:
                        break
                    f.write(chunk)
                    h.update(chunk)
                    done += len(chunk)
                    _st["dl"] = (dest.name, done, total)
    finally:
        _st["dl"] = None
    if sha and h.hexdigest() != sha:
        part.unlink(missing_ok=True)
        raise NotReady("%s arrived broken (checksum), it is downloaded again next time" % dest.name)
    part.replace(dest)
    return dest


def fetch_server():
    with urllib.request.urlopen(urllib.request.Request(D.LLAMA_RELEASES, headers={"User-Agent": "lighting"}),
                                timeout=30, context=tls()) as r:
        releases = json.loads(r.read().decode("utf-8"))
    bad = (M.meta("llama_bad") or "").split(",")
    assets = [a for rel in releases if not rel.get("draft") for a in rel.get("assets", [])]
    for pattern in D.MEM_LLAMA_ASSETS.get(platform_key(), []):
        for a in assets:
            name = a["name"]
            folder = re.sub(r"\.(zip|tar\.gz)$", "", name)
            if re.search(pattern, name) and folder not in bad:
                path = fetch(a["browser_download_url"], models_dir() / "llama.cpp" / name, a.get("size"))
                out = models_dir() / "llama.cpp" / folder
                if name.endswith(".zip"):
                    with zipfile.ZipFile(path) as z:
                        z.extractall(out)
                else:
                    with tarfile.open(path) as t:
                        t.extractall(out, filter="data")
                path.unlink(missing_ok=True)
                for p in out.rglob("llama-server*"):
                    p.chmod(0o755)
                return
    raise NotReady("no llama.cpp build for %s (set lighting config memory_server <path to llama-server>)"
                   % platform_key())


def download():
    if games.running():
        raise NotReady("a game is running, the download waits")
    if not cfg().get("memory_server") and server_file() is None:
        _st["note"] = "downloading llama.cpp"
        fetch_server()
    for kind in ("chat", "embed"):
        path = model_file(kind)
        if path.exists() or cfg().get("memory_model" if kind == "chat" else "memory_embed"):
            continue
        spec = D.MEM_MODELS[chat_key() if kind == "chat" else "embed"]
        _st["note"] = "downloading " + spec["file"]
        fetch("https://huggingface.co/%s/resolve/main/%s" % (spec["repo"], spec["file"]), path, spec["size"],
              spec["sha256"])
    _st["note"] = ""


def download_bg():
    if _st.get("thread") and _st["thread"].is_alive():
        return False

    def run():
        try:
            download()
        except Exception as e:
            _st["note"] = "download stopped: %s" % e

    _st["thread"] = threading.Thread(target=run, daemon=True)
    _st["thread"].start()
    return True


def status(wait=True):
    parts = []
    for kind in ("chat", "embed"):
        a = api(kind, wait)
        if a:
            parts.append("%s: %s via %s" % (kind, a[1], a[0]))
            continue
        f = model_file(kind)
        s = _st[kind]
        state = "running" if s and s.alive() else "ready" if ready(kind) else "not downloaded"
        parts.append("%s: %s (%s)" % (kind, f.name, state))
    dl = _st["dl"]
    if dl:
        parts.append("download %s %d%%" % (dl[0], 100 * dl[1] // max(1, dl[2])))
    elif _st["note"]:
        parts.append(_st["note"])
    return " | ".join(parts)
