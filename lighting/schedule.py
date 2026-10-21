import json
import os
import re
import shutil
import subprocess
from pathlib import Path

from lighting import defaults as D
from lighting.common import Fail

FILE = D.HOME / "schedules.json"
RUNNER = D.HOME / "run.pyw"
RUNNER_PY = b"""import pathlib
import sys

home = pathlib.Path(__file__).parent
sys.path.insert(0, (home / "client-root").read_text("utf-8").strip())
if sys.stdout is None:
    sys.stdout = sys.stderr = open(home / "schedule.log", "a", encoding="utf-8")
from lighting import cli

sys.argv = ["lighting"] + sys.argv[1:]
cli.main()
"""
DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
DAYS_DE = {"mo": "mon", "di": "tue", "mi": "wed", "do": "thu", "fr": "fri", "sa": "sat", "so": "sun"}
HELP = "lighting routine schedule <name> daily 08:00 | weekly mon 09:30 | hourly | every 30m [param=value ...]"


def load():
    try:
        return json.loads(FILE.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def store(data):
    D.HOME.mkdir(parents=True, exist_ok=True)
    FILE.write_text(json.dumps(data, indent=1), "utf-8")


def parse(words):
    w = [x.lower() for x in words]
    if not w:
        raise Fail("when should it run?", HELP)
    if w[0] == "hourly":
        return {"kind": "every", "minutes": 60}, words[1:]
    if w[0] == "every" and len(w) > 1:
        m = re.fullmatch(r"(\d+)\s*(m|min|h)?", w[1])
        if not m:
            raise Fail("cannot read %s" % words[1], HELP)
        n = int(m.group(1)) * (60 if m.group(2) == "h" else 1)
        if n < 5:
            raise Fail("at most every 5 minutes", HELP)
        return {"kind": "every", "minutes": n}, words[2:]
    if w[0] in ("daily", "täglich", "taeglich"):
        return {"kind": "daily", "at": clock(words[1:2])}, words[2:]
    if w[0] in ("weekly", "wöchentlich") and len(w) > 1:
        day = DAYS_DE.get(w[1][:2], w[1][:3])
        if day not in DAYS:
            raise Fail("unknown day %s" % words[1], HELP)
        return {"kind": "weekly", "day": day, "at": clock(words[2:3])}, words[3:]
    raise Fail("unknown schedule %s" % words[0], HELP)


def clock(part):
    m = re.fullmatch(r"(\d{1,2}):(\d{2})", part[0] if part else "")
    if not m or int(m.group(1)) > 23 or int(m.group(2)) > 59:
        raise Fail("needs a time like 08:00", HELP)
    return "%02d:%02d" % (int(m.group(1)), int(m.group(2)))


def label(when):
    if when["kind"] == "every":
        return "every %d min" % when["minutes"] if when["minutes"] != 60 else "hourly"
    if when["kind"] == "daily":
        return "daily %s" % when["at"]
    return "weekly %s %s" % (when["day"], when["at"])


def task_name(name):
    return "Lighting " + name


def write_runner():
    D.HOME.mkdir(parents=True, exist_ok=True)
    try:
        if RUNNER.read_bytes() == RUNNER_PY:
            return
    except OSError:
        pass
    RUNNER.write_bytes(RUNNER_PY)


def add_windows(name, when, args):
    write_runner()
    py = D.PYW if D.PYW.exists() else D.PY
    line = '"%s" "%s" %s' % (py, RUNNER, " ".join('"%s"' % a.replace('"', "") for a in ["run", name] + args))
    cmd = ["schtasks", "/Create", "/F", "/TN", task_name(name), "/TR", line]
    if when["kind"] == "every":
        cmd += ["/SC", "MINUTE", "/MO", str(when["minutes"])]
    elif when["kind"] == "daily":
        cmd += ["/SC", "DAILY", "/ST", when["at"]]
    else:
        cmd += ["/SC", "WEEKLY", "/D", when["day"].upper(), "/ST", when["at"]]
    res = subprocess.run(cmd, capture_output=True, timeout=30, creationflags=D.NO_WINDOW)
    if res.returncode:
        raise Fail("Task Scheduler refused: %s" % res.stderr.decode("utf-8", "replace").strip()[:200])


def plist_path(name):
    return Path.home() / "Library" / "LaunchAgents" / ("dev.qdhshamiro.lighting.%s.plist" % name)


def add_macos(name, when, args):
    import plistlib
    from lighting import install
    data = {"Label": "dev.qdhshamiro.lighting.%s" % name,
            "ProgramArguments": [str(install.client_dst()), "run", name] + args,
            "StandardOutPath": str(D.HOME / ("schedule-%s.log" % name)),
            "StandardErrorPath": str(D.HOME / ("schedule-%s.log" % name))}
    if when["kind"] == "every":
        data["StartInterval"] = when["minutes"] * 60
    else:
        h, m = (int(x) for x in when["at"].split(":"))
        cal = {"Hour": h, "Minute": m}
        if when["kind"] == "weekly":
            cal["Weekday"] = (DAYS.index(when["day"]) + 1) % 7
        data["StartCalendarInterval"] = cal
    p = plist_path(name)
    p.parent.mkdir(parents=True, exist_ok=True)
    uid = str(os.getuid())
    subprocess.run(["launchctl", "bootout", "gui/%s" % uid, str(p)], capture_output=True, timeout=15)
    p.write_bytes(plistlib.dumps(data))
    res = subprocess.run(["launchctl", "bootstrap", "gui/%s" % uid, str(p)], capture_output=True, timeout=15)
    if res.returncode:
        raise Fail("launchd refused: %s" % res.stderr.decode("utf-8", "replace").strip()[:200])


def unit_dir():
    return Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "systemd" / "user"


def add_linux(name, when, args):
    from lighting import install
    if not shutil.which("systemctl"):
        raise Fail("needs systemd (systemctl --user)")
    unit = "lighting-%s" % name
    env = "".join("Environment=%s=%s\n" % (k, os.environ[k]) for k in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY",
                                                                         "XDG_SESSION_TYPE", "DBUS_SESSION_BUS_ADDRESS")
                  if os.environ.get(k))
    exec_line = " ".join([str(install.client_dst()), "run", name] + ["'%s'" % a.replace("'", "") for a in args])
    service = "[Unit]\nDescription=Lighting routine %s\n\n[Service]\nType=oneshot\n%sExecStart=%s\n" % (name, env, exec_line)
    if when["kind"] == "every":
        timer_when = "OnActiveSec=%dmin\nOnUnitActiveSec=%dmin\n" % (when["minutes"], when["minutes"])
    elif when["kind"] == "daily":
        timer_when = "OnCalendar=*-*-* %s:00\nPersistent=true\n" % when["at"]
    else:
        timer_when = "OnCalendar=%s *-*-* %s:00\nPersistent=true\n" % (when["day"].capitalize(), when["at"])
    timer = "[Unit]\nDescription=Lighting routine %s\n\n[Timer]\n%s\n[Install]\nWantedBy=timers.target\n" % (name, timer_when)
    d = unit_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / (unit + ".service")).write_text(service, "utf-8")
    (d / (unit + ".timer")).write_text(timer, "utf-8")
    subprocess.run(["systemctl", "--user", "daemon-reload"], capture_output=True, timeout=30)
    res = subprocess.run(["systemctl", "--user", "enable", "--now", unit + ".timer"], capture_output=True, timeout=30)
    if res.returncode:
        raise Fail("systemd refused: %s" % res.stderr.decode("utf-8", "replace").strip()[:200])


