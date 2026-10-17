import base64
import hashlib
import json
import os
import re
import shutil
import stat
import sys
from pathlib import Path

if sys.platform == "win32":
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
HOST_SH = b'#!/bin/sh\nd=$(dirname "$0")\nexec "$d/venv/bin/python3" -X utf8 "$d/host.py" "$@"\n'
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


def host_manifest(ext_id):
    return {
        "name": D.HOST_NAME,
        "description": "Lighting native host",
        "path": str(D.HOST_BAT),
        "type": "stdio",
        "allowed_origins": ["chrome-extension://%s/" % ext_id] if ext_id else [],
    }


def write_host(ext_id):
    write_if_changed(D.HOST_BAT, HOST_BAT if D.WIN else HOST_SH)
    if not D.WIN:
        os.chmod(D.HOST_BAT, 0o755)
    write_if_changed(D.HOST_PY, HOST_PY)
    write_if_changed(D.HOST_JSON, json.dumps(host_manifest(ext_id), indent=2).encode("utf-8"))


def host_files():
    return {name: base / "NativeMessagingHosts" / (D.HOST_NAME + ".json") for name, base in D.BROWSER_DIRS.items()}


def register():
    done = []
    if not D.WIN:
        data = D.HOST_JSON.read_bytes() if D.HOST_JSON.exists() else b""
        for name, path in host_files().items():
            if not path.parent.parent.exists() and name not in installed_browsers():
                continue
            try:
                path.parent.mkdir(parents=True, exist_ok=True)
                write_if_changed(path, data)
                done.append(name)
            except OSError:
                pass
        return done
    for name, path in D.BROWSER_KEYS.items():
        try:
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, path + "\\" + D.HOST_NAME) as k:
                winreg.SetValueEx(k, "", 0, winreg.REG_SZ, str(D.HOST_JSON))
            done.append(name)
        except OSError:
            pass
    return done


def unregister():
    if not D.WIN:
        for path in host_files().values():
            try:
                path.unlink()
            except OSError:
                pass
        return
    for path in D.BROWSER_KEYS.values():
        try:
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, path + "\\" + D.HOST_NAME)
        except OSError:
            pass


def client_src():
    return D.ROOT / "bin" / D.CLIENT


def client_dst():
    return D.BIN / D.CLIENT_NAME


def copy_exe():
    src, dst = client_src(), client_dst()
    if not src.exists() or src.resolve() == dst.resolve():
        return
    data = src.read_bytes()
    try:
        if dst.read_bytes() == data:
            return
    except OSError:
        pass
    D.BIN.mkdir(parents=True, exist_ok=True)
    old = dst.with_suffix(".old")
    for step in (old.unlink, lambda: dst.replace(old)):
        try:
            step()
        except OSError:
            pass
    dst.write_bytes(data)
    if not D.WIN:
        os.chmod(dst, os.stat(dst).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def link_unix():
    link = D.LOCAL_BIN / "lighting"
    try:
        if link.is_symlink() and os.readlink(link) == str(client_dst()):
            return False
        if link.exists() and not link.is_symlink():
            return False
        D.LOCAL_BIN.mkdir(parents=True, exist_ok=True)
        if link.is_symlink():
            link.unlink()
        link.symlink_to(client_dst())
        return True
    except OSError:
        return False


def add_to_path():
    if not D.WIN:
        return link_unix()
    import ctypes
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0, winreg.KEY_READ | winreg.KEY_WRITE) as k:
        try:
            cur, kind = winreg.QueryValueEx(k, "Path")
        except OSError:
            cur, kind = "", winreg.REG_EXPAND_SZ
        parts = [p for p in cur.split(";") if p]
        if any(p.rstrip("\\").lower() == str(D.BIN).lower() for p in parts):
            return False
        winreg.SetValueEx(k, "Path", 0, kind, ";".join(parts + [str(D.BIN)]))
    ctypes.windll.user32.SendMessageTimeoutW(0xFFFF, 0x1A, 0, "Environment", 2, 2000, None)
    return True


def mcp_entry():
    return {"command": str(client_dst()), "args": ["mcp"]}


def eol_of(path):
    crlf = chr(13) + chr(10)
    try:
        return crlf if crlf.encode() in path.read_bytes() else chr(10)
    except OSError:
        return chr(10)


