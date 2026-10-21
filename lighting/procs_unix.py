import os
import re
import signal
import subprocess
import sys

MAC = sys.platform == "darwin"


def etime(text):
    m = re.fullmatch(r"(?:(\d+)-)?(?:(\d+):)?(\d+):(\d+)", (text or "").strip())
    if not m:
        return 0.0
    d, h, mi, s = (int(x or 0) for x in m.groups())
    return float(((d * 24 + h) * 60 + mi) * 60 + s)


def ps(fields):
    try:
        out = subprocess.run(["ps", "-axo", fields], capture_output=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    return out.decode("utf-8", "replace").splitlines()


def table():
    rows = {}
    for line in ps("pid=,ppid=,etime=,rss=,comm="):
        parts = line.split(None, 4)
        if len(parts) < 5:
            continue
        pid, ppid, age, rss, comm = parts
        rows[int(pid)] = {"pid": int(pid), "ppid": int(ppid), "age": etime(age), "rss": int(rss) // 1024,
                          "exe": os.path.basename(comm.strip()).lower(), "cmd": ""}
    for line in ps("pid=,args="):
        parts = line.strip().split(None, 1)
        if len(parts) == 2 and parts[0].isdigit() and int(parts[0]) in rows:
            rows[int(parts[0])]["cmd"] = parts[1]
    return rows


def is_ours(exe):
    return exe.startswith("lighting") or re.fullmatch(r"python[\d.]*", exe) is not None


def processes():
    found, names = [], {}
    for p in table().values():
        names.setdefault(p["exe"], []).append(p["pid"])
        if is_ours(p["exe"]):
            found.append({k: p[k] for k in ("pid", "ppid", "exe", "cmd", "age")})
    return found, names


def running(name):
    want = os.path.basename(name).lower()
    return any(p["exe"] == want or os.path.basename((p["cmd"].split(" --")[0])).lower() == want
               for p in table().values())


def memory_mb(pid):
    if not MAC:
        try:
            with open("/proc/%d/status" % pid) as f:
                for line in f:
                    if line.startswith("VmRSS:"):
                        return int(line.split()[1]) // 1024
        except (OSError, ValueError):
            return 0
        return 0
    rows = ps("pid=,rss=")
    for line in rows:
        parts = line.split()
        if len(parts) == 2 and parts[0] == str(pid):
            return int(parts[1]) // 1024
    return 0


def memory_load():
    if not MAC:
        info = {}
        try:
            with open("/proc/meminfo") as f:
                for line in f:
                    k, v = line.split(":", 1)
                    info[k] = int(v.split()[0])
        except (OSError, ValueError):
            return 0, 0.0
        total, avail = info.get("MemTotal", 1), info.get("MemAvailable", 0)
        return int(100 - avail * 100 / max(total, 1)), avail / 2 ** 20
    try:
        total = int(subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, timeout=5).stdout)
        vm = subprocess.run(["vm_stat"], capture_output=True, timeout=5).stdout.decode()
    except (OSError, ValueError, subprocess.SubprocessError):
        return 0, 0.0
    size = int((re.search(r"page size of (\d+)", vm) or [0, 4096])[1])
    pages = {k.strip(): int(v.strip(" .")) for k, v in re.findall(r"^([^:\n]+):\s+(\d+)\.", vm, re.M)}
    free = (pages.get("Pages free", 0) + pages.get("Pages inactive", 0) + pages.get("Pages speculative", 0)) * size
    return int(100 - free * 100 / max(total, 1)), free / 2 ** 30


def kill(pid):
    rows = table()
    kids, todo = [], [pid]
    while todo:
        cur = todo.pop()
        kids.append(cur)
        todo += [p["pid"] for p in rows.values() if p["ppid"] == cur and p["pid"] not in kids]
    for p in reversed(kids):
        try:
            os.kill(p, signal.SIGKILL)
        except OSError:
            pass
