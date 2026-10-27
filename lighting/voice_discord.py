import json
import os
import re
import subprocess
import sys
import time

from lighting.voice import (DEFAULTS, HIDDEN, HOME, LOG, SAY, STATE, alive, config, keys, log, loop, persona_file,
                            save_config, state)

NEEDED = ("listen_device", "discord_out")


def lighting(*args, timeout=40):
    from lighting import defaults as D
    exe = D.BIN / "lighting.exe"
    res = subprocess.run([str(exe if exe.exists() else "lighting")] + list(args), capture_output=True,
                         timeout=timeout, creationflags=HIDDEN)
    raw = res.stdout + res.stderr
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return raw.decode("cp1252", "replace")


def discord():
    for line in lighting("windows").splitlines():
        m = re.match(r"(w\d+) (.*) - Discord\.exe", line.strip())
        if m and "Updater" not in m.group(2):
            return m.group(1)
    raise RuntimeError("Discord is not open")


def controls(win):
    return lighting("snap", win, "--full", timeout=60)


def rows(snap):
    for line in snap.splitlines():
        m = re.match(r"\s*(d\d+) (\w+) \"([^\"]*)\"(.*)", line)
        if m:
            yield m.group(1), m.group(2), m.group(3), "*" in m.group(4)


def ref(snap, pattern, kind=None):
    return next((r for r in rows(snap) if re.search(pattern, r[2], re.I) and (not kind or r[1] == kind)), None)


def pick(win, menu, item, name):
    for attempt in range(3):
        lighting("snap", win)
        if ref(controls(win), "", "menuitem"):
            lighting("press", "escape")
            lighting("press", "escape")
            time.sleep(0.4)
        lighting("click", menu)
        opener = None
        for _ in range(4):
            time.sleep(0.6)
            opener = ref(controls(win), "^" + item)
            if opener:
                break
        if opener:
            break
        log("menu '%s' not open yet, again" % item)
        lighting("press", "escape")
        time.sleep(0.5)
    else:
        raise RuntimeError("Discord menu '%s' not found" % item)
    lighting("click", opener[0])
    for _ in range(4):
        time.sleep(0.6)
        snap = controls(win)
        if ref(snap, "", "radio"):
            break
    before = next((r[2] for r in rows(snap) if r[1] == "radio" and r[3]), "")
    target = name and (ref(snap, "^%s$" % re.escape(name), "radio") or ref(snap, re.escape(name), "radio"))
    if target and not target[3]:
        lighting("click", target[0])
    lighting("press", "escape")
    lighting("press", "escape")
    time.sleep(0.4)
    return before


def devices(win, mic, out):
    return {"mic": pick(win, "Eingabeoptionen", "Eingabegerät", mic),
            "out": pick(win, "Ausgabeoptionen", "Ausgabegerät", out)}


def restore(win, prev):
    for menu, item, key in (("Eingabeoptionen", "Eingabegerät", "mic"), ("Ausgabeoptionen", "Ausgabegerät", "out")):
        for attempt in range(3):
            if not prev.get(key):
                break
            pick(win, menu, item, prev[key])
            if pick(win, menu, item, "") == prev[key]:
                break
            log("restore %s did not stick, again" % key)


def chat(win, target):
    name = target.lstrip("@")
    name = config()["aliases"].get(name.lower(), name)
    if name.lower() in controls(win).splitlines()[0].lower():
        return
    for attempt in range(3):
        lighting("snap", win)
        box = ref(controls(win), "Quick Switcher")
        if not box:
            lighting("press", "ctrl+k")
            time.sleep(0.8)
            box = ref(controls(win), "Quick Switcher")
        if not box:
            continue
        lighting("type", box[0], name)
        time.sleep(1.5)
        lighting("press", "enter")
        time.sleep(1.5)
        if name.lower() in controls(win).splitlines()[0].lower():
            return
        lighting("press", "escape")
    raise RuntimeError("could not open the Discord chat '%s'" % name)


