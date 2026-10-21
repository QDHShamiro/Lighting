import io
import os
import re
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET
import zipfile

from lighting import defaults as D
from lighting.common import Fail, best_only, cap, terms, tiers

TOOLS = D.HOME / "platform-tools"
ZIP = "https://dl.google.com/android/repository/platform-tools-latest-%s.zip" % (
    "windows" if D.WIN else "darwin" if D.MAC else "linux")
ROLES = {"Button": "button", "ImageButton": "button", "CheckBox": "checkbox", "Switch": "checkbox",
         "ToggleButton": "checkbox", "RadioButton": "radio", "EditText": "edit", "AutoCompleteTextView": "edit",
         "Spinner": "combobox", "SeekBar": "slider", "TextView": "text", "ImageView": "image", "TabWidget": "tab"}
KEYS = {"back": 4, "home": 3, "recent": 187, "recents": 187, "apps": 187, "menu": 82, "enter": 66, "tab": 61,
        "delete": 67, "backspace": 67, "del": 67, "space": 62, "power": 26, "volup": 24, "volumeup": 24,
        "voldown": 25, "volumedown": 25, "mute": 164, "playpause": 85, "next": 87, "nexttrack": 87, "prev": 88,
        "prevtrack": 88, "up": 19, "down": 20, "left": 21, "right": 22, "search": 84, "camera": 27, "wake": 224,
        "sleep": 223, "esc": 111, "escape": 111}
HELP = ("lighting phone devices | snap [-f x] | tap p3|\"text\" | type \"text\" [--enter] | key back|home|enter | "
        "swipe up|down|left|right | shot | launch <app> | open <url> | apps [-f x] | setup")


class Phone:
    def __init__(self):
        self.serial = None
        self.refs = {}
        self.size = None


def state(ctx):
    if not hasattr(ctx, "phone"):
        ctx.phone = Phone()
    return ctx.phone


