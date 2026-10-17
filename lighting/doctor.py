import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

from lighting import defaults as D
from lighting.procs import HIDDEN, biggest, classify, is_daemon, kill, memory_load, memory_mb, processes, top_of

if D.WIN:
    import ctypes
    import ctypes.wintypes as W
    import winreg

    u32 = ctypes.WinDLL("user32")
    u32.GetForegroundWindow.restype = W.HWND
    u32.GetWindowThreadProcessId.argtypes = [W.HWND, ctypes.POINTER(W.DWORD)]

OK, WARN, FAIL = "ok", "warn", "FAIL"
FIX_ORDER = ["venv", "files", "exe", "path", "data", "out", "log", "trace", "kill", "daemon", "ext", "agents"]
AREAS = ["install", "client", "files", "daemon", "procs", "browser", "mcp", "claude", "agents", "ocr", "sound", "data", "disk",
         "memory", "errors", "focus", "update"]
EXE_PATH = re.compile(r"([A-Za-z]:(?:\\\\|\\|/)[^\"'\n]*?lighting\.exe)", re.I) if D.WIN else \
    re.compile(r"(/[^\"'\n]*?/lighting(?:-macos|-linux-\w+)?)(?=[\"'\s])")
TRACEBACK = "Traceback (most recent call last)"
OCR_NAME = {"windows": "Windows OCR", "macos": "macOS Vision OCR", "linux": "Tesseract OCR"}[D.OS]


def read_text(path):
    try:
        return Path(path).read_text("utf-8", errors="replace")
    except OSError:
        return ""


def read_bytes(path):
    try:
        return Path(path).read_bytes()
    except OSError:
        return b""


def read_json(path):
    try:
        return json.loads(Path(path).read_text("utf-8"))
    except (OSError, ValueError):
        return None


def broken_json(path):
    try:
        json.loads(path.read_text("utf-8"))
    except OSError:
        return False
    except ValueError:
        return True
    return False


def digest(path):
    data = read_bytes(path)
    return hashlib.sha256(data).hexdigest() if data else ""


def same_path(a, b):
    try:
        return Path(a).resolve() == Path(b).resolve()
    except (OSError, TypeError, ValueError):
        return False


def tilde(path):
    s, home = str(path), str(Path.home())
    return "~" + s[len(home):] if s.lower().startswith(home.lower()) else s


def plural(n, word):
    return "%d %s%s" % (n, word, "" if n == 1 else "es" if word.endswith(("s", "sh")) else "s")


def span(s):
    s = int(s)
    return "%d s" % s if s < 120 else "%d min" % (s // 60) if s < 7200 else "%d h" % (s // 3600) if s < 172800 else "%d d" % (s // 86400)


def vtuple(v):
    return tuple(int(x) for x in re.findall(r"\d+", v or "")[:3])


def newer(a, b):
    return vtuple(a) > vtuple(b)


def pidfile_pid():
    try:
        return int(D.PIDFILE.read_text("utf-8").strip())
    except (OSError, ValueError):
        return None


def user_path_has(folder):
    if not D.WIN:
        from lighting import install
        link = D.LOCAL_BIN / "lighting"
        on_path = any(same_path(p, D.LOCAL_BIN) for p in os.environ.get("PATH", "").split(os.pathsep) if p)
        return on_path and link.is_symlink() and same_path(link, install.client_dst())
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as k:
            cur = winreg.QueryValueEx(k, "Path")[0]
    except OSError:
        return False
    want = str(folder).rstrip("\\").lower()
    return any(os.path.expandvars(p).rstrip("\\").lower() == want for p in cur.split(";") if p)


def reg_value(path):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path) as k:
            return winreg.QueryValueEx(k, "")[0]
    except OSError:
        return None


def ext_stale():
    src, dst = D.ROOT / "extension", D.EXT
    if not (src / "manifest.json").exists():
        return False
    if (read_json(dst / "manifest.json") or {}).get("version") != D.version():
        return True
    for p in src.rglob("*"):
        if p.is_file() and p.name != "manifest.json" and read_bytes(dst / p.relative_to(src)) != read_bytes(p):
            return True
    return False


