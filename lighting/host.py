import ctypes
import json
import msvcrt
import os
import struct
import sys
import threading

from lighting import boot
from lighting import ipc


def read(stream):
    raw = stream.read(4)
    if len(raw) < 4:
        return None
    size = struct.unpack("<I", raw)[0]
    data = stream.read(size)
    if len(data) < size:
        return None
    return json.loads(data.decode("utf-8"))


def write(stream, lock, obj):
    data = json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    with lock:
        stream.write(struct.pack("<I", len(data)) + data)
        stream.flush()


def main():
    try:
        ctypes.windll.winmm.timeBeginPeriod(1)
    except (AttributeError, OSError):
        pass
    msvcrt.setmode(sys.stdin.fileno(), os.O_BINARY)
    msvcrt.setmode(sys.stdout.fileno(), os.O_BINARY)
    inp, out, lock = sys.stdin.buffer, sys.stdout.buffer, threading.Lock()
    first = read(inp)
    if not first:
        return
    try:
        conn = boot.connect_or_start()
    except SystemExit:
        return
    first["type"] = "hello"
    first["token"] = ipc.token()
    first["origin"] = sys.argv[1] if len(sys.argv) > 1 else ""
    ipc.send(conn, first)

    def down():
        try:
            while True:
                write(out, lock, ipc.recv(conn))
        except (EOFError, OSError, ValueError):
            pass
        os._exit(0)

    threading.Thread(target=down, daemon=True).start()
    try:
        while True:
            msg = read(inp)
            if msg is None:
                break
            ipc.send(conn, msg)
    except (EOFError, OSError, ValueError):
        pass
    os._exit(0)
