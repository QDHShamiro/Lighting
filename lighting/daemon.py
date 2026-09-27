import collections
import ctypes
import json
import ctypes.wintypes as W
import os
import queue
import sys
import threading
import time
import traceback
from multiprocessing.connection import Listener

from lighting import defaults as D
from lighting import ipc


TRACE = (D.HOME / "trace").exists()


class Aborted(Exception):
    pass


class Host:
    def __init__(self, daemon, conn, hello):
        self.d = daemon
        self.conn = conn
        self.info = hello
        self.brand = (hello.get("brand") or "browser").lower()
        self.pending = {}
        self.lock = threading.Lock()
        self.seq = 0
        self.alive = True
        self.connected = time.time()
        self.focused = time.time() if hello.get("focused") else 0.0

    def call(self, cmd, args=None, tab=None, timeout=D.CALL_TIMEOUT, group=None):
        with self.lock:
            self.seq += 1
            i = self.seq
            box = queue.Queue(1)
            self.pending[i] = box
        try:
            t0 = time.perf_counter()
            ipc.send(self.conn, {"id": i, "cmd": cmd, "args": args or {}, "tab": tab, "group": group})
            t1 = time.perf_counter()
            try:
                msg = box.get(timeout=timeout)
            except queue.Empty:
                raise TimeoutError("browser did not answer '%s' within %ds" % (cmd, timeout))
            t2 = time.perf_counter()
            if TRACE:
                print("trace browser %s: send %.2f ms, reply %.2f ms, inside extension %.2f ms" % (
                    cmd, (t1 - t0) * 1000, (t2 - t0) * 1000, msg.get("extMs", -1)), file=sys.stderr, flush=True)
        finally:
            self.pending.pop(i, None)
        if msg.get("abort"):
            raise Aborted()
        return msg

    def post(self, cmd, args=None):
        try:
            ipc.send(self.conn, {"id": 0, "cmd": cmd, "args": args or {}})
        except OSError:
            pass

    def fail_all(self, msg):
        for box in list(self.pending.values()):
            try:
                box.put_nowait(msg)
            except queue.Full:
                pass

    def loop(self):
        try:
            while True:
                msg = ipc.recv(self.conn)
                if msg.get("id"):
                    box = self.pending.get(msg["id"])
                    if box:
                        box.put(msg)
                elif msg.get("event"):
                    self.d.on_event(self, msg)
        except (EOFError, OSError, ValueError):
            pass
        finally:
            self.alive = False
            self.fail_all({"ok": False, "error": "browser disconnected"})
            self.d.drop_host(self)


