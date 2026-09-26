import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
os.environ["LIGHTING_HOME"] = tempfile.mkdtemp(prefix="lighting-test-")
os.environ["LIGHTING_ROOT"] = str(ROOT)
sys.path.insert(0, str(ROOT))

from lighting import commands, install, routines
from lighting.common import best_only, terms, tiers

WINDOWS = sys.platform == "win32"


def test_filter_tiers():
    assert terms(" Save | CANCEL |") == ["save", "cancel"]
    assert tiers(["mit"], "MIT license") == [3]
    assert tiers(["lic"], "MIT license") == [2]
    assert tiers(["mit"], "commits") == [1]
    assert tiers(["zzz"], "commits") == [0]
    rows = [(tiers(["sos"], n), n) for n in ("Sosa La M", "SOS", "Chaos")]
    assert best_only([r for r in rows if any(r[0])]) == ["SOS"]


def test_steps_and_tokens():
    assert commands.split_steps('fill "Note=a;b"; click Go ;; press Enter') == ['fill "Note=a;b"', "click Go", "press Enter"]
    assert commands.tokenize('type "Search box" hello world') == ["type", "Search box", "hello", "world"]
    back = chr(92)
    assert commands.tokenize("fill Path=C:" + back + "Users" + back + "x") == ["fill", "Path=C:" + back + "Users" + back + "x"]
    assert commands.parse(["open", "x.com", "-n", "--timeout", "5", "--f=a|b"]) == (
        "open", ["x.com"], {"new": True, "timeout": "5", "f": "a|b"})


def page(url, title):
    return {"kind": "web", "url": url, "title": title}


def issue_episode(title):
    w = page("https://github.com/QDHShamiro/Lighting/issues/new", "New issue")
    return {"start": page("https://github.com/QDHShamiro/Lighting/issues", "Issues"), "steps": [
        {"cmd": "open", "args": ["github.com/QDHShamiro/Lighting/issues"], "flags": {}, "where": w},
        {"cmd": "click", "args": ["New issue"], "flags": {}, "where": w},
        {"cmd": "fill", "args": ["Title=" + title], "flags": {}, "where": w},
        {"cmd": "click", "args": ["Create"], "flags": {}, "where": page("https://github.com/x/1", title)},
    ]}


def test_learn_from_two_runs():
    a, b = issue_episode("Bug in snap"), issue_episode("Crash on launch")
    blk = routines.align(a["steps"], b["steps"])
    assert blk is not None
    body = routines.build(a["steps"], b, blk)
    assert body["params"] == {"title": "Crash on launch"}
    assert ["Title={title}"] in [s["args"] for s in body["steps"]]
    assert body["check"]["has"] == ["{title}"]
    assert routines.covers(body, issue_episode("Third one")["steps"]) == {"title": "Third one"}
    assert routines.align(a["steps"][:1], b["steps"][:1]) is None


def test_param_names():
    used = set()
    fill = {"cmd": "fill", "args": ["Search query=cats"]}
    assert routines.param_name("", fill, "cats", used) == "search_query"
    assert routines.param_name("", {"cmd": "open", "args": ["x"]}, "x", {"page"}) == "page2"
    assert routines.param_name("https", {"cmd": "click", "args": ["x"]}, "x", used) == "item"
    assert routines.fill("go {a} {b}", {"a": "1"}) == "go 1 {b}"


def test_config_merge_keeps_line_endings():
    d = Path(tempfile.mkdtemp())
    for eol in ("\n", "\r\n"):
        toml = d / ("c%d.toml" % len(eol))
        toml.write_bytes(("model = \"x\"" + eol + eol + "[mcp_servers.lighting]" + eol + "command = \"old\"" + eol).encode())
        install.merge_toml(toml)
        install.merge_toml(toml)
        raw = toml.read_bytes().decode()
        assert raw.count("[mcp_servers.lighting]") == 1 and "old" not in raw and 'model = "x"' in raw
        assert ("\r\n" in raw) == (eol == "\r\n") and raw.replace("\r\n", "").count("\r") == 0
    js = d / "mcp.json"
    js.write_text('{"mcpServers": {"other": {"command": "x"}}}', "utf-8")
    install.merge_json(js, "mcpServers", {"command": "lighting.exe", "args": ["mcp"]})
    data = __import__("json").loads(js.read_text("utf-8"))
    assert set(data["mcpServers"]) == {"other", "lighting"}


def test_sources_have_no_hidden_bytes():
    bad = {chr(0), chr(0xFFFE), chr(0xFFFF), chr(0xFEFF)}
    files = list((ROOT / "lighting").glob("*.py")) + list((ROOT / "extension").glob("*.js")) + list((ROOT / "tests").glob("*.py"))
    assert len(files) > 20
    for f in files:
        text = f.read_text("utf-8")
        assert not bad & set(text), "%s has a NUL, BOM or noncharacter" % f.name
        if f.suffix == ".py":
            compile(text, str(f), "exec")


def test_run_dialog_names():
    if not WINDOWS:
        return
    from lighting import desktop
    assert desktop.run_name("calc") == "calc.exe"
    assert desktop.run_name("notepad.exe") == "notepad.exe"
    assert desktop.run_name("definitely-not-an-app-7f3") is None
    assert desktop.run_name("calc ..") is None
    assert desktop.run_name("..\\calc") is None


class FakeWin:
    def __init__(self, stubborn):
        self.open, self.stubborn, self.front = {1, 2, 3}, stubborn, None

    def alive(self, h):
        return h in self.open

    visible = alive

    def close(self, h):
        if h not in self.stubborn:
            self.open.discard(h)

    def set_foreground(self, h):
        self.front = h


class Ctx:
    target = None


def test_cleanup_closes_only_launched_windows():
    if not WINDOWS:
        return
    from lighting import desktop
    real, fake = desktop.win, FakeWin(stubborn={2})
    desktop.win = fake
    try:
        ctx = Ctx()
        st = desktop.state(ctx)
        st.launched, st.front0, st.hwnd = {1: "Rechner", 2: "Editor"}, 3, 1
        ctx.target = ("app", 1)
        closed, left = desktop.cleanup(ctx)
        assert closed == ["Rechner"] and left == ["Editor"]
        assert fake.open == {2, 3} and fake.front == 3
        assert st.launched == {} and st.hwnd is None and ctx.target is None
        assert desktop.cleanup(ctx) == ([], [])
        st.launched = {3: "Paint"}
        assert desktop.keep(ctx) == 1 and st.launched == {}
    finally:
        desktop.win = real


if __name__ == "__main__":
    tests = [(n, f) for n, f in sorted(globals().items()) if n.startswith("test_") and callable(f)]
    failed = 0
    for name, fn in tests:
        try:
            fn()
            print("PASS", name)
        except Exception as e:
            failed += 1
            print("FAIL", name, "|", type(e).__name__, e)
    print("%d/%d passed" % (len(tests) - failed, len(tests)))
    sys.exit(1 if failed else 0)