def daemon_errors():
    try:
        with open(D.DAEMON_LOG, "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - D.DOCTOR_LOG_TAIL))
            text = f.read().decode("utf-8", "replace")
    except OSError:
        return 0, ""
    at = text.rfind(D.DAEMON_STARTED)
    text = text[at:] if at >= 0 else text
    n = text.count(TRACEBACK)
    if not n:
        return 0, ""
    rows = text[text.rfind(TRACEBACK):].splitlines()[1:]
    last = next((r.strip() for r in rows if re.match(r"^[A-Za-z_][\w.]*(?:Error|Exception|Exit|Interrupt)\b", r)), "")
    return n, last


def fetch_release():
    import ssl
    import truststore
    import urllib.request
    req = urllib.request.Request(D.RELEASES_URL, headers={"Accept": "application/vnd.github+json", "User-Agent": "lighting/" + D.version()})
    with urllib.request.urlopen(req, timeout=D.DOCTOR_NET_S, context=truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)) as r:
        return str(json.loads(r.read().decode("utf-8")).get("tag_name") or "").lstrip("vV")


def stamp_current():
    from lighting import boot
    return D.PY.exists() and read_text(D.STAMP) == boot.signature()


def ocr_languages():
    from lighting import ocr
    return ocr.languages()


def mcp_ping():
    from lighting import selftest
    box = []
    selftest.mcp_probe(box, 3)
    if isinstance(box[0], Exception):
        raise box[0]
    return box[0]


def ask(argv, timeout):
    from lighting import ipc
    t = time.perf_counter()
    try:
        res = ipc.request({"argv": argv, "sid": ""}, timeout)
    except TimeoutError:
        return None, 0.0, "timeout"
    except (OSError, EOFError, ValueError):
        return None, 0.0, "down"
    return res, (time.perf_counter() - t) * 1000, ""


