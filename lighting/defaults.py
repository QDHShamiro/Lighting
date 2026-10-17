import json
import os
import platform
import sys
from pathlib import Path

WIN = sys.platform == "win32"
MAC = sys.platform == "darwin"
LINUX = not WIN and not MAC
OS = "windows" if WIN else "macos" if MAC else "linux"
ROOT = Path(os.environ.get("LIGHTING_ROOT") or Path(__file__).resolve().parent.parent).resolve()
HOME = Path(os.environ.get("LIGHTING_HOME") or Path.home() / ".lighting")
VENV = HOME / "venv"
PY = VENV / "Scripts" / "python.exe" if WIN else VENV / "bin" / "python3"
PYW = VENV / "Scripts" / "pythonw.exe" if WIN else PY
STAMP = VENV / ".lighting-stamp"
USER = (os.environ.get("USERNAME") or os.environ.get("USER") or "user").lower()
PIPE = "\\\\.\\pipe\\lighting-" + USER if WIN else str(HOME / ("lighting-%s.sock" % USER))
ARCH = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
CLIENT = "lighting.exe" if WIN else "lighting-macos" if MAC else "lighting-linux-" + ARCH
CLIENT_NAME = "lighting.exe" if WIN else "lighting"
LOCAL_BIN = Path.home() / ".local" / "bin"
NO_WINDOW = 0x08000000 if WIN else 0
FFMPEG_HINT = "winget install Gyan.FFmpeg" if WIN else "brew install ffmpeg" if MAC else "sudo apt install ffmpeg"
KEY = HOME / "key"
OUT = HOME / "out"
LOG = HOME / "log.jsonl"
SESSIONS = HOME / "sessions.json"
BRAIN = HOME / "brain.json"
IMPROVE = HOME / "improve.json"
CONFIG = HOME / "config.json"
BLOCKLIST = HOME / "blocklist.txt"
EXT = HOME / "extension"
BIN = HOME / "bin"
HOST_BAT = HOME / ("host.bat" if WIN else "host.sh")
HOST_PY = HOME / "host.py"
HOST_NAME = "dev.qdhshamiro.lighting"
HOST_JSON = HOME / (HOST_NAME + ".json")
PIDFILE = HOME / "daemon.pid"
DAEMON_LOG = HOME / "daemon.log"

BROWSER_KEYS = {
    "brave": r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    "chrome": r"Software\Google\Chrome\NativeMessagingHosts",
    "edge": r"Software\Microsoft\Edge\NativeMessagingHosts",
    "chromium": r"Software\Chromium\NativeMessagingHosts",
}
_SUPPORT = Path.home() / "Library" / "Application Support"
_CONFIG = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
BROWSER_DIRS = {
    "brave": _SUPPORT / "BraveSoftware" / "Brave-Browser" if MAC else _CONFIG / "BraveSoftware" / "Brave-Browser",
    "chrome": _SUPPORT / "Google" / "Chrome" if MAC else _CONFIG / "google-chrome",
    "edge": _SUPPORT / "Microsoft Edge" if MAC else _CONFIG / "microsoft-edge",
    "chromium": _SUPPORT / "Chromium" if MAC else _CONFIG / "chromium",
}
if WIN:
    BROWSER_EXES = {
        "brave": [r"BraveSoftware\Brave-Browser\Application\brave.exe"],
        "chrome": [r"Google\Chrome\Application\chrome.exe"],
        "edge": [r"Microsoft\Edge\Application\msedge.exe"],
    }
elif MAC:
    BROWSER_EXES = {
        "brave": ["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"],
        "chrome": ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"],
        "edge": ["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"],
        "chromium": ["/Applications/Chromium.app/Contents/MacOS/Chromium"],
    }
else:
    BROWSER_EXES = {
        "brave": ["brave-browser", "brave"],
        "chrome": ["google-chrome", "google-chrome-stable"],
        "edge": ["microsoft-edge", "microsoft-edge-stable"],
        "chromium": ["chromium", "chromium-browser"],
    }
BROWSER_ORDER = ["brave", "chrome", "edge"] + ([] if WIN else ["chromium"])
BROWSER_PROCS = {"brave": "brave", "chrome": "chrome", "edge": "msedge", "chromium": "chromium"} if LINUX else {}

CLIENT_TIMEOUT = 180.0
CALL_TIMEOUT = 20.0
LOAD_TIMEOUT = 25.0
BOOT_WAIT = 10.0
BROWSER_START_WAIT = 20.0
RECONNECT_GRACE = 2.5

SNAP_LINES = 150
TEXT_CHARS = 6000
PDF_BYTES = 30_000_000
LEAK_QUERY = 200
OUT_CHARS = 12000
DIFF_LINES = 5
NAV_LINES = 40
NAV_CHARS = 1600
SHOT_WIDTH = 1024
SMALL_WIDTH = 768
JPEG_QUALITY = 70
OUT_KEEP_HOURS = 24
LOG_MAX_BYTES = 5_000_000

