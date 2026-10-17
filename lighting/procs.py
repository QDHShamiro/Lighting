import ctypes
import ctypes.wintypes as W
import re
import subprocess
import time

from lighting import defaults as D

HIDDEN = 0x08000000
LIMITED = 0x1000
CMDLINE_INFO = 60
SNAP_PROCESS = 2
FILETIME_EPOCH = 116444736000000000
SILENT = {"done", "suggest"}
LONG_OK = {"mcp", "doctor"}
PYTHONS = {"python.exe", "pythonw.exe"}


class PE(ctypes.Structure):
    _fields_ = [("dwSize", W.DWORD), ("cntUsage", W.DWORD), ("th32ProcessID", W.DWORD),
                ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", W.DWORD), ("cntThreads", W.DWORD),
                ("th32ParentProcessID", W.DWORD), ("pcPriClassBase", ctypes.c_long), ("dwFlags", W.DWORD),
                ("szExeFile", ctypes.c_wchar * 260)]


k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.CreateToolhelp32Snapshot.restype = W.HANDLE
k32.CreateToolhelp32Snapshot.argtypes = [W.DWORD, W.DWORD]
k32.Process32FirstW.argtypes = [W.HANDLE, ctypes.POINTER(PE)]
k32.Process32NextW.argtypes = [W.HANDLE, ctypes.POINTER(PE)]
k32.OpenProcess.restype = W.HANDLE
k32.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
k32.GetProcessTimes.argtypes = [W.HANDLE] + [ctypes.POINTER(W.FILETIME)] * 4
k32.CloseHandle.argtypes = [W.HANDLE]
nt = ctypes.WinDLL("ntdll")
nt.NtQueryInformationProcess.argtypes = [W.HANDLE, W.ULONG, ctypes.c_void_p, W.ULONG, ctypes.POINTER(W.ULONG)]
nt.NtQueryInformationProcess.restype = ctypes.c_long


class PMC(ctypes.Structure):
    _fields_ = [("cb", W.DWORD), ("PageFaultCount", W.DWORD)] + [(n, ctypes.c_size_t) for n in (
        "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
        "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]


class MEMSTAT(ctypes.Structure):
    _fields_ = [("dwLength", W.DWORD), ("dwMemoryLoad", W.DWORD)] + [(n, ctypes.c_ulonglong) for n in (
        "ullTotalPhys", "ullAvailPhys", "ullTotalPageFile", "ullAvailPageFile", "ullTotalVirtual", "ullAvailVirtual",
        "ullAvailExtendedVirtual")]


psapi = ctypes.WinDLL("psapi")
psapi.GetProcessMemoryInfo.argtypes = [W.HANDLE, ctypes.POINTER(PMC), W.DWORD]
k32.GlobalMemoryStatusEx.argtypes = [ctypes.POINTER(MEMSTAT)]


def memory_mb(pid):
    h = k32.OpenProcess(LIMITED, False, pid)
    if not h:
        return 0
    try:
        c = PMC()
        c.cb = ctypes.sizeof(PMC)
        return c.WorkingSetSize // 2 ** 20 if psapi.GetProcessMemoryInfo(h, ctypes.byref(c), c.cb) else 0
    finally:
        k32.CloseHandle(h)


def memory_load():
    m = MEMSTAT()
    m.dwLength = ctypes.sizeof(MEMSTAT)
    k32.GlobalMemoryStatusEx(ctypes.byref(m))
    return m.dwMemoryLoad, m.ullAvailPhys / 2 ** 30


def label(exe, cmd):
    script = next((a.strip('"') for a in re.findall(r'"[^"]+"|\S+', cmd or "") if a.strip('"').lower().endswith((".py", ".pyw"))), "")
    return "%s %s" % (exe, script) if exe in PYTHONS and script else exe


def biggest(pids, ps, n):
    cmds = {p["pid"]: p["cmd"] for p in ps}
    total = {}
    for exe, ids in pids.items():
        for pid in ids:
            key = label(exe, cmds.get(pid, ""))
            total[key] = total.get(key, 0) + memory_mb(pid)
    return sorted(((mb, key) for key, mb in total.items()), reverse=True)[:n]


def detail(pid):
    h = k32.OpenProcess(LIMITED, False, pid)
    if not h:
        return "", 0.0
    try:
        times = [W.FILETIME() for _ in range(4)]
        age = 0.0
        if k32.GetProcessTimes(h, *[ctypes.byref(t) for t in times]):
            ft = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
            age = max(0.0, time.time() - (ft - FILETIME_EPOCH) / 1e7)
        size = W.ULONG(0)
        nt.NtQueryInformationProcess(h, CMDLINE_INFO, None, 0, ctypes.byref(size))
        if not size.value:
            return "", age
        buf = ctypes.create_string_buffer(size.value)
        if nt.NtQueryInformationProcess(h, CMDLINE_INFO, buf, size.value, ctypes.byref(size)) != 0:
            return "", age
        n = ctypes.c_ushort.from_buffer(buf).value
        ptr = ctypes.c_void_p.from_buffer(buf, ctypes.sizeof(ctypes.c_void_p)).value
        return (ctypes.wstring_at(ptr, n // 2) if ptr else ""), age
    finally:
        k32.CloseHandle(h)


def processes():
    found, names = [], {}
    snap = k32.CreateToolhelp32Snapshot(SNAP_PROCESS, 0)
    if not snap or snap == ctypes.c_void_p(-1).value:
        return found, names
    try:
        e = PE()
        e.dwSize = ctypes.sizeof(PE)
        more = k32.Process32FirstW(snap, ctypes.byref(e))
        while more:
            exe = e.szExeFile.lower()
            names.setdefault(exe, []).append(e.th32ProcessID)
            if exe == "lighting.exe" or exe in PYTHONS:
                cmd, age = detail(e.th32ProcessID)
                found.append({"pid": e.th32ProcessID, "ppid": e.th32ParentProcessID, "exe": exe, "cmd": cmd, "age": age})
            more = k32.Process32NextW(snap, ctypes.byref(e))
    finally:
        k32.CloseHandle(snap)
    return found, names


def client_args(cmd):
    rest = re.split(r"lighting\.exe\"?", cmd or "", maxsplit=1, flags=re.I)
    return rest[1].split() if len(rest) > 1 else []


def is_daemon(p):
    return p["exe"] in PYTHONS and re.search(r"-m\s+lighting\s+daemon\b", p["cmd"]) is not None


def is_host(p):
    return p["exe"] in PYTHONS and "host.py" in p["cmd"].lower()


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
        if p["exe"] != "lighting.exe":
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
    subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True, timeout=15, creationflags=HIDDEN)