class Daemon:
    def __init__(self):
        self.jobs = queue.Queue()
        self.hosts = []
        self.events = collections.deque(maxlen=30)
        self.abort = threading.Event()
        self.started = time.time()
        self.hotkey = False
        self.hosts_seen = set()
        self.lock = threading.Lock()
        self.abort_gen = 0
        self.ctxs = {}
        self.epochs = {}
        self.saved = None

    def session_rows(self):
        rows = {}
        for sid, c in self.ctxs.items():
            st = getattr(c, "app", None)
            if sid:
                rows[sid] = {"group": c.group, "tabs": list(c.tabs), "owned": sorted(c.owned), "used": int(c.used),
                             "target": list(c.target) if c.target else None,
                             "windows": dict(st.windows) if st else {}, "hwnd": st.hwnd if st else None,
                             "launched": {str(k): v for k, v in st.launched.items()} if st else {}}
        return rows

    def save_sessions(self):
        rows = self.session_rows()
        sig = json.dumps([{k: dict(v, used=v["used"] // 600) for k, v in rows.items()}, self.epochs], sort_keys=True)
        if sig == self.saved:
            return
        self.saved = sig
        try:
            D.SESSIONS.write_text(json.dumps({"epochs": self.epochs, "sessions": rows}), "utf-8")
        except OSError:
            pass

    def load_sessions(self, commands):
        from lighting import desktop
        try:
            data = json.loads(D.SESSIONS.read_text("utf-8"))
        except (OSError, ValueError):
            return
        self.epochs = data.get("epochs") or {}
        for sid, s in (data.get("sessions") or {}).items():
            if not sid or time.time() - s.get("used", 0) > D.SESSION_IDLE_S:
                continue
            c = commands.Context(self, sid)
            c.group, c.tabs, c.owned, c.used = s.get("group") or c.group, list(s.get("tabs") or []), set(s.get("owned") or []), s.get("used", 0)
            c.target = tuple(s["target"]) if s.get("target") else None
            st = desktop.state(c)
            st.windows = {k: int(v) for k, v in (s.get("windows") or {}).items()}
            st.launched = {int(k): v for k, v in (s.get("launched") or {}).items()}
            st.hwnd = s.get("hwnd")
            self.ctxs[sid] = c

    def note_epoch(self, brand, ep):
        if ep and self.epochs.get(brand, ep) != ep:
            for c in list(self.ctxs.values()):
                c.tabs, c.owned = [], set()
        if ep:
            self.epochs[brand] = ep

    def add_host(self, conn, hello):
        host = Host(self, conn, hello)
        self.note_epoch(host.brand, hello.get("epoch"))
        with self.lock:
            self.hosts.append(host)
        from lighting import browser
        browser.on_connect(self, host)
        host.loop()

    def drop_host(self, host):
        with self.lock:
            if host in self.hosts:
                self.hosts.remove(host)

    def on_event(self, host, msg):
        kind = msg.get("event")
        if kind == "focus":
            host.focused = time.time() if msg.get("focused") else host.focused
            return
        if kind == "hello":
            host.info.update(msg)
            self.note_epoch(host.brand, msg.get("epoch"))
            return
        text = msg.get("text")
        if text:
            self.events.append(text)

    def abort_all(self):
        self.abort.set()
        self.abort_gen += 1
        for host in list(self.hosts):
            host.fail_all({"ok": False, "abort": True})
            host.post("abort")
        while True:
            try:
                _, reply = self.jobs.get_nowait()
            except queue.Empty:
                break
            if reply is not None:
                reply.put({"out": "stopped (Ctrl+Alt+End or lighting abort)", "code": 1})
        self.jobs.put((None, None))

    def handle(self, conn):
        try:
            if not conn.poll(10):
                raise TimeoutError()
            raw = conn.recv_bytes()
            fast = ipc.is_fast(raw)
            msg = ipc.decode_fast(raw) if fast else json.loads(raw.decode("utf-8"))
        except (EOFError, OSError, ValueError, TimeoutError, IndexError):
            conn.close()
            return
        if not ipc.valid(msg.pop("token", None)):
            conn.close()
            return
        if msg.get("type") == "hello":
            self.add_host(conn, msg)
            return
        if (msg.get("argv") or [""])[0] == "abort":
            self.abort_all()
            res = {"out": "stopped the running and queued commands", "code": 0}
        else:
            reply = queue.Queue(1)
            self.jobs.put((msg, reply))
            res = reply.get()
        try:
            if fast:
                conn.send_bytes(ipc.encode_fast(res))
            else:
                ipc.send(conn, res)
        except OSError:
            pass
        finally:
            conn.close()
        if res.get("shutdown"):
            os._exit(0)

    def context(self, commands, sid):
        now = time.time()
        for key in [k for k, c in self.ctxs.items() if k and now - c.used > D.SESSION_IDLE_S]:
            del self.ctxs[key]
        ctx = self.ctxs.get(sid)
        if ctx is None:
            ctx = self.ctxs[sid] = commands.Context(self, sid)
            if sid:
                taken = {c.group for c in self.ctxs.values() if c is not ctx}
                n = 1
                while "Lighting #%d" % n in taken:
                    n += 1
                ctx.group = "Lighting #%d" % n
        ctx.used = now
        return ctx

    def worker(self):
        from lighting import commands
        waits = []
        while True:
            left = max(0.0, min(p.due for p, _, _ in waits) - time.time()) if waits else None
            try:
                msg, reply = self.jobs.get(timeout=left)
            except queue.Empty:
                msg = reply = None
            if msg is not None:
                self.abort.clear()
                ctx = self.context(commands, msg.get("sid") or "")
                res = self.guarded(lambda: commands.run(ctx, msg))
                if res.get("pending"):
                    waits.append((res["pending"], reply, ctx))
                else:
                    reply.put(res)
                self.save_sessions()
            now = time.time()
            for w in [w for w in waits if w[0].due <= now or w[0].gen != self.abort_gen]:
                res = self.guarded(lambda: commands.poll_pending(w[2], w[0]))
                if res is not None:
                    waits.remove(w)
                    w[1].put(res)
                    self.save_sessions()

    def guarded(self, fn):
        try:
            return fn()
        except Aborted:
            return {"out": "stopped (Ctrl+Alt+End or lighting abort)", "code": 1}
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            sys.stderr.flush()
            return {"out": "err: %s: %s" % (type(e).__name__, str(e)[:300]), "code": 1}

    def hotkey_loop(self):
        user32 = ctypes.windll.user32
        if not user32.RegisterHotKey(None, 1, D.HOTKEY_MODS | 0x4000, D.HOTKEY_VK):
            return
        self.hotkey = True
        msg = W.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            if msg.message == 0x0312:
                self.abort_all()

    def serve(self):
        D.HOME.mkdir(parents=True, exist_ok=True)
        try:
            listener = Listener(D.PIPE, family="AF_PIPE")
        except OSError:
            return
        ipc.token()
        D.PIDFILE.write_text(str(os.getpid()), "utf-8")
        from lighting import commands
        self.load_sessions(commands)
        cleanup_out()
        try:
            from lighting import install
            install.copy_exe()
        except OSError:
            pass
        threading.Thread(target=self.worker, daemon=True).start()
        threading.Thread(target=self.hotkey_loop, daemon=True).start()
        threading.Thread(target=warm_apps, daemon=True).start()
        while True:
            try:
                conn = listener.accept()
            except Exception:
                continue
            threading.Thread(target=self.handle, args=(conn,), daemon=True).start()


def warm_apps():
    try:
        from lighting import desktop
        desktop.start_apps()
    except Exception:
        pass


def cleanup_out():
    D.OUT.mkdir(parents=True, exist_ok=True)
    limit = time.time() - D.OUT_KEEP_HOURS * 3600
    for p in D.OUT.iterdir():
        try:
            if p.stat().st_mtime < limit:
                p.unlink()
        except OSError:
            pass


def main():
    try:
        ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    except (AttributeError, OSError):
        pass
    try:
        ctypes.windll.winmm.timeBeginPeriod(1)
    except (AttributeError, OSError):
        pass
    Daemon().serve()