def merge_json(path, key, entry):
    try:
        data = json.loads(path.read_text("utf-8"))
    except FileNotFoundError:
        data = {}
    except ValueError:
        raise SystemExit("%s is not valid JSON, fix it or add the server by hand" % path)
    data.setdefault(key, {})["lighting"] = entry
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2), "utf-8", newline=eol_of(path))


def merge_toml(path):
    exe = str(client_dst()).replace("\\", "\\\\")
    block = '[mcp_servers.lighting]\ncommand = "%s"\nargs = ["mcp"]\n' % exe
    try:
        text = path.read_text("utf-8")
    except FileNotFoundError:
        text = ""
    eol = eol_of(path)
    text = re.sub(r"(?ms)^\[mcp_servers\.lighting\]\n.*?(?=^\[|\Z)", "", text).rstrip()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((text + "\n\n" if text else "") + block, "utf-8", newline=eol)


def bionic_skill():
    return Path.home() / ".lmstudio" / "skills" / "lighting"


def copy_skill(dst):
    shutil.copytree(D.ROOT / "skills" / "lighting", dst, dirs_exist_ok=True)


def antigravity_configs():
    gem = Path.home() / ".gemini"
    return [gem / "config" / "mcp_config.json", gem / "antigravity" / "mcp_config.json"]


def antigravity_skills():
    gem = Path.home() / ".gemini"
    return [gem / "skills" / "lighting", gem / "antigravity" / "global_skills" / "lighting",
            gem / "antigravity" / "skills" / "lighting", gem / "antigravity-cli" / "skills" / "lighting"]


def antigravity():
    main, *old = antigravity_configs()
    merge_json(main, "mcpServers", mcp_entry())
    for p in old:
        if p.exists():
            merge_json(p, "mcpServers", mcp_entry())
    first, *rest = antigravity_skills()
    copy_skill(first)
    for p in rest:
        if p.parent.parent.exists():
            copy_skill(p)


def agent_files():
    home = Path.home()
    if D.WIN:
        appdata = Path(os.environ.get("APPDATA") or home / "AppData" / "Roaming")
        zed = appdata / "Zed" / "settings.json"
    elif D.MAC:
        appdata = home / "Library" / "Application Support"
        zed = home / ".config" / "zed" / "settings.json"
    else:
        appdata = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
        zed = appdata / "zed" / "settings.json"
    vsext = appdata / "Code" / "User" / "globalStorage"
    return {
        "antigravity": antigravity_configs()[0],
        "claude-desktop": appdata / "Claude" / "claude_desktop_config.json",
        "cline": vsext / "saoudrizwan.claude-dev" / "settings" / "cline_mcp_settings.json",
        "codex": home / ".codex" / "config.toml",
        "cursor": home / ".cursor" / "mcp.json",
        "gemini": home / ".gemini" / "settings.json",
        "lmstudio": home / ".lmstudio" / "mcp.json",
        "roo": vsext / "rooveterinaryinc.roo-cline" / "settings" / "mcp_settings.json",
        "vscode": appdata / "Code" / "User" / "mcp.json",
        "windsurf": home / ".codeium" / "windsurf" / "mcp_config.json",
        "zed": zed,
    }


def lmstudio():
    merge_json(agent_files()["lmstudio"], "mcpServers", mcp_entry())
    copy_skill(bionic_skill())


def agents():
    f = agent_files()
    return {
        "antigravity": antigravity,
        "bionic": lmstudio,
        "claude-desktop": lambda: merge_json(f["claude-desktop"], "mcpServers", mcp_entry()),
        "cline": lambda: merge_json(f["cline"], "mcpServers", dict(mcp_entry(), disabled=False)),
        "codex": lambda: merge_toml(f["codex"]),
        "cursor": lambda: merge_json(f["cursor"], "mcpServers", mcp_entry()),
        "gemini": lambda: merge_json(f["gemini"], "mcpServers", mcp_entry()),
        "lmstudio": lmstudio,
        "roo": lambda: merge_json(f["roo"], "mcpServers", mcp_entry()),
        "vscode": lambda: merge_json(f["vscode"], "servers", dict(mcp_entry(), type="stdio")),
        "windsurf": lambda: merge_json(f["windsurf"], "mcpServers", mcp_entry()),
        "zed": lambda: merge_json(f["zed"], "context_servers", dict(mcp_entry(), source="custom")),
    }


