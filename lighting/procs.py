import re
import sys
import subprocess

from lighting import defaults as D

if D.WIN:
    from lighting.procs_win import memory_load, memory_mb, processes
else:
    from lighting.procs_unix import memory_load, memory_mb, processes, running

HIDDEN = 0x08000000 if sys.platform == "win32" else 0
SILENT = {"done", "suggest"}
LONG_OK = {"mcp", "doctor"}
PYTHONS = {"python.exe", "pythonw.exe"}
CLIENT_RE = re.compile(r"lighting(?:\.exe|-macos|-linux-\w+)?\"?(?=\s|$)", re.I)


def is_python(exe):
    return exe in PYTHONS or re.fullmatch(r"python[\d.]*w?", exe or "") is not None


def is_client(exe):
    return exe == "lighting.exe" or re.fullmatch(r"lighting(?:-macos|-linux-\w+)?", exe or "") is not None


def label(exe, cmd):
    script = next((a.strip('"') for a in re.findall(r'"[^"]+"|\S+', cmd or "") if a.strip('"').lower().endswith((".py", ".pyw"))), "")
    return "%s %s" % (exe, script) if is_python(exe) and script else exe


def biggest(pids, ps, n):
    cmds = {p["pid"]: p["cmd"] for p in ps}
    total = {}
    for exe, ids in pids.items():
        for pid in ids:
            key = label(exe, cmds.get(pid, ""))
            total[key] = total.get(key, 0) + memory_mb(pid)
    return sorted(((mb, key) for key, mb in total.items()), reverse=True)[:n]


def client_args(cmd):
    rest = CLIENT_RE.split(cmd or "", maxsplit=1)
    return rest[1].split() if len(rest) > 1 else []


def is_daemon(p):
    return is_python(p["exe"]) and re.search(r"-m\s+lighting\s+daemon\b", p["cmd"]) is not None


def is_host(p):
    return is_python(p["exe"]) and "host.py" in p["cmd"].lower()


def tops(ps, test):
    by = {p["pid"]: p for p in ps}
    return [p for p in ps if test(p) and not (p["ppid"] in by and test(by[p["ppid"]]))]


def top_of(ps, pid, test):
    by = {p["pid"]: p for p in ps}
    p = by.get(pid)
    while p and p["ppid"] in by and test(by[p["ppid"]]) and by[p["ppid"]] is not p:
        p = by[p["ppid"]]
    return p["pid"] if p else None


def classify(ps, daemon_pid, host_pids):
    hung = []
    for p in ps:
        if not is_client(p["exe"]):
            continue
        args = client_args(p["cmd"])
        first = args[0].lower() if args else ""
        if not first or first in LONG_OK:
            continue
        if (first in SILENT and p["age"] > D.DOCTOR_HUNG_S) or p["age"] > D.DOCTOR_STUCK_S:
            hung.append((p["pid"], first, p["age"]))
    ours = top_of(ps, daemon_pid, is_daemon) if daemon_pid else None
    extra = [p["pid"] for p in tops(ps, is_daemon) if p["pid"] != ours]
    hosts = tops(ps, is_host)
    orphans = []
    if host_pids is not None:
        linked = {top_of(ps, pid, is_host) for pid in host_pids}
        orphans = [p["pid"] for p in hosts if p["pid"] not in linked and p["age"] > D.DOCTOR_HUNG_S]
    return {"hung": hung, "extra": extra, "hosts": len(hosts), "orphans": orphans}


def kill(pid):
    if not D.WIN:
        from lighting.procs_unix import kill as unix_kill
        return unix_kill(pid)
    subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True, timeout=15, creationflags=HIDDEN)
