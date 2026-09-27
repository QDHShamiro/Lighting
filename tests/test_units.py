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


def test_confirmed_steps_are_not_learned():
    class Ctx:
        class d:
            events = []

    def pair(yes):
        a, b = issue_episode("Bug in snap"), issue_episode("Crash on launch")
        for ep in (a, b):
            ep.update(via=None, cost=100)
            if yes:
                ep["steps"][-1]["flags"] = {"yes": True}
        return a, b

    a, b = pair(True)
    assert routines.learn(Ctx(), b, [a]) is None
    a, b = pair(False)
    learned = routines.learn(Ctx(), b, [a])
    assert learned is not None and learned["params"] == {"title": "Crash on launch"}
    routines.path(learned["name"]).unlink()


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


def test_video_times_and_scenes():
    from PIL import Image
    from lighting import browser
    assert browser.seconds("1:30") == 90 and browser.seconds("1:02:03") == 3723 and browser.seconds(None) is None
    assert browser.clock(65) == "1:05" and browser.clock(3723) == "1:02:03"
    black, white, grey = (Image.new("RGB", (64, 36), c) for c in ("black", "white", (120, 120, 120)))
    assert browser.distinct([black, black.copy(), white, black.copy()], 2) == [0, 2]
    assert browser.distinct([black, grey, white], 3) == [0, 1, 2]


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


def test_times_and_keys():
    from lighting import keys
    from lighting.common import parse_ms
    assert parse_ms("10m", 0) == 600000 and parse_ms("90s", 0) == 90000 and parse_ms("1500", 0) == 1500
    assert parse_ms(None, 7) == 7 and parse_ms(True, 5) == 5
    assert keys.norm("Strg+Umschalt+N") == "ctrl+shift+n" and keys.norm("g then d") == "g d"
    assert keys.norm("Alt, F") == "alt+f" and keys.norm("Control+K") == "ctrl+k"
    assert keys.from_name("Search (Ctrl+K)") == ("Ctrl+K", "Search") and keys.from_name("Search") is None
    keys.learn("discord.exe", "Ctrl+K", 'd93 edit "Where to?"')
    assert keys.known("discord.exe")["ctrl+k"]["src"] == "learned"
    assert "win+e" not in keys.known("discord.exe") and "win+e" in keys.known("windows")
    first = keys.hint("discord.exe")
    assert "ctrl+k" in first and keys.hint("discord.exe") == ""
    if WINDOWS:
        from lighting import win
        assert win.combo_parts("ctrl++") == ["ctrl", "+"] and win.combo_parts("ctrl+shift+a") == ["ctrl", "shift", "a"]
        assert win.vk_of("playpause")[0] == 0xB3 and win.vk_of("arrowdown")[0] == 0x28 and win.vk_of("numpad5")[0] == 0x65


def test_owner_prefers_the_exe():
    if not WINDOWS:
        return
    from lighting import desktop
    wins = [{"exe": "WindowsTerminal.exe", "title": "Obsidian lighting session", "fg": True},
            {"exe": "Obsidian.exe", "title": "Vault - Obsidian 1.13", "fg": False}]
    assert desktop.owner_of(wins, "Obsidian")["exe"] == "Obsidian.exe"
    assert desktop.owner_of(wins[:1], "Obsidian")["exe"] == "WindowsTerminal.exe"


def test_app_links_and_risky_schemes():
    from lighting import browser
    from lighting import defaults as D
    assert browser.normalize_url("localhost:3000/x") == "http://localhost:3000/x"
    assert browser.normalize_url("obsidian://open?vault=V").split(":")[0] not in D.WEB_SCHEMES
    assert browser.normalize_url("github.com").startswith("https://")
    assert {"ms-msdt", "search-ms", "file"} <= D.RISKY_SCHEMES and "spotify" not in D.RISKY_SCHEMES


