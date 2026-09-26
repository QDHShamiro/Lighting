import hashlib
import os
import shutil
import subprocess
import sys
import time

from lighting import defaults as D
from lighting import ipc

DETACHED = 0x00000200 | 0x08000000
BREAKAWAY = 0x01000000


def say(msg):
    print("lighting: " + msg, file=sys.stderr, flush=True)


def signature():
    req = hashlib.sha1((D.ROOT / "requirements.txt").read_bytes()).hexdigest()[:12]
    return req + "|" + str(D.ROOT) + "|" + D.version()


def uv():
    exe = shutil.which("uv")
    if not exe:
        raise SystemExit("lighting needs uv: https://docs.astral.sh/uv/getting-started/installation/")
    return exe


def ensure_venv():
    sig = signature()
    try:
        old = D.STAMP.read_text("utf-8")
    except OSError:
        old = ""
    if old == sig and D.PY.exists():
        if not (D.HOME / "client-root").exists():
            (D.HOME / "client-root").write_text(str(D.ROOT), "utf-8")
        os.utime(D.STAMP)
        return False
    if not D.PY.exists() or old.split("|")[0] != sig.split("|")[0]:
        say("installing dependencies (one time, ~20-60 s) ...")
        D.HOME.mkdir(parents=True, exist_ok=True)
        if not D.PY.exists():
            subprocess.run([uv(), "venv", "--quiet", "--python", ">=3.11", str(D.VENV)], check=True)
        subprocess.run([uv(), "pip", "install", "--quiet", "--python", str(D.PY),
                        "-r", str(D.ROOT / "requirements.txt")], check=True)
    from lighting import install
    (D.HOME / "client-root").write_text(str(D.ROOT), "utf-8")
    install.refresh()
    stop_daemon()
    D.STAMP.write_text(sig, "utf-8")
    return True


def stop_daemon():
    try:
        ipc.request({"argv": ["stop"]}, 5)
    except (OSError, EOFError, TimeoutError, ValueError):
        return
    deadline = time.time() + 5
    while time.time() < deadline:
        try:
            ipc.connect().close()
        except OSError:
            return
        time.sleep(0.05)


def start_daemon():
    exe = D.PYW if D.PYW.exists() else D.PY if D.PY.exists() else sys.executable
    env = dict(os.environ, PYTHONPATH=str(D.ROOT), LIGHTING_ROOT=str(D.ROOT))
    D.HOME.mkdir(parents=True, exist_ok=True)
    log = open(D.HOME / "daemon.log", "ab")
    args = [str(exe), "-X", "utf8", "-m", "lighting", "daemon"]
    kw = dict(env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, cwd=str(D.HOME), close_fds=True)
    try:
        subprocess.Popen(args, creationflags=DETACHED | BREAKAWAY, **kw)
    except OSError:
        subprocess.Popen(args, creationflags=DETACHED, **kw)
    finally:
        log.close()


def connect_or_start():
    try:
        return ipc.connect()
    except OSError:
        pass
    start_daemon()
    deadline = time.time() + D.BOOT_WAIT
    while time.time() < deadline:
        try:
            return ipc.connect()
        except OSError:
            time.sleep(0.05)
    raise SystemExit("lighting: daemon did not start, see " + str(D.HOME / "daemon.log"))


def request(payload, timeout=D.CLIENT_TIMEOUT):
    conn = connect_or_start()
    try:
        ipc.send(conn, dict(payload, token=ipc.token()))
        return ipc.recv(conn, timeout)
    finally:
        conn.close()
