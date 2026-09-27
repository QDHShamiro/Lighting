import json
import os
from pathlib import Path

ROOT = Path(os.environ.get("LIGHTING_ROOT") or Path(__file__).resolve().parent.parent).resolve()
HOME = Path(os.environ.get("LIGHTING_HOME") or Path.home() / ".lighting")
VENV = HOME / "venv"
PY = VENV / "Scripts" / "python.exe"
PYW = VENV / "Scripts" / "pythonw.exe"
STAMP = VENV / ".lighting-stamp"
PIPE = "\\\\.\\pipe\\lighting-" + os.environ.get("USERNAME", "user").lower()
KEY = HOME / "key"
OUT = HOME / "out"
LOG = HOME / "log.jsonl"
CONFIG = HOME / "config.json"
BLOCKLIST = HOME / "blocklist.txt"
EXT = HOME / "extension"
BIN = HOME / "bin"
HOST_BAT = HOME / "host.bat"
HOST_PY = HOME / "host.py"
HOST_NAME = "dev.qdhshamiro.lighting"
HOST_JSON = HOME / (HOST_NAME + ".json")
PIDFILE = HOME / "daemon.pid"

BROWSER_KEYS = {
    "brave": r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    "chrome": r"Software\Google\Chrome\NativeMessagingHosts",
    "edge": r"Software\Microsoft\Edge\NativeMessagingHosts",
    "chromium": r"Software\Chromium\NativeMessagingHosts",
}
BROWSER_EXES = {
    "brave": [r"BraveSoftware\Brave-Browser\Application\brave.exe"],
    "chrome": [r"Google\Chrome\Application\chrome.exe"],
    "edge": [r"Microsoft\Edge\Application\msedge.exe"],
}
BROWSER_ORDER = ["brave", "chrome", "edge"]

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
LISTEN_CHARS = 1800

IDLE_MS = 300
BLITZ_WAIT_MS = 3000
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