def call(win, target):
    server, _, channel = target.partition("/")
    snap = controls(win)
    where = ref(snap, r"Sprachchat verbunden|Sprachverbindung") and snap.splitlines()[0]
    if ref(snap, "^Verbindung trennen$"):
        log("already in a call (%s), staying there" % (where or "?"))
        return False
    if server.strip():
        chat(win, server.strip())
        snap = controls(win)
    btn = None
    if channel.strip():
        btn = ref(snap, r"^%s \(Sprachkanal\)" % re.escape(channel.strip()), "button")
        if not btn:
            raise RuntimeError("voice channel '%s' not found on this server" % channel.strip())
    btn = btn or (ref(snap, "^(Sprachanruf beitreten|Anruf beitreten|Beitreten|Sprachanruf starten)$", "button")
                  or ref(snap, r"\(Sprachkanal\), \S", "button"))
    if not btn:
        raise RuntimeError("no call button or busy voice channel in this chat")
    log("joining: " + btn[2])
    for attempt in range(4):
        time.sleep(1.2)
        snap = controls(win)
        if ref(snap, "^Verbindung trennen$"):
            return True
        enter = ref(snap, "^(Sprachchat beitreten|Sprachkanal beitreten|Beitreten)$", "button")
        if enter:
            lighting("click", enter[0])
    raise RuntimeError("clicked '%s' but Discord is not connected" % btn[2])
    lighting("click", btn[0])


def ringing(win):
    snap = controls(win)
    if not ref(snap, "(eingehend|ruft an|incoming)"):
        return None
    return ref(snap, "^(Annehmen|Anruf annehmen|Beitreten)$", "button")


def hangup(win):
    btn = ref(controls(win), "^Verbindung trennen$")
    if btn:
        lighting("click", btn[0])


def others():
    query = ("Get-CimInstance Win32_Process -Filter \"name like 'python%'\" | "
             "Where-Object { $_.CommandLine -like '*lighting.voice_discord session*' } | ForEach-Object { $_.ProcessId }")
    out = subprocess.run(["powershell", "-NoProfile", "-Command", query], capture_output=True, text=True,
                         creationflags=HIDDEN).stdout
    return [int(x) for x in out.split() if x.isdigit() and int(x) != os.getpid()]


def session(target, answered=None):
    for old in others():
        subprocess.run(["taskkill", "/PID", str(old), "/F"], capture_output=True, creationflags=HIDDEN)
        log("stopped an older session (pid %d)" % old)
    cfg = config()
    win = discord()
    prev, joined = {"mic": cfg["owner_mic"], "out": cfg["home_out"]}, False
    try:
        prev = devices(win, cfg["discord_mic"], cfg["discord_out"])
        if re.search("cable|" + re.escape(cfg["discord_out"]), prev.get("out", ""), re.I):
            prev["out"] = cfg["home_out"]
        if re.search("cable", prev.get("mic", ""), re.I):
            prev["mic"] = cfg["owner_mic"]
        state(prev=prev, win=win)
        if answered:
            lighting("click", answered)
            joined = True
        else:
            joined = call(win, target)
        time.sleep(1.5)
        loop(target)
    finally:
        try:
            if joined and current().get("hangup", True):
                hangup(win)
        finally:
            restore(win, prev)
            state(status="stopped")


def watch():
    state(watch_pid=os.getpid())
    while True:
        try:
            hit = ringing(discord())
            if hit:
                log("incoming call, answering")
                session("", answered=hit[0])
        except Exception as e:
            log("watch: %s" % str(e)[:160])
        time.sleep(3)


def spawn(*args):
    from lighting import defaults as D
    HOME.mkdir(parents=True, exist_ok=True)
    out = open(LOG, "a", encoding="utf-8")
    p = subprocess.Popen([sys.executable, "-W", "ignore", "-m", "lighting.voice_discord"] + list(args),
                         cwd=str(D.ROOT), stdout=out, stderr=out, creationflags=HIDDEN | 0x00000008)
    return p.pid