def install_agent(name):
    table = agents()
    if name not in table:
        raise SystemExit("usage: lighting install <%s>" % "|".join(table))
    from lighting import boot
    boot.ensure_venv()
    refresh()
    table[name]()
    print("lighting MCP server added to %s, restart it to load the tool" % name)
    return 0


def refresh():
    D.HOME.mkdir(parents=True, exist_ok=True)
    D.OUT.mkdir(parents=True, exist_ok=True)
    if not D.BLOCKLIST.exists():
        D.BLOCKLIST.write_text(DEFAULT_BLOCKLIST, "utf-8")
    ext_id = copy_extension() or extension_id(source_manifest())
    write_host(ext_id)
    register()
    try:
        copy_exe()
        if bionic_skill().exists():
            copy_skill(bionic_skill())
        for p in antigravity_skills():
            if p.exists():
                copy_skill(p)
    except OSError:
        pass
    return ext_id


def browser_exe(name):
    if D.MAC:
        return next((p for p in D.BROWSER_EXES.get(name, []) if os.path.exists(p)), None)
    if D.LINUX:
        return next((shutil.which(p) for p in D.BROWSER_EXES.get(name, []) if shutil.which(p)), None)
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
    try:
        if add_to_path():
            where = D.BIN if D.WIN else D.LOCAL_BIN / "lighting"
            print("added %s to your PATH (new terminals and other AIs find `lighting`)" % where)
    except OSError as e:
        print("could not add %s to PATH: %s" % (D.BIN, e))
    if not D.WIN:
        if claude_env():
            print("Claude Code: LIGHTING_CLIENT=%s in ~/.claude/settings.json (MCP and hooks use it, restart Claude Code)" % D.CLIENT)
        if D.LOCAL_BIN.as_posix() not in os.environ.get("PATH", ""):
            print("note: %s is not on your PATH, add it to your shell profile" % D.LOCAL_BIN)
    connected = connected_lines()
    if not connected and not manual and browsers:
        how = {"windows": "Windows UI Automation", "macos": "macOS Accessibility", "linux": "AT-SPI"}[D.OS]
        print("loading the extension by itself (%s, takes ~10 s) ..." % how, flush=True)
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


def claude_env(path=None):
    path = path or Path.home() / ".claude" / "settings.json"
    if not path.parent.exists():
        return False
    try:
        data = json.loads(path.read_text("utf-8"))
    except FileNotFoundError:
        data = {}
    except ValueError:
        return False
    env = data.setdefault("env", {})
    if env.get("LIGHTING_CLIENT") == D.CLIENT:
        return False
    env["LIGHTING_CLIENT"] = D.CLIENT
    path.write_text(json.dumps(data, indent=2), "utf-8", newline=eol_of(path))
    return True


def uninstall(purge):
    from lighting import boot
    boot.stop_daemon()
    unregister()
    if purge:
        for p in (D.OUT, D.EXT, D.BIN, D.HOME / "routines"):
            shutil.rmtree(p, ignore_errors=True)
        for p in (D.HOST_BAT, D.HOST_PY, D.HOST_JSON, D.KEY, D.LOG, D.CONFIG, D.PIDFILE, D.STAMP, D.HOME / "client-root",
                  D.HOME / "episodes.jsonl", D.HOME / "apps.json", D.HOME / "bench-real.json"):
            try:
                p.unlink()
            except OSError:
                pass
        print("removed the browser link and %s (the venv stays, delete %s by hand if wanted)" % (D.HOME, D.VENV))
    else:
        print("removed the browser link, stopped daemon (data kept in %s, add --purge to remove)" % D.HOME)
    return 0


def main(argv):
    if argv[0] == "uninstall":
        return uninstall("--purge" in argv)
    try:
        if argv[0] == "install":
            return install_agent(argv[1] if len(argv) > 1 else "")
        pick = next((a for a in argv[1:] if a in D.BROWSER_EXES), None)
        return setup(manual="--manual" in argv, browser=pick)
    except SystemExit as e:
        print(e, file=sys.stderr)
        return 1