def test_chat_rows_and_own_messages():
    from lighting import chat
    rows = [{"key": k, "num": n, "name": k} for k, n in (("a", 10), ("b", 11), ("c", 12))]
    assert [r["key"] for r in chat.fresh_rows(chat.baseline(None, rows[:2]), rows)] == ["c"]
    hashed = [dict(r, num=None) for r in rows]
    assert [r["key"] for r in chat.fresh_rows(chat.baseline(None, hashed[:2]), hashed)] == ["c"]
    st = {"sent": [chat.norm("Hello  there")]}
    assert chat.said_by_me(st, "me: hello there") and chat.said_by_me(st, "hello there")
    assert not chat.said_by_me(st, "peer: ok, 11 chars")
    assert chat.is_stamp("16:27") and chat.is_stamp("Sonntag, 27. September 2026 16:27")
    assert not chat.is_stamp("itsluiss (ItsLuis) 16:13") and chat.short_sender("itsluiss (ItsLuis)") == "itsluiss"
    assert chat.LINK_RE.search("see https://x.io") and not chat.LINK_RE.search("wie gehts dir so")


def test_recall_finds_routines_by_words():
    import time
    from lighting import recall
    r = {"name": "discord-markieren", "tags": ["markiere", "@"], "params": {"user": "Luis"},
         "steps": [{"cmd": "launch", "args": ["discord"], "where": {"kind": "app", "exe": "Discord.exe"}}]}
    stems = lambda text: {recall.stem(w) for w in recall.words(text)}
    assert recall.fits(stems("markiere Tom auf Discord"), r)
    assert not recall.fits(stems("fix the minecraft plugin"), r)
    prompts = []
    recall.note_prompt(prompts, "markiere Luis auf Discord bitte")
    assert recall.tags_for(prompts, time.time(), {"user": "Luis"}) == ["markiere", "discord"]


def test_routines_cleanup():
    import time
    old = routines.new_routine({"params": {}, "steps": [{"cmd": "open", "args": ["x.com"]}], "check": None}, "learned", 10)
    old.update(name="old-unused", created="2020-01-01 10:00")
    routines.save(old)
    routines.save(dict(old, name="old-saved", source="saved"))
    routines.prune()
    names = {r["name"] for r in routines.load_all()}
    assert "old-unused" not in names and "old-saved" in names
    routines.path("old-saved").unlink()
    step = lambda url: {"cmd": "open", "args": [url], "flags": {}, "where": None}
    nav = {"steps": [step("a.com"), step("b.com")], "via": None, "start": None, "cost": 10, "t0": time.time()}

    class C:
        class d:
            events = []

    assert routines.learn(C(), nav, [nav]) is None
    assert routines.slug("Öffne Spotify") == "oeffne-spotify"


def test_repeated_app_lines_fold():
    if not WINDOWS:
        return
    from lighting import desktop
    lines = ['d1 listitem "a"'] + ['d%d button "Reply"' % i for i in range(2, 7)] + ['d9 button "Send"']
    out, folded = desktop.fold(lines)
    assert folded == 4 and out[1] == 'd2 button "Reply" (x5, same in each item)' and out[-1] == 'd9 button "Send"'
    assert desktop.fold(lines[:3])[1] == 0


def test_search_url_templates():
    from lighting import sites
    assert sites.derive("https://www.youtube.com/results?search_query=lofi+beats&sp=x", "lofi beats") == \
        "https://www.youtube.com/results?search_query={q}"
    assert sites.derive("https://example.com/find/lighting%20docs", "lighting docs") == "https://example.com/find/{q}"
    assert sites.derive("https://example.com/?q=a&x=a", "a") is None
    assert sites.build("https://x.com/s?q={q}", "a b") == "https://x.com/s?q=a+b"
    assert sites.named("youtube") == "youtube.com" and sites.named("paper") is None
    assert sites.template("m.youtube.com").startswith("https://www.youtube.com/results")


def test_unread_patterns():
    from lighting import chat
    dms, at, rest = chat.unread_of(["DEV arbteit , 10 ungelesene Nachrichten", "2 Erwähnungen • NOVASMP | S2",
                                    "1 Erwähnung CrystalBox", "Ungelesene Nachrichten, Stellar, Sprachanruf aktiv",
                                    "Friends, 3 unread messages", "2 mentions General", "Quiet room"])
    assert dms == [(10, "DEV arbteit"), (3, "Friends")]
    assert sorted(at) == [(1, "CrystalBox"), (2, "General"), (2, "NOVASMP | S2")] and at[-1] == (1, "CrystalBox")
    assert rest == ["Stellar"]