def current():
    try:
        return json.loads(STATE.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def drop(text):
    SAY.mkdir(parents=True, exist_ok=True)
    (SAY / ("%d.txt" % time.time_ns())).write_text(text, "utf-8")


def cmd_voice(ctx, pos, flags):
    from lighting import defaults as D
    from lighting.common import Fail
    if not D.WIN:
        raise Fail("voice works on Windows only for now (it talks through VB-Cable)")
    sub = pos[0] if pos else "status"
    cur = current()
    running = alive(cur.get("pid")) and cur.get("status") == "running"
    if sub in ("join", "here"):
        if running:
            raise Fail("already in a call (%s)" % (cur.get("target") or "here"), "lighting voice leave")
        missing = [n for n in ("GROQ_API_KEY", "GEMINI_API_KEY") if n not in keys()]
        if missing:
            raise Fail("missing " + ", ".join(missing), "add it to ~/.config/watch/.env")
        unset = [n for n in NEEDED if not config().get(n)]
        if unset:
            raise Fail("voice needs your audio devices first: " + ", ".join(unset),
                       "lighting voice set listen_device=<what Discord plays to> discord_out=<the same device>")
        target = " ".join(pos[1:]) if sub == "join" else ""
        if sub == "join" and not target:
            raise Fail("join needs a chat name", 'lighting voice join "Friends"')
        state(status="starting", target=target)
        return "voice: joining %s (pid %d), check with: lighting voice status" % (
            target or "the open chat", spawn("session", target))
    if sub == "leave":
        if not running:
            return "voice: not in a call"
        drop("__leave__")
        prev = cur.get("prev") or {}
        return "voice: leaving, Discord goes back to %s / %s" % (prev.get("mic", "?"), prev.get("out", "?"))
    if sub == "say":
        if not running:
            raise Fail("not in a call", 'lighting voice join "Name"')
        drop(" ".join(pos[1:]))
        return "voice: queued"
    if sub == "watch":
        on = (pos[1] if len(pos) > 1 else "on") != "off"
        if alive(cur.get("watch_pid")):
            subprocess.run(["taskkill", "/PID", str(cur["watch_pid"]), "/F"], capture_output=True, creationflags=HIDDEN)
            state(watch_pid=0)
        return "voice: answering calls %s" % ("on (pid %d)" % spawn("watch") if on else "off")
    if sub == "set":
        cfg = config()
        for kv in pos[1:]:
            k, _, v = kv.partition("=")
            if k not in DEFAULTS:
                raise Fail("unknown setting '%s'" % k, "settings: " + ", ".join(DEFAULTS))
            d = DEFAULTS[k]
            cfg[k] = v.lower() in ("1", "on", "true", "ja") if isinstance(d, bool) else type(d)(v)
        save_config(cfg)
        return "voice: saved " + ", ".join(pos[1:])
    if sub in ("persona", "prompt"):
        call = " ".join(pos[1:])
        path = persona_file(call)
        return "voice persona for %s: %s (%s; it is used only in this call, other calls get a neutral one)" % (
            call or "the open chat", path.as_posix(), "exists" if path.exists() else "missing, write it there")
    lines = ["voice: %s%s" % ("in a call with " + (cur.get("target") or "the open chat") if running else
                              "not in a call" + (" (%s)" % cur["status"] if str(cur.get("status", "")).startswith("crash") else ""),
                              ", answering calls" if alive(cur.get("watch_pid")) else "")]
    if running:
        lines += ["heard: " + (cur.get("last_heard") or "-"), "said: " + (cur.get("last_said") or "-")]
    try:
        lines += ["log:"] + LOG.read_text("utf-8", errors="replace").splitlines()[-6:]
    except OSError:
        pass
    return "\n".join(lines)


if __name__ == "__main__":
    try:
        if sys.argv[1:2] == ["watch"]:
            watch()
        else:
            session(sys.argv[2] if len(sys.argv) > 2 else "")
    except Exception as e:
        log("crashed: %s: %s" % (type(e).__name__, e))
        state(status="crashed: %s" % str(e)[:120])
        raise