class Doctor:
    def __init__(self, offline=False, update_row=None):
        self.rows = []
        self.offline = offline
        self.update_row = update_row
        self.info = None
        self.stamp_ok = False
        self.kills = []
        self.stale_agents = []
        self.bad_files = []
        self.plugin_key = "lighting@lighting"
        self.jobs = {}
        if not offline and update_row is None:
            self.spawn("update", fetch_release)
        if stamp_current():
            self.spawn("mcp", mcp_ping)
        self.spawn("ocr", ocr_languages)
        self.ps, self.names = processes()

    def spawn(self, name, fn):
        box = {}

        def go():
            try:
                box["v"] = fn()
            except Exception as e:
                box["e"] = e

        t = threading.Thread(target=go, daemon=True)
        t.start()
        self.jobs[name] = (t, box)

    def result(self, name, timeout):
        t, box = self.jobs[name]
        t.join(timeout)
        if t.is_alive():
            raise TimeoutError("no answer within %d s" % timeout)
        if "e" in box:
            raise box["e"]
        return box.get("v")

    def add(self, level, area, text, hint="", fix=()):
        self.rows.append((level, area, text, hint, (fix,) if isinstance(fix, str) else tuple(fix)))

    def check(self):
        for step in (self.install, self.client, self.files, self.daemon, self.procs, self.browser, self.claude,
                     self.agents, self.ocr, self.sound, self.data, self.disk, self.memory, self.errors, self.focus, self.mcp,
                     self.update):
            try:
                step()
            except Exception as e:
                self.add(FAIL, step.__name__, "check failed: %s: %s" % (type(e).__name__, str(e)[:120]))
        return self

    def running(self, browser):
        from lighting import install
        exe = install.browser_exe(browser)
        if exe and not D.WIN:
            from lighting import procs
            return procs.running(exe)
        return bool(exe) and os.path.basename(exe).lower() in self.names

    def install(self):
        from lighting import boot
        uv = shutil.which("uv") or next((p for p in boot.uv_paths() if os.path.isfile(p)), None)
        if not uv:
            self.add(WARN, "install", "uv not found: setup and updates cannot install Python packages",
                     "install uv: https://docs.astral.sh/uv/getting-started/installation/")
        if not D.PY.exists():
            self.add(FAIL, "install", "no Python environment in %s" % tilde(D.VENV), "creates it with uv", "venv")
            return
        here = same_path(sys.prefix, D.VENV)
        py = "%d.%d.%d" % sys.version_info[:3] if here else " ".join(run_out([str(D.PY), "--version"]).split()[1:2])
        stamp, want = read_text(D.STAMP).split("|"), boot.signature().split("|")
        if stamp[:1] != want[:1]:
            self.add(FAIL, "install", "Python packages do not match requirements.txt", "installs them", "venv")
        elif stamp[1:] != want[1:]:
            self.add(WARN, "install", "set up from %s, running %s" % (" ".join(stamp[1:][::-1]), " ".join(want[1:][::-1])),
                     "lighting setup")
        else:
            self.stamp_ok = True
            self.add(OK, "install", "Python %s, packages match requirements.txt%s" % (py, ", uv found" if uv else ""))

    def client(self):
        from lighting import install
        src, dst = install.client_src(), install.client_dst()
        if not src.exists():
            self.add(FAIL, "client", "%s is missing" % src, "reinstall the plugin: claude plugin install " + self.plugin_key)
            return
        n = len(self.rows)
        if digest(dst) != digest(src):
            self.add(WARN, "client", "%s differs from %s (new terminals and other AIs run it)" % (tilde(dst), src),
                     "copies it", "exe")
        if not user_path_has(D.BIN):
            where = D.BIN if D.WIN else D.LOCAL_BIN / "lighting"
            self.add(WARN, "client", "%s is not on your PATH: other AIs and new terminals do not find lighting" % tilde(where),
                     "adds it", "path")
        found = shutil.which("lighting")
        if found and not (same_path(found, src) or same_path(found, dst)):
            self.add(WARN, "client", "`lighting` starts %s, not Lighting's client" % found, "rename or delete that file")
        if not D.WIN:
            env = (read_json(Path.home() / ".claude" / "settings.json") or {}).get("env") or {}
            if (Path.home() / ".claude").exists() and env.get("LIGHTING_CLIENT") != D.CLIENT:
                self.add(WARN, "client", "Claude Code does not know the %s client: MCP and hooks fail" % D.OS,
                         "lighting setup, then restart Claude Code")
        if len(self.rows) == n:
            self.add(OK, "client", "%s current in %s and on PATH" % (dst.name, tilde(D.BIN)))

    def files(self):
        from lighting import install
        ext_id = install.extension_id(install.source_manifest())
        host = read_json(D.HOST_JSON) or {}
        bad = []
        if host.get("path") != str(D.HOST_BAT) or "chrome-extension://%s/" % ext_id not in (host.get("allowed_origins") or []):
            bad.append("host manifest")
        if read_bytes(D.HOST_BAT) != (install.HOST_BAT if D.WIN else install.HOST_SH) or read_bytes(D.HOST_PY) != install.HOST_PY:
            bad.append("%s/host.py" % D.HOST_BAT.name)
        root = read_text(D.HOME / "client-root").strip()
        if not root or not (Path(root) / "lighting" / "__init__.py").exists():
            bad.append("client-root")
        browsers = install.installed_browsers()
        if D.WIN:
            missing = [b for b in browsers if reg_value(D.BROWSER_KEYS[b] + "\\" + D.HOST_NAME) != str(D.HOST_JSON)]
        else:
            files = install.host_files()
            missing = [b for b in browsers if read_bytes(files[b]) != read_bytes(D.HOST_JSON)]
        if missing:
            bad.append(("registry for " if D.WIN else "host manifest for ") + ", ".join(missing))
        if bad:
            self.add(FAIL, "files", "browser link broken: " + ", ".join(bad), "rewrites it", "files")
        if ext_stale():
            self.add(WARN, "files", "extension copy in %s is older than Lighting" % tilde(D.EXT), "copies and reloads it", "ext")
        elif not bad:
            self.add(OK, "files", "native host registered for %s, extension copy current" % (", ".join(browsers) or "no browser"))

    def daemon(self):
        res, ms, err = ask(["doctor-info"], D.DOCTOR_WAIT_S)
        pid = pidfile_pid()
        note = ""
        if err == "down" and not any(p["pid"] == pid and is_daemon(p) for p in self.ps):
            from lighting import boot
            t = time.perf_counter()
            try:
                boot.connect_or_start().close()
            except SystemExit:
                self.add(FAIL, "daemon", "does not start", "read %s" % tilde(D.DAEMON_LOG))
                return
            note = "was not running, started in %d ms; " % ((time.perf_counter() - t) * 1000)
            self.wait_hosts()
            res, ms, err = ask(["doctor-info"], D.DOCTOR_WAIT_S)
        if err:
            self.add(FAIL, "daemon", "pid %s does not answer within %d s (hung, or busy with a long command)" % (pid, D.DOCTOR_WAIT_S),
                     "restarts it", "daemon")
            return
        info = res.get("info") or {}
        if not info:
            self.add(WARN, "daemon", "runs an old version without doctor support", "restarts it", "daemon")
            return
        self.info = info
        if info.get("version") != D.version() or not same_path(info.get("root"), D.ROOT):
            self.add(WARN, "daemon", "runs %s from %s, this is %s from %s" % (info.get("version"), info.get("root"), D.version(), D.ROOT),
                     "restarts it", "daemon")
        else:
            self.add(OK, "daemon", "%spid %d, %s, up %s, answers in %d ms" % (note, info["pid"], info["version"], span(info["up"]), ms))
        if not info.get("hotkey"):
            self.add(WARN, "daemon", "Ctrl+Alt+End is taken by another program: the stop hotkey does not work",
                     "close the program that uses it, then lighting stop")
        if info.get("trace") or (D.HOME / "trace").exists():
            self.add(WARN, "daemon", "profiling is on (~/.lighting/trace): daemon.log grows with every command",
                     "turns it off from the next daemon start", "trace")

    def wait_hosts(self):
        from lighting import install
        if not any(self.running(b) for b in install.installed_browsers()):
            return
        end = time.time() + D.DOCTOR_RECONNECT_S
        while time.time() < end:
            res, _, _ = ask(["doctor-info"], D.DOCTOR_WAIT_S)
            if res and any(h.get("alive") for h in (res.get("info") or {}).get("hosts") or []):
                return
            time.sleep(0.3)

    def procs(self):
        self.ps, self.names = processes()
        hosts = (self.info or {}).get("hosts") or []
        pids = [h.get("hostpid") for h in hosts]
        c = classify(self.ps, (self.info or {}).get("pid") or pidfile_pid(), pids if self.info and all(pids) else None)
        if c["hung"]:
            self.kills += [p for p, _, _ in c["hung"]]
            self.add(WARN, "procs", "%s stuck: %s" % (plural(len(c["hung"]), "lighting client"),
                                                    ", ".join("%s (%s)" % (a, span(age)) for _, a, age in c["hung"][:4])),
                     "ends them", "kill")
        if c["extra"]:
            self.kills += c["extra"]
            self.add(WARN, "procs", "%s left over from tests or an old version (pid %s)" % (
                plural(len(c["extra"]), "more daemon"), ", ".join(map(str, c["extra"][:4]))), "ends them", "kill")
        if c["orphans"]:
            self.kills += c["orphans"]
            self.add(WARN, "procs", "%s not connected to any browser" % plural(len(c["orphans"]), "host process"),
                     "ends them, the browser reconnects", "kill")
        if not (c["hung"] or c["extra"] or c["orphans"]):
            self.add(OK, "procs", "nothing stuck (%s)" % plural(c["hosts"], "browser host"))

    def browser(self):
        from lighting import install
        names = install.installed_browsers()
        if not names:
            self.add(FAIL, "browser", "no Brave, Chrome or Edge found", "install Brave or Chrome")
            return
        if self.info is None:
            self.add(WARN, "browser", "not checked, the daemon does not answer")
            return
        hosts = [h for h in self.info.get("hosts") or [] if h.get("alive")]
        brands = {h.get("brand") for h in hosts}
        for h in hosts:
            if h.get("ext") and h["ext"] != D.version():
                self.add(WARN, "browser", "%s runs extension %s, Lighting is %s" % (h["brand"], h["ext"], D.version()),
                         "reloads it", "ext")
        idle = [n for n in names if n not in brands and self.running(n)]
        pref = str(D.config().get("browser") or "auto").lower()
        if pref != "auto" and pref not in names:
            self.add(WARN, "browser", "config browser is %s, which is not installed" % pref, "lighting config browser auto")
        if hosts:
            self.add(OK, "browser", "connected: " + ", ".join("%s %s (extension %s)" % (
                h.get("brand"), str(h.get("browser") or "?").split(".")[0], h.get("ext") or "?") for h in hosts))
            if idle:
                self.add(WARN, "browser", "%s open without the Lighting extension" % ", ".join(idle), "lighting setup " + idle[0])
        elif idle:
            self.add(FAIL, "browser", "%s is open, but the Lighting extension is not connected" % idle[0], "lighting setup " + idle[0])
        else:
            self.add(OK, "browser", "no browser open: Lighting starts %s on the next web command" % (pref if pref != "auto" else names[0]))

    def mcp(self):
        if "mcp" not in self.jobs or self.info is None:
            return
        try:
            got = self.result("mcp", D.DOCTOR_MCP_WAIT_S)
        except TimeoutError:
            self.add(FAIL, "mcp", "lighting mcp did not answer within %d s" % D.DOCTOR_MCP_WAIT_S, "restarts the daemon", "daemon")
        except Exception as e:
            self.add(FAIL, "mcp", "lighting mcp failed: %s" % str(e)[:100], "lighting setup")
        else:
            self.add(OK, "mcp", "lighting mcp starts in %d ms, then %.1f ms per call" % got[:2])

    def claude(self):
        base = Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude")
        if not base.exists():
            return
        inst = read_json(base / "plugins" / "installed_plugins.json") or {}
        found = [(k, v) for k, vs in (inst.get("plugins") or {}).items() if k.split("@")[0] == "lighting" for v in (vs or [])]
        if not found:
            self.add(WARN, "claude", "Lighting is not installed as a Claude Code plugin",
                     "claude plugin marketplace add QDHShamiro/Lighting, then claude plugin install lighting@lighting")
            return
        key, e = sorted(found, key=lambda kv: kv[1].get("scope") != "user")[0]
        self.plugin_key = key
        path = Path(e.get("installPath") or "")
        settings = read_json(base / "settings.json") or {}
        n = len(self.rows)
        if (settings.get("enabledPlugins") or {}).get(key) is False:
            self.add(WARN, "claude", "plugin %s is turned off" % key, "claude plugin enable " + key)
        if not (path / ".claude-plugin" / "plugin.json").exists():
            self.add(FAIL, "claude", "Claude Code's copy of Lighting is missing (%s)" % path, "claude plugin install " + key)
            return
        ver = e.get("version") or "?"
        if ver != D.version():
            hint = '"%s" setup' % (path / "bin" / D.CLIENT) if newer(ver, D.version()) else \
                "claude plugin update %s, then restart Claude Code" % key
            self.add(WARN, "claude", "Claude Code loads Lighting %s (skill, hooks), the daemon runs %s" % (ver, D.version()), hint)
        hooks = read_text(path / "hooks" / "hooks.json")
        if "done --quiet" not in hooks or " suggest" not in hooks:
            self.add(WARN, "claude", "plugin hooks missing: tabs stay open after replies", "claude plugin install " + key)
        if settings.get("disableAllHooks"):
            self.add(WARN, "claude", "hooks are off (disableAllHooks): tabs stay open after replies, no routine hints",
                     "remove disableAllHooks from %s" % tilde(base / "settings.json"))
        perms = settings.get("permissions") or {}
        deny = [r for r in perms.get("deny") or [] if "lighting" in str(r).lower()]
        if deny:
            self.add(WARN, "claude", "deny rules block Lighting: " + ", ".join(deny[:3]), "remove them from " + tilde(base / "settings.json"))
        if not any(str(r).startswith("Bash(lighting") for r in perms.get("allow") or []) and perms.get("defaultMode") != "bypassPermissions":
            self.add(WARN, "claude", "Claude Code asks before every lighting command", "/lighting:setup")
        if not (path / "skills" / "lighting" / "SKILL.md").exists():
            self.add(WARN, "claude", "the lighting skill is missing", "claude plugin install " + key)
        if len(self.rows) == n:
            self.add(OK, "claude", "plugin %s %s on, hooks on, skill ready, no permission prompts" % (key, ver))

    def agents(self):
        from lighting import install
        good, stale = [], []
        for name, p in install.agent_files().items():
            paths = [x.replace("\\\\", "\\") for x in EXE_PATH.findall(read_text(p))]
            if paths:
                (stale if any(not os.path.exists(x) for x in paths) else good).append(name)
        self.stale_agents = stale
        if stale:
            self.add(WARN, "agents", "%s point to a Lighting client that is gone" % ", ".join(stale), "rewrites their entries", "agents")
        if good:
            self.add(OK, "agents", "also set up for: " + ", ".join(good))

    def ocr(self):
        try:
            tags = self.result("ocr", D.DOCTOR_MCP_WAIT_S)
        except Exception as e:
            self.add(FAIL, "ocr", OCR_NAME + " does not load: %s" % str(e)[:80], "reinstalls the Python packages", "venv")
            return
        if tags:
            self.add(OK, "ocr", OCR_NAME + " languages: " + ", ".join(tags))
        else:
            self.add(FAIL, "ocr", "no OCR language installed: read w2 and o-refs cannot read the screen",
                     "Windows Settings > Time & language > Language & region > add English")

    def sound(self):
        from lighting import audio
        keys = audio.keys()
        asr = "Groq" if audio.GROQ[0] in keys else "OpenAI" if audio.OPENAI[0] in keys else "local Whisper"
        if audio.ffmpeg():
            self.add(OK, "sound", "ffmpeg found, speech to text: captions, then %s" % asr)
        else:
            self.add(WARN, "sound", "ffmpeg not found: shot --seconds (screen video) fails, listen uploads bigger files",
                     D.FFMPEG_HINT)

    def data(self):
        rdir = D.HOME / "routines"
        routines = sorted(rdir.glob("*.json")) if rdir.exists() else []
        files = [D.CONFIG, D.SESSIONS, D.BRAIN, D.IMPROVE, D.KEYS, D.SITES, D.HOME / "apps.json", D.HOME / "bench-real.json"]
        self.bad_files = [p for p in files + routines if p.exists() and broken_json(p)]
        if self.bad_files:
            self.add(FAIL, "data", "unreadable, Lighting ignores them: " + ", ".join(p.name for p in self.bad_files[:5]),
                     "renames them to *.json.bad", "data")
        cfg = read_json(D.CONFIG)
        unknown = sorted(set(cfg) - set(D.CONFIG_DEFAULTS)) if isinstance(cfg, dict) else []
        if unknown:
            self.add(WARN, "data", "unknown keys in config.json: " + ", ".join(unknown), "lighting config lists the valid ones")
        if not self.bad_files:
            pages = len((read_json(D.BRAIN) or {}).get("pages") or {})
            self.add(OK, "data", "settings and learned data readable (%s, %d pages learned)" % (plural(len(routines), "routine"), pages))

    def disk(self):
        free = shutil.disk_usage(D.HOME).free / 1e9
        out = sum(p.stat().st_size for p in D.OUT.iterdir() if p.is_file()) / 1e6 if D.OUT.exists() else 0.0
        log = D.DAEMON_LOG.stat().st_size / 1e6 if D.DAEMON_LOG.exists() else 0.0
        n = len(self.rows)
        if free < D.DOCTOR_DISK_MIN_GB:
            self.add(WARN, "disk", "only %.1f GB free on %s: screenshots, videos and downloads can fail" % (free, D.HOME.anchor),
                     "free up space on " + D.HOME.anchor)
        if out > D.DOCTOR_OUT_MAX_MB:
            self.add(WARN, "disk", "%s holds %d MB" % (tilde(D.OUT), out), "deletes files older than %s" % span(D.DOCTOR_OUT_KEEP_S), "out")
        if log > D.DOCTOR_LOG_MAX_MB:
            self.add(WARN, "disk", "daemon.log is %d MB" % log, "empties it", "log")
        if len(self.rows) == n:
            self.add(OK, "disk", "%.0f GB free on %s, out %d MB, daemon.log %.1f MB" % (free, D.HOME.anchor, out, log))

    def memory(self):
        load, free = memory_load()
        own = memory_mb(self.info["pid"]) if self.info else 0
        if own > D.DOCTOR_DAEMON_MAX_MB:
            self.add(WARN, "memory", "the daemon uses %d MB" % own, "restarts it", "daemon")
        if load >= D.DOCTOR_MEM_PCT:
            top = ", ".join("%s %.1f GB" % (key, mb / 1024) for mb, key in biggest(self.names, self.ps, 3))
            self.add(WARN, "memory", "%d%% in use, %.1f GB free: Lighting and every app get slower; biggest: %s" % (load, free, top),
                     "close programs you do not need")
        elif own <= D.DOCTOR_DAEMON_MAX_MB:
            self.add(OK, "memory", "%d%% in use, daemon %d MB" % (load, own))

    def errors(self):
        from lighting import brain
        crashes, last = daemon_errors()
        if crashes:
            self.add(WARN, "errors", "%s inside the daemon since it started, last: %s" % (plural(crashes, "error"), last[:100] or "?"),
                     "report it with %s: github.com/QDHShamiro/Lighting/issues" % tilde(D.DAEMON_LOG))
        s = brain.summary(7)
        if not s["runs"]:
            self.add(OK, "errors", "no commands in the last 7 days")
            return
        rate = brain.pct(s["failed"], s["runs"])
        if s["failed"] >= D.IMPROVE_MIN_FAILS and rate >= D.DOCTOR_FAIL_PCT and s["patterns"]:
            self.add(WARN, "errors", "%.0f%% of %d commands failed in 7 days, most often: %s" % (rate, s["runs"], s["patterns"][0]["error"][:60]),
                     "lighting improve")
        else:
            self.add(OK, "errors", "%d commands in 7 days, %.1f%% failed" % (s["runs"], rate))

    def focus(self):
        if not D.WIN:
            acc = (self.info or {}).get("access") or {}
            missing = [k for k, v in acc.items() if v is False]
            if acc.get("error"):
                self.add(FAIL, "focus", "desktop access does not load: %s" % acc["error"], "lighting setup")
            elif missing and D.MAC:
                self.add(FAIL, "focus", "macOS blocks %s for the Lighting daemon: no app control" % " and ".join(missing),
                         "System Settings > Privacy & Security: allow your terminal app (and Brave/Chrome), then lighting stop")
            elif missing:
                self.add(WARN, "focus", "missing: %s" % ", ".join(missing),
                         "sudo apt install gir1.2-atspi-2.0 xclip (Wayland: ydotool wl-clipboard)")
            elif acc:
                self.add(OK, "focus", "desktop access: " + ", ".join(sorted(acc)))
            return
        from lighting import win
        hwnd = u32.GetForegroundWindow()
        if not hwnd:
            return
        pid = W.DWORD()
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        mine = (self.info or {}).get("pid") or os.getpid()
        if pid.value and win.elevated(pid.value) and not win.elevated(mine):
            self.add(WARN, "focus", "%s in front runs as administrator: Windows blocks Lighting's clicks and keys there" % (
                win.exe_of(pid.value) or "a system window"), "click another window, or start that app without admin rights")
        else:
            self.add(OK, "focus", "the window in front accepts input")

    def update(self):
        if self.update_row:
            self.rows.append(self.update_row)
            return
        if "update" not in self.jobs:
            return
        try:
            tag = self.result("update", D.DOCTOR_NET_S + 1)
        except Exception:
            self.add(OK, "update", "GitHub not reachable, update check skipped")
        else:
            if tag and newer(tag, D.version()):
                self.add(WARN, "update", "Lighting %s is out, you run %s" % (tag, D.version()),
                         "claude plugin marketplace update %s, then claude plugin update %s" % (self.plugin_key.split("@")[-1], self.plugin_key))
            else:
                self.add(OK, "update", "up to date (newest release %s)" % (tag or "?"))
        self.update_row = self.rows[-1]

    def fix(self):
        keys = {k for level, _, _, _, fx in self.rows if level != OK for k in fx}
        done = []
        for k in FIX_ORDER:
            if k in keys:
                try:
                    done.append("fixed " + getattr(self, "fix_" + k)())
                except Exception as e:
                    done.append("could not fix %s: %s: %s" % (k, type(e).__name__, str(e)[:120]))
        return done

    def fix_venv(self):
        from lighting import boot
        try:
            D.STAMP.unlink()
        except OSError:
            pass
        boot.ensure_venv()
        return "install: Python packages installed"

    def fix_files(self):
        from lighting import install
        install.refresh()
        return "files: native host files, registry and extension copy rewritten"

    def fix_exe(self):
        from lighting import install
        install.copy_exe()
        return "client: copied lighting.exe to " + tilde(D.BIN)

    def fix_path(self):
        from lighting import install
        install.add_to_path()
        return "client: added %s to PATH (new terminals see it)" % tilde(D.BIN)

    def fix_data(self):
        for p in self.bad_files:
            p.replace(p.with_name(p.name + ".bad"))
        return "data: moved aside " + ", ".join(p.name for p in self.bad_files)

    def fix_out(self):
        limit, n = time.time() - D.DOCTOR_OUT_KEEP_S, 0
        for p in D.OUT.iterdir():
            try:
                if p.is_file() and p.stat().st_mtime < limit:
                    p.unlink()
                    n += 1
            except OSError:
                pass
        return "disk: deleted %s from %s" % (plural(n, "old file"), tilde(D.OUT))

    def fix_log(self):
        with open(D.DAEMON_LOG, "r+b") as f:
            f.truncate(0)
        return "disk: emptied daemon.log"

    def fix_trace(self):
        (D.HOME / "trace").unlink(missing_ok=True)
        return "daemon: profiling off from the next daemon start"

    def fix_kill(self):
        for pid in self.kills:
            kill(pid)
        return "procs: ended %s" % plural(len(self.kills), "stuck process")

    def fix_daemon(self):
        from lighting import boot
        boot.stop_daemon()
        pid = (self.info or {}).get("pid") or pidfile_pid()
        ps, _ = processes()
        if pid and any(p["pid"] == pid for p in ps):
            kill(top_of(ps, pid, is_daemon) or pid)
        boot.connect_or_start().close()
        self.names = processes()[1]
        self.wait_hosts()
        res, _, _ = ask(["doctor-info"], D.DOCTOR_WAIT_S)
        return "daemon: restarted (pid %s)" % ((res or {}).get("info") or {}).get("pid", "?")

    def fix_ext(self):
        res, _, err = ask(["ext-reload"], D.DOCTOR_MCP_WAIT_S * 2)
        return "browser: " + ((res or {}).get("out") or err or "?")

    def fix_agents(self):
        from lighting import install
        table = install.agents()
        for name in self.stale_agents:
            table[name]()
        return "agents: rewrote " + ", ".join(self.stale_agents)


