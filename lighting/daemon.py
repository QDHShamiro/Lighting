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

    def call(self, cmd, args=None, tab=None, timeout=D.CALL_TIMEOUT):
        with self.lock:
            self.seq += 1
            i = self.seq
            box = queue.Queue(1)
            self.pending[i] = box
        try:
            ipc.send(self.conn, {"id": i, "cmd": cmd, "args": args or {}, "tab": tab})
            try:
                msg = box.get(timeout=timeout)
            except queue.Empty:
                raise TimeoutError("browser did not answer '%s' within %ds" % (cmd, timeout))
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

    def add_host(self, conn, hello):
        host = Host(self, conn, hello)
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
            return
        text = msg.get("text")
        if text:
            self.events.append(text)

    def abort_all(self):
        self.abort.set()
        for host in list(self.hosts):
            host.fail_all({"ok": False, "abort": True})
            host.post("abort")
        while True:
            try:
                _, reply = self.jobs.get_nowait()
            except queue.Empty:
                break
            reply.put({"out": "stopped (Ctrl+Alt+End or lighting abort)", "code": 1})

    def handle(self, conn):
        try:
            if not conn.poll(10):
                raise TimeoutError()
            raw = conn.recv_bytes()
            fast = raw.startswith(ipc.FAST)
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

    def worker(self):
        from lighting import commands
        ctx = commands.Context(self)
        while True:
            msg, reply = self.jobs.get()
            self.abort.clear()
            try:
                res = commands.run(ctx, msg)
            except Aborted:
                res = {"out": "stopped (Ctrl+Alt+End or lighting abort)", "code": 1}
            except Exception as e:
                traceback.print_exc(file=sys.stderr)
                sys.stderr.flush()
                res = {"out": "err: %s: %s" % (type(e).__name__, str(e)[:300]), "code": 1}
            reply.put(res)

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
        cleanup_out()
        threading.Thread(target=self.worker, daemon=True).start()
        threading.Thread(target=self.hotkey_loop, daemon=True).start()
        while True:
            try:
                conn = listener.accept()
            except Exception:
                continue
            threading.Thread(target=self.handle, args=(conn,), daemon=True).start()


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
    Daemon().serve()
