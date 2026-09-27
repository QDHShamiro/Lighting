import hmac
import json
import os
from multiprocessing.connection import Client

from lighting import defaults as D

FAST = b"L1\x00"
FAST2 = b"L2\x00"


def key():
    try:
        data = D.KEY.read_bytes()
        if len(data) >= 32:
            return data
    except OSError:
        pass
    D.HOME.mkdir(parents=True, exist_ok=True)
    data = os.urandom(32)
    D.KEY.write_bytes(data)
    return data


_TOKEN = []


def token():
    if not _TOKEN:
        _TOKEN.append(key()[:32].hex())
    return _TOKEN[0]


def valid(tok):
    return isinstance(tok, str) and hmac.compare_digest(tok, token())


def send(conn, obj):
    conn.send_bytes(json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def recv(conn, timeout=None):
    if timeout is not None and not conn.poll(timeout):
        raise TimeoutError("no answer from lighting daemon")
    return json.loads(conn.recv_bytes().decode("utf-8"))


def is_fast(raw):
    return raw.startswith(FAST) or raw.startswith(FAST2)


def decode_fast(raw):
    parts = raw[len(FAST):].decode("utf-8").split("\x00")
    sid = ""
    if raw.startswith(FAST2):
        sid = parts.pop(2)
    tok, cwd, secret, argv = parts[0], parts[1], parts[2], parts[3:]
    msg = {"token": tok, "cwd": cwd, "argv": argv, "sid": sid}
    if secret.startswith("+"):
        msg["secret"] = secret[1:]
    return msg


def encode_fast(res):
    return ("%d\x00%s\x00%s" % (res.get("code", 0), res.get("image") or "", res.get("out", ""))).encode("utf-8")


def connect():
    return Client(D.PIPE, family="AF_PIPE")


def request(payload, timeout=D.CLIENT_TIMEOUT):
    conn = connect()
    try:
        send(conn, dict(payload, token=token()))
        return recv(conn, timeout)
    finally:
        conn.close()