def remove_os(name):
    if D.WIN:
        subprocess.run(["schtasks", "/Delete", "/F", "/TN", task_name(name)], capture_output=True, timeout=30,
                       creationflags=D.NO_WINDOW)
    elif D.MAC:
        p = plist_path(name)
        subprocess.run(["launchctl", "bootout", "gui/%d" % os.getuid(), str(p)], capture_output=True, timeout=15)
        try:
            p.unlink()
        except OSError:
            pass
    else:
        unit = "lighting-%s" % name
        subprocess.run(["systemctl", "--user", "disable", "--now", unit + ".timer"], capture_output=True, timeout=30)
        for ext in (".service", ".timer"):
            try:
                (unit_dir() / (unit + ext)).unlink()
            except OSError:
                pass
        subprocess.run(["systemctl", "--user", "daemon-reload"], capture_output=True, timeout=30)


def add(name, words):
    from lighting import routines
    r = routines.load(name)
    when, rest = parse(words)
    args = [a for a in rest if "=" in a]
    if any("@secret" in a for s in r["steps"] for a in s["args"]):
        raise Fail("%s types a secret, a schedule cannot pass one" % r["name"])
    (add_windows if D.WIN else add_macos if D.MAC else add_linux)(r["name"], when, args)
    data = load()
    data[r["name"]] = {"when": when, "args": args}
    store(data)
    return "scheduled %s %s%s (%s)" % (r["name"], label(when), " " + " ".join(args) if args else "",
                                       {"windows": "Task Scheduler", "macos": "launchd", "linux": "systemd timer"}[D.OS])


def remove(name):
    data = load()
    key = next((k for k in data if k.lower() == name.lower()), None)
    if not key:
        raise Fail("%s has no schedule" % name, "lighting routine schedules")
    remove_os(key)
    del data[key]
    store(data)
    return "unscheduled %s" % key


def listing():
    data = load()
    if not data:
        return "no scheduled routines -> %s" % HELP
    return "\n".join("%s  %s%s" % (k, label(v["when"]), " " + " ".join(v["args"]) if v["args"] else "")
                     for k, v in sorted(data.items()))
