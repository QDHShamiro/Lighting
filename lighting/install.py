import base64
import hashlib
import json
import shutil
import sys
import winreg

from lighting import defaults as D

DEFAULT_BLOCKLIST = """paypal.com
paypal.me
stripe.com
checkout.stripe.com
klarna.com
sofort.com
giropay.de
paysafecard.com
wise.com
revolut.com
n26.com
bunq.com
sparkasse.de
dkb.de
ing.de
comdirect.de
commerzbank.de
deutsche-bank.de
postbank.de
consorsbank.de
volksbank.de
vr.de
targobank.de
santander.de
hypovereinsbank.de
bankofamerica.com
chase.com
wellsfargo.com
"""


def extension_id(manifest):
    key = manifest.get("key")
    if not key:
        return None
    digest = hashlib.sha256(base64.b64decode(key)).hexdigest()[:32]
    return "".join(chr(ord("a") + int(c, 16)) for c in digest)


def source_manifest():
    try:
        return json.loads((D.ROOT / "extension" / "manifest.json").read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def copy_extension():
    src = D.ROOT / "extension"
    if not (src / "manifest.json").exists():
        return None
    shutil.copytree(src, D.EXT, dirs_exist_ok=True)
    manifest = json.loads((D.EXT / "manifest.json").read_text("utf-8"))
    manifest["version"] = D.version()
    (D.EXT / "manifest.json").write_text(json.dumps(manifest, indent=2), "utf-8")
    return extension_id(manifest)


HOST_BAT = b'@"%~dp0venv\\Scripts\\python.exe" -X utf8 "%~dp0host.py" %* & exit /b\r\n'
HOST_PY = b"""import pathlib
import sys

sys.path.insert(0, (pathlib.Path(__file__).parent / "client-root").read_text("utf-8").strip())
from lighting import host

host.main()
"""


def write_if_changed(path, data):
    try:
        if path.read_bytes() == data:
            return
    except OSError:
        pass
    path.write_bytes(data)


def write_host(ext_id):
    write_if_changed(D.HOST_BAT, HOST_BAT)
    write_if_changed(D.HOST_PY, HOST_PY)
    host = {
        "name": D.HOST_NAME,
        "description": "Lighting native host",
        "path": str(D.HOST_BAT),
        "type": "stdio",
        "allowed_origins": ["chrome-extension://%s/" % ext_id] if ext_id else [],
    }
    write_if_changed(D.HOST_JSON, json.dumps(host, indent=2).encode("utf-8"))


def register():
    done = []
    for name, path in D.BROWSER_KEYS.items():
        try:
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, path + "\\" + D.HOST_NAME) as k:
                winreg.SetValueEx(k, "", 0, winreg.REG_SZ, str(D.HOST_JSON))
            done.append(name)
        except OSError:
            pass
    return done


def unregister():
    for path in D.BROWSER_KEYS.values():
        try:
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, path + "\\" + D.HOST_NAME)
        except OSError:
            pass


def refresh():
    D.HOME.mkdir(parents=True, exist_ok=True)
    D.OUT.mkdir(parents=True, exist_ok=True)
    if not D.BLOCKLIST.exists():
        D.BLOCKLIST.write_text(DEFAULT_BLOCKLIST, "utf-8")
    ext_id = copy_extension() or extension_id(source_manifest())
    write_host(ext_id)
    register()
    return ext_id


def browser_exe(name):
    import os
    bases = [os.environ.get("PROGRAMFILES", r"C:\Program Files"),
             os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)"),
             os.environ.get("LOCALAPPDATA", "")]
    for rel in D.BROWSER_EXES.get(name, []):
        for base in bases:
            path = os.path.join(base, rel) if base else ""
            if path and os.path.exists(path):
                return path
    return None


def installed_browsers():
    return [name for name in D.BROWSER_ORDER if browser_exe(name)]


def connected_lines():
    from lighting import boot
    try:
        res = boot.request({"argv": ["status"]}, 10)
    except (OSError, SystemExit, TimeoutError, ValueError):
        return []
    return [l for l in res.get("out", "").splitlines() if l.startswith("browser ") and "none connected" not in l]


def setup(manual=False, browser=None):
    from lighting import boot
    boot.ensure_venv()
    ext_id = refresh()
    browsers = installed_browsers()
    print("lighting %s set up" % D.version())
    print("native host registered for: %s" % ", ".join(register()))
    print("browsers found: %s" % (", ".join(browsers) or "none"))
    connected = connected_lines()
    if not connected and not manual and browsers:
        print("loading the extension by itself (Windows UI Automation, takes ~10 s) ...", flush=True)
        try:
            res = boot.request({"argv": ["autoload"] + ([browser] if browser else [])}, 150)
            print(res.get("out", ""))
        except (OSError, SystemExit, TimeoutError, ValueError) as e:
            print("automatic load failed: %s" % e)
        connected = connected_lines()
    if connected:
        print("\n".join(connected))
        print("ready: try `lighting open https://example.com`")
        return 0
    print("one manual step left, once per browser:")
    print("  1. open brave://extensions (or chrome://extensions, edge://extensions)")
    print("  2. switch on Developer mode")
    print("  3. Load unpacked -> %s" % D.EXT)
    print("extension id %s | then run: lighting status" % (ext_id or "?"))
    return 0


def uninstall(purge):
    from lighting import boot
    boot.stop_daemon()
    unregister()
    if purge:
        for p in (D.OUT, D.EXT):
            shutil.rmtree(p, ignore_errors=True)
        for p in (D.HOST_BAT, D.HOST_PY, D.HOST_JSON, D.KEY, D.LOG, D.CONFIG, D.PIDFILE, D.STAMP, D.HOME / "client-root"):
            try:
                p.unlink()
            except OSError:
                pass
        print("removed registry keys and %s (the venv stays, delete %s by hand if wanted)" % (D.HOME, D.VENV))
    else:
        print("removed registry keys, stopped daemon (data kept in %s, add --purge to remove)" % D.HOME)
    return 0


def main(argv):
    if argv[0] == "uninstall":
        return uninstall("--purge" in argv)
    try:
        pick = next((a for a in argv[1:] if a in D.BROWSER_EXES), None)
        return setup(manual="--manual" in argv, browser=pick)
    except SystemExit as e:
        print(e, file=sys.stderr)
        return 1