def adb_path():
    exe = "adb.exe" if D.WIN else "adb"
    found = shutil.which("adb")
    cands = [found] if found else []
    cands += [str(TOOLS / exe)]
    if D.WIN:
        cands.append(os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk", "platform-tools", exe))
    else:
        cands += [os.path.expanduser("~/Library/Android/sdk/platform-tools/adb"), os.path.expanduser("~/Android/Sdk/platform-tools/adb")]
    return next((c for c in cands if c and os.path.isfile(c)), None)


def install_tools():
    import ssl
    import urllib.request
    try:
        import truststore
        context = truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    except ImportError:
        context = None
    with urllib.request.urlopen(ZIP, timeout=60, context=context) as r:
        data = r.read()
    D.HOME.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        z.extractall(D.HOME)
    if not D.WIN:
        for name in ("adb", "fastboot"):
            p = TOOLS / name
            if p.exists():
                os.chmod(p, 0o755)
    return adb_path()


def adb(ctx, *args, timeout=20, raw=False, serial=True):
    exe = adb_path()
    if not exe:
        raise Fail("phone control needs adb (Android platform-tools)", "lighting phone setup (downloads them from Google)")
    cmd = [exe]
    st = state(ctx)
    if serial and st.serial:
        cmd += ["-s", st.serial]
    cmd += [str(a) for a in args]
    try:
        res = subprocess.run(cmd, capture_output=True, timeout=timeout, creationflags=D.NO_WINDOW)
    except subprocess.TimeoutExpired:
        raise Fail("adb did not answer within %d s" % timeout, "unplug and plug the phone in, then lighting phone devices")
    if raw:
        return res.stdout
    out = res.stdout.decode("utf-8", "replace")
    err = res.stderr.decode("utf-8", "replace").strip()
    if res.returncode and not out:
        if "unauthorized" in err:
            raise Fail("the phone has not allowed this computer yet", "unlock it and tap Allow on the USB debugging prompt")
        if "no devices" in err or "not found" in err:
            raise Fail("no phone connected", "plug it in with USB debugging on (Settings > Developer options)")
        raise Fail("adb: %s" % (err.splitlines() or ["exit %d" % res.returncode])[-1][:200])
    return out


def devices(ctx):
    out = adb(ctx, "devices", "-l", serial=False)
    rows = []
    for line in out.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2:
            model = next((p.split(":", 1)[1] for p in parts if p.startswith("model:")), "")
            rows.append((parts[0], parts[1], model))
    return rows


def pick(ctx, flags):
    st = state(ctx)
    if flags.get("device"):
        st.serial = flags["device"]
        return st.serial
    rows = [r for r in devices(ctx) if r[1] == "device"]
    if st.serial and any(r[0] == st.serial for r in rows):
        return st.serial
    if not rows:
        bad = devices(ctx)
        if any(r[1] == "unauthorized" for r in bad):
            raise Fail("the phone has not allowed this computer yet", "unlock it and tap Allow on the USB debugging prompt")
        raise Fail("no phone connected", "plug it in with USB debugging on (Settings > Developer options)")
    if len(rows) > 1:
        raise Fail("%d phones connected" % len(rows), "lighting phone snap --device %s" % rows[0][0])
    st.serial = rows[0][0]
    return st.serial


def bounds(text):
    m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", text or "")
    return tuple(int(x) for x in m.groups()) if m else (0, 0, 0, 0)


def dump(ctx):
    xml = adb(ctx, "exec-out", "uiautomator", "dump", "/dev/tty", timeout=30)
    start, end = xml.find("<?xml"), xml.rfind("</hierarchy>")
    if start < 0 or end < 0:
        xml = adb(ctx, "shell", "uiautomator dump /sdcard/lighting.xml >/dev/null && cat /sdcard/lighting.xml", timeout=30)
        start, end = xml.find("<?xml"), xml.rfind("</hierarchy>")
    if start < 0 or end < 0:
        raise Fail("the phone gave no screen tree (screen off or a secure screen?)", "lighting phone key wake, then snap")
    return ET.fromstring(xml[start:end + len("</hierarchy>")])


def own_text(node):
    return " ".join((node.get("text") or node.get("content-desc") or "").split())


def kind(node):
    cls = (node.get("class") or "").rsplit(".", 1)[-1]
    if cls in ROLES:
        return ROLES[cls]
    if node.get("checkable") == "true":
        return "checkbox"
    return "button" if node.get("clickable") == "true" or node.get("long-clickable") == "true" else "text"


def is_control(node):
    return (node.get("clickable") == "true" or node.get("checkable") == "true" or node.get("long-clickable") == "true"
            or kind(node) in ("edit", "slider"))


def inner(node, out, toggles):
    for kid in node.findall("node"):
        if kid.get("checkable") == "true":
            toggles.append(kid)
        elif is_control(kid) and own_text(kid):
            continue
        t = own_text(kid)
        if t and t not in out:
            out.append(t)
        inner(kid, out, toggles)


def describe(node):
    role = kind(node)
    name = own_text(node)
    texts, toggles = [], []
    if role not in ("edit",):
        inner(node, texts, toggles)
    if not name and texts:
        name = texts[0]
    sub = texts[1] if len(texts) > 1 and texts[0] == name else ""
    check = node if node.get("checkable") == "true" else (toggles[0] if toggles else None)
    if check is not None:
        role = "checkbox" if role in ("button", "text", "checkbox") else role
    if not name and node.get("resource-id") and role != "text":
        name = "#" + node.get("resource-id").rsplit("/", 1)[-1]
    extra = ""
    if check is not None:
        extra += " [x]" if check.get("checked") == "true" else " [ ]"
    if role == "edit":
        hint = node.get("hint") or ""
        if node.get("password") == "true":
            extra += " =***"
        elif hint and hint != name:
            extra += " (%s)" % hint[:30]
        if node.get("focused") == "true":
            extra += " (focused)"
    if node.get("selected") == "true":
        extra += " *"
    if node.get("enabled") == "false":
        extra += " (disabled)"
    if sub and len(sub) <= 60:
        extra += " - " + sub
    return role, name, extra, toggles


def snapshot(ctx, flags):
    st = state(ctx)
    root = dump(ctx)
    st.refs = {}
    filt = terms(flags.get("f"))
    lines, app = [], ""
    seen = set()

    def visit(node, inside):
        nonlocal app
        app = app or node.get("package") or ""
        l, t, r, b = bounds(node.get("bounds"))
        big = r - l >= 4 and b - t >= 4
        control = is_control(node) and id(node) not in seen
        mine = inside
        if big and control:
            role, name, extra, toggles = describe(node)
            seen.update(id(x) for x in toggles)
            if name or role in ("edit", "slider"):
                x, y = (l + r) // 2, (t + b) // 2
                if toggles and node.get("checkable") != "true":
                    tl, tt, tr, tb = bounds(toggles[0].get("bounds"))
                    if tr - tl >= 4 and tb - tt >= 4:
                        x, y = (tl + tr) // 2, (tt + tb) // 2
                add(role, name, extra, x, y)
                mine = True
        elif big and not inside and id(node) not in seen and node.get("scrollable") != "true":
            name = own_text(node)
            if name:
                add("text", name, "", (l + r) // 2, (t + b) // 2)
        for kid in node.findall("node"):
            visit(kid, mine)

    def add(role, name, extra, x, y):
        ref = "p%d" % (len(st.refs) + 1)
        st.refs[ref] = (x, y, name, role)
        label = name if len(name) <= 60 else name[:57] + "..."
        line = "%s %s%s%s" % (ref, role, ' "%s"' % label.replace('"', "'") if label else "", extra)
        tier = tiers(filt, name, role, extra) if filt else None
        if tier is None or any(tier):
            lines.append((tier, line))

    for top in root.findall("node"):
        visit(top, False)
    rows = best_only(lines) if filt else [l for _, l in lines]
    head = "[phone %s] %s (%d controls)" % (st.serial, app, len(st.refs))
    return cap(head + "\n" + ("\n".join(rows) or "(nothing found)"), "phone-snap", lines=D.SNAP_LINES)


def find(ctx, spec):
    st = state(ctx)
    if re.fullmatch(r"p\d+", spec):
        if spec not in st.refs:
            raise Fail("%s is unknown or from an older snapshot" % spec, "lighting phone snap")
        return st.refs[spec]
    snapshot(ctx, {})
    want = spec.lower()
    ranked = [(4 if v[2].lower() == want else tiers([want], v[2])[0], k, v) for k, v in st.refs.items()]
    top = max((r for r, _, _ in ranked), default=0)
    if not top:
        raise Fail('nothing called "%s" on the phone' % spec, "lighting phone snap")
    return next(v for r, _, v in ranked if r == top)


def screen_size(ctx):
    st = state(ctx)
    if not st.size:
        m = re.search(r"(\d+)x(\d+)", adb(ctx, "shell", "wm", "size"))
        st.size = (int(m.group(1)), int(m.group(2))) if m else (1080, 2400)
    return st.size


def type_text(ctx, text):
    if not text.isascii():
        raise Fail("adb can only type plain ASCII text", "type it in parts, or paste it on the phone")
    chunk = re.sub(r"([\\\"'`$&|;<>()*?!#~ ])", lambda m: "%s" if m.group(1) == " " else "\\" + m.group(1), text)
    adb(ctx, "shell", "input", "text", chunk)


def after(ctx, flags, what):
    time.sleep(0.6)
    if flags.get("f"):
        return what + "\n" + snapshot(ctx, flags)
    return what


def apps(ctx, flags):
    out = adb(ctx, "shell", "pm", "list", "packages", "-3")
    pk = sorted(l.split(":", 1)[1].strip() for l in out.splitlines() if l.startswith("package:"))
    if flags.get("f"):
        want = terms(flags["f"])
        pk = [p for p in pk if any(w in p.lower() for w in want)]
    return cap("\n".join(pk) or "no apps found", "phone-apps", lines=80)


def launch(ctx, name):
    if "." not in name:
        out = adb(ctx, "shell", "pm", "list", "packages")
        pk = [l.split(":", 1)[1].strip() for l in out.splitlines() if l.startswith("package:")]
        hits = [p for p in pk if name.lower() in p.lower()]
        hits.sort(key=lambda p: (not p.lower().endswith("." + name.lower()), len(p)))
        if not hits:
            raise Fail('no app matches "%s"' % name, "lighting phone apps -f %s" % name)
        name = hits[0]
    out = adb(ctx, "shell", "monkey", "-p", name, "-c", "android.intent.category.LAUNCHER", "1")
    if "No activities found" in out or "aborted" in out.lower():
        raise Fail("%s has no app to open" % name)
    return name


def cmd_phone(ctx, pos, flags):
    sub = pos[0].lower() if pos else "snap"
    rest = pos[1:]
    if sub == "setup":
        if adb_path():
            return "adb found: %s" % adb_path()
        return "downloaded Android platform-tools to %s" % (install_tools() or TOOLS)
    if sub == "devices":
        rows = devices(ctx)
        if not rows:
            return "no phone connected -> plug it in with USB debugging on (Settings > Developer options)"
        return "\n".join("%s %s %s" % r for r in rows)
    if sub == "help":
        return HELP
    pick(ctx, flags)
    if sub == "snap":
        return snapshot(ctx, flags)
    if sub in ("tap", "click"):
        if not rest:
            raise Fail("tap needs a ref or text", HELP)
        x, y, name, role = find(ctx, " ".join(rest))
        if flags.get("long"):
            adb(ctx, "shell", "input", "swipe", x, y, x, y, 700)
        else:
            adb(ctx, "shell", "input", "tap", x, y)
        return after(ctx, flags, 'ok (tapped %s "%s" at %d,%d)' % (role, name[:40], x, y))
    if sub == "type":
        if rest and re.fullmatch(r"p\d+", rest[0]):
            x, y, _, _ = find(ctx, rest[0])
            adb(ctx, "shell", "input", "tap", x, y)
            time.sleep(0.3)
            rest = rest[1:]
        text = ctx.secret if ctx.secret is not None else " ".join(rest)
        if text:
            type_text(ctx, text)
        if flags.get("enter"):
            adb(ctx, "shell", "input", "keyevent", KEYS["enter"])
        return after(ctx, flags, "ok (typed %d chars%s)" % (len(text), " +enter" if flags.get("enter") else ""))
    if sub in ("key", "press"):
        names = rest or ["back"]
        for k in names:
            code = KEYS.get(k.lower())
            if code is None and not k.isdigit():
                raise Fail("unknown phone key %s" % k, "back, home, recent, enter, delete, volup, voldown, power, playpause")
            adb(ctx, "shell", "input", "keyevent", code if code is not None else k)
        return after(ctx, flags, "ok (%s)" % " ".join(names))
    if sub in ("swipe", "scroll"):
        w, h = screen_size(ctx)
        way = (rest[0] if rest else "up").lower()
        cx, cy = w // 2, h // 2
        dx, dy = {"up": (0, -h // 3), "down": (0, h // 3), "left": (-w // 3, 0), "right": (w // 3, 0)}.get(way, (0, -h // 3))
        adb(ctx, "shell", "input", "swipe", cx - dx // 2, cy - dy // 2, cx + dx // 2, cy + dy // 2, 250)
        return after(ctx, flags, "ok (swiped %s)" % way)
    if sub in ("shot", "screenshot"):
        from PIL import Image
        from lighting.browser import save_image, threshold
        data = adb(ctx, "exec-out", "screencap", "-p", raw=True, timeout=30)
        img = Image.open(io.BytesIO(data))
        return save_image(ctx, img, "phone-%s" % state(ctx).serial, bool(flags.get("if-changed")),
                          int(flags.get("width") or D.SMALL_WIDTH), threshold(flags))
    if sub in ("launch", "start"):
        if not rest:
            raise Fail("launch needs an app", "lighting phone launch spotify")
        pkg = launch(ctx, " ".join(rest))
        return after(ctx, flags, "ok (started %s)" % pkg)
    if sub == "open":
        if not rest:
            raise Fail("open needs a link", "lighting phone open https://example.com")
        adb(ctx, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", rest[0])
        return after(ctx, flags, "ok (opened %s on the phone)" % rest[0][:80])
    if sub == "apps":
        return apps(ctx, flags)
    raise Fail("unknown phone command %s" % sub, HELP)