def test_captions_and_noise():
    from lighting import audio
    vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000 align:start\nHello there\n\n00:00:02.000 --> 00:00:04.000\n" \
          "Hello there\n<c>general</c> Kenobi\n\n1\n00:01:05.500 --> 00:01:07.000\n[Music]\n"
    segs = audio.vtt_segments(vtt)
    assert audio.lines_of(segs) == ["0:00 Hello there", "0:02 general Kenobi"]
    assert audio.spoken({"no_speech_prob": 0.1, "avg_logprob": -0.3}) and not audio.spoken({"no_speech_prob": 0.9})
    assert audio.clock(3723) == "1:02:03"


def test_session_contexts_and_nested_runs():
    from lighting import commands

    class D:
        events = []
        ctxs = {}
        abort_gen = 0

        class abort:
            @staticmethod
            def is_set():
                return False

    a, b = commands.Context(D(), "a"), commands.Context(D(), "b")
    a.tabs, b.tabs = ["t1"], ["t2"]
    assert a.tabs != b.tabs and a.ep is None and b.owned == set()
    r = {"name": "loop", "params": {}, "steps": [{"cmd": "run", "args": ["loop"], "flags": {}}], "check": None,
         "stats": {"runs": 0, "ok": 0, "fail": 0, "ms": 0, "saved": 0, "recent": []}, "source": "saved"}
    routines.save(r)
    try:
        a.run_stack = ["loop"]
        try:
            routines.cmd_run(a, ["loop"], {})
            assert False, "a routine that calls itself must fail"
        except Exception as e:
            assert "calls itself" in str(e)
    finally:
        routines.path("loop").unlink()


def test_learned_steps_reuse_routines():
    st = lambda c, *a: {"cmd": c, "args": list(a), "flags": {}, "where": None}
    inner = {"name": "gh-open", "params": {"page": "Lighting"}, "flaky": False,
             "steps": [st("open", "github.com/QDHShamiro/{page}"), st("click", "Issues")]}
    steps = [st("open", "github.com/QDHShamiro/Tools"), st("click", "Issues"), st("fill", "Title={title}")]
    out = routines.compact(steps, [inner])
    assert [s["cmd"] for s in out] == ["run", "fill"] and out[0]["args"] == ["gh-open", "page=Tools"]
    assert routines.compact(steps[1:], [inner]) == steps[1:]


def test_same_steps_as_a_saved_routine_are_not_learned_again():
    import time

    class C:
        class d:
            events = []

    step = lambda c, a: {"cmd": c, "args": [a], "flags": {}, "where": {"kind": "web", "url": "https://x.com/a", "title": "A"}}
    saved = routines.new_routine({"params": {}, "steps": [step("open", "x.com/a"), step("click", "Go")], "check": None},
                                 "saved", 10)
    saved["name"] = "x-go"
    routines.save(saved)
    try:
        ep = {"steps": [step("open", "x.com/b"), step("click", "Stop")], "via": None, "start": step("open", "")["where"],
              "cost": 10, "t0": time.time()}
        assert routines.learn(C(), ep, [ep]) is None
    finally:
        routines.path("x-go").unlink()


def test_json_shape():
    from lighting.browser import shape
    s = shape({"items": [{"id": 1, "name": "a"}], "total": 2, "meta": {"q": "x"}})
    assert s.startswith("items[] (1) {id: 1, name: \"a\"}") and "total: 2" in s


def test_skill_and_mcp_budget():
    from lighting import mcp
    skill = (ROOT / "skills" / "lighting" / "SKILL.md").read_bytes()
    assert len(skill) <= 3300, "SKILL.md grew to %d bytes (budget 3300)" % len(skill)
    assert len(mcp.DESCRIPTION) <= 700, "MCP description grew to %d chars" % len(mcp.DESCRIPTION)


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