def run_out(args):
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=10, creationflags=HIDDEN).stdout or ""
    except (OSError, subprocess.SubprocessError):
        return ""


def render(rows):
    lines = ["lighting doctor %s | %s" % (D.version(), D.ROOT)]
    rank = lambda r: AREAS.index(r[1]) if r[1] in AREAS else len(AREAS)
    for level, area, text, hint, fix in sorted(rows, key=rank):
        line = "%-4s %-7s %s" % (level, area, text)
        if level != OK and fix:
            line += " -> lighting doctor --fix" + (" (%s)" % hint if hint else "")
        elif level != OK and hint:
            line += " -> fix: " + hint
        lines.append(line)
    fails = sum(1 for r in rows if r[0] == FAIL)
    warns = sum(1 for r in rows if r[0] == WARN)
    auto = sum(1 for r in rows if r[0] != OK and r[4])
    tail = "%s, %s, %d ok" % (plural(fails, "problem"), plural(warns, "warning"), len(rows) - fails - warns)
    lines.append(tail + (" | lighting doctor --fix repairs %d" % auto if auto else ""))
    return "\n".join(lines)


def rerun(argv):
    root = read_text(D.HOME / "client-root").strip()
    if os.environ.get("LIGHTING_DOCTOR_ROOT") or not root or same_path(root, D.ROOT) or \
            not (Path(root) / "lighting" / "doctor.py").exists():
        return None
    exe = D.PY if D.PY.exists() else Path(sys.executable)
    env = dict(os.environ, PYTHONPATH=root, LIGHTING_ROOT=root, LIGHTING_DOCTOR_ROOT=root)
    try:
        res = subprocess.run([str(exe), "-X", "utf8", "-m", "lighting"] + list(argv), stdout=subprocess.PIPE, env=env,
                             timeout=900, creationflags=HIDDEN, encoding="utf-8", errors="replace")
    except (OSError, subprocess.SubprocessError):
        return None
    return (res.stdout or "").rstrip(), res.returncode


def run(argv):
    flags = set(argv[1:])
    child = rerun(argv)
    if child is not None:
        return child
    doc = Doctor("--offline" in flags).check()
    code = lambda rows: 1 if any(r[0] == FAIL for r in rows) else 0
    if "--fix" not in flags:
        return render(doc.rows), code(doc.rows)
    done = doc.fix()
    if not done:
        return render(doc.rows) + "\nnothing that --fix can repair", code(doc.rows)
    again = Doctor(doc.offline, doc.update_row).check()
    return "\n".join(done + [render(again.rows)]), code(again.rows)


def main(argv):
    text, code = run(argv)
    print(text)
    return code