LONG_WAITS = {"inbox": 300_000, "reply": 300_000, "wait": 10_000, "listen": 900_000}
WEB_SCHEMES = {"http", "https", "file", "about", "chrome", "brave", "edge", "view-source", "data", "blob",
               "chrome-extension"}
RISKY_SCHEMES = {"file", "ms-msdt", "search-ms", "search", "ms-officecmd", "ms-word", "ms-excel", "ms-powerpoint",
                 "ms-visio", "ms-access", "ms-appinstaller", "ms-cxh", "ms-cxh-full", "hcp", "javascript", "vbscript"}
CHAT_POLL_S = 1.0
CHAT_GAP_S = 2.0
CHAT_UNANSWERED = 5
CHAT_TEXT = 300
CHAT_LINES = 5
KEYS = HOME / "keys.json"
KEY_HINT_EVERY_S = 7200
SUGGEST_EVERY_S = 600
PRUNE_DAYS = 14
SESSION_IDLE_S = 12 * 3600
SITES = HOME / "sites.json"
LISTEN_MAX_S = 300
LISTEN_TIMEOUT_S = 900
VIDEO_MAX_S = 60
VIDEO_FPS = 15
VIDEO_WIDTH = 1280
VIDEO_CRF = 30
SPAWN_TIMEOUT_S = 90
APP_WAIT_MS = 30000
APP_WAIT_OCR_S = 1.5
LISTEN_CHARS = 1800

IDLE_MS = 300
BLITZ_WAIT_MS = 800
BUSY_WAIT_S = 10
DONE_GRACE_S = 20
REPEAT_NOTE_AT = 3
REPEAT_BRAKE_AT = 5
REPEAT_BRAKE_MAX_S = 30
REPEAT_WINDOW_S = 120
HINT_LITERAL = 6
TYPE_SETTLE_S = 0.3
KEY_QUIET_S = 0.25
BRAIN_TOP = 15
BRAIN_PAGE_USES = 3
BRAIN_HOST_USES = 10
BRAIN_MIN_HITS = 3
BRAIN_MIN_HIDDEN = 5
BRAIN_FIX_SURE = 2
BRAIN_FIX_WINDOW_S = 90
BRAIN_SAVE_S = 30
BRAIN_MAX_PAGES = 400
BRAIN_MAX_ELS = 150
BRAIN_HALF_LIFE_D = 30
IMPROVE_EVERY_S = 7 * 86400
IMPROVE_MIN_FAILS = 10
DOCTOR_WAIT_S = 10
DOCTOR_HUNG_S = 60
DOCTOR_STUCK_S = 3600
DOCTOR_FAIL_PCT = 10
DOCTOR_DISK_MIN_GB = 2
DOCTOR_OUT_MAX_MB = 500
DOCTOR_OUT_KEEP_S = 3600
DOCTOR_LOG_MAX_MB = 20
DOCTOR_LOG_TAIL = 262144
DOCTOR_MEM_PCT = 85
DOCTOR_DAEMON_MAX_MB = 1024
DOCTOR_MCP_WAIT_S = 20
DOCTOR_RECONNECT_S = 8
DOCTOR_NET_S = 3
RELEASES_URL = "https://api.github.com/repos/QDHShamiro/Lighting/releases/latest"
DAEMON_STARTED = "lighting daemon started"
POINTER_MS = 1200
POINTER_COLOR = "#ff8a00"
HOTKEY_MODS = 0x0002 | 0x0001
HOTKEY_VK = 0x23
UIA_BUDGET_MS = 1500
OCR_UPSCALE_BELOW = 1200

CONFIG_DEFAULTS = {
    "browser": "auto",
    "pointer": True,
    "ocr_lang": "auto",
    "shot_width": SHOT_WIDTH,
    "learn": True,
    "cleanup": True,
    "audio": "auto",
    "window": True,
    "brain": True,
    "snap_top": BRAIN_TOP,
}

RISK_WORDS = [
    "buy", "purchase", "pay", "checkout", "place order", "order now", "subscribe",
    "delete", "remove account", "close account", "transfer", "wire",
    "merge pull request", "confirm merge", "squash and merge", "rebase and merge",
    "kaufen", "bezahlen", "zahlungspflichtig", "jetzt bestellen", "abonnieren",
    "löschen", "konto schließen", "überweisen",
]


def version():
    try:
        return json.loads((ROOT / ".claude-plugin" / "plugin.json").read_text("utf-8"))["version"]
    except (OSError, ValueError, KeyError):
        return "0.0.0"


def config():
    data = dict(CONFIG_DEFAULTS)
    try:
        data.update(json.loads(CONFIG.read_text("utf-8")))
    except (OSError, ValueError):
        pass
    return data


def save_config(data):
    HOME.mkdir(parents=True, exist_ok=True)
    CONFIG.write_text(json.dumps(data, indent=2), "utf-8")
