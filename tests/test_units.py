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


def pr_episode(title, body):
    w = page("https://github.com/x/y/compare/main...dev?expand=1", "Compare")
    return {"start": w, "steps": [
        {"cmd": "open", "args": ["https://github.com/x/y/compare/main...dev?expand=1"], "flags": {}, "where": w},
        {"cmd": "type", "args": ["Add a title *", title], "flags": {}, "where": w},
        {"cmd": "type", "args": ["Comment", body], "flags": {}, "where": w},
        {"cmd": "click", "args": ["Create pull request"], "flags": {}, "where": page("https://github.com/x/y/pull/1", title)},
    ]}


def test_typed_values_become_whole_named_params():
    a, b = pr_episode("Lighting 0.9.0: sound", "short"), pr_episode("Lighting 1.0.0: own windows", "other")
    body = routines.build(a["steps"], b, routines.align(a["steps"], b["steps"]))
    assert body["params"].get("title") == "Lighting 1.0.0: own windows" and body["params"].get("comment") == "other"
    long_a, long_b = pr_episode("A", "x" * 90), pr_episode("B", "x" * 90)
    blk = routines.align(long_a["steps"], long_b["steps"])
    assert any(routines.long_literal(s) for s in routines.build(long_a["steps"], long_b, blk)["steps"])
    assert routines.label_name("Add a title *") == "title"


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
        assert raw.count("[mcp_servers.lighting]") == 1 and '"old"' not in raw and 'model = "x"' in raw, repr(raw)
        assert ("\r\n" in raw) == (eol == "\r\n") and raw.replace("\r\n", "").count("\r") == 0, repr(raw)
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

    def foreground(self):
        return 1


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


def test_navigation_only_routines_are_not_learned():
    import time

    class C:
        class d:
            events = []

    step = lambda c, a, u: {"cmd": c, "args": a, "flags": {}, "where": {"kind": "web", "url": u, "title": "T"}}
    h, o, o2 = "http://127.0.0.1:8769/", "http://127.0.0.1:8769/outer.html", "http://127.0.0.1:8769/outer2.html"
    ep = lambda steps: {"steps": steps, "via": None, "start": step("open", [""], h)["where"], "cost": 300, "t0": time.time()}
    a = ep([step("open", [h], h), step("open", [h], h), step("open", [o], o)])
    b = ep([step("open", [h], h), step("open", [o], o), step("open", [h], h), step("open", [o2], o2),
            step("run", ["selftest-rec", "email=x"], o2)])
    blk = routines.align(a["steps"], b["steps"])
    assert blk is not None
    body = routines.compact(routines.build(a["steps"], b, blk)["steps"], [])
    assert all(s["cmd"] in routines.NAV_ONLY for s in body)
    assert routines.learn(C(), b, [a]) is None


def test_json_shape():
    from lighting.browser import shape
    s = shape({"items": [{"id": 1, "name": "a"}], "total": 2, "meta": {"q": "x"}})
    assert s.startswith("items[] (1) {id: 1, name: \"a\"}") and "total: 2" in s


def test_sessions_survive_a_restart():
    from lighting import daemon, desktop

    class Fake:
        events, abort_gen, saved = [], 0, None
        session_rows, save_sessions = daemon.Daemon.session_rows, daemon.Daemon.save_sessions
        load_sessions, note_epoch = daemon.Daemon.load_sessions, daemon.Daemon.note_epoch

    f = Fake()
    f.ctxs, f.epochs = {}, {"brave": 5}
    c = daemon.Daemon.context(f, commands, "s1")
    c.tabs, c.owned = ["t4"], {"t4"}
    desktop.state(c).windows = {"w2": 1234}
    f.save_sessions()
    g = Fake()
    g.ctxs, g.epochs = {}, {}
    g.load_sessions(commands)
    r = g.ctxs["s1"]
    assert (r.group, r.tabs, r.owned, desktop.state(r).windows, g.epochs) == (c.group, ["t4"], {"t4"}, {"w2": 1234}, {"brave": 5})
    g.note_epoch("brave", 6)
    assert r.tabs == [] and r.owned == set()


def test_session_windows_keep_close_done():
    from lighting import browser, commands, daemon, desktop

    class Fake:
        ctxs = {}
        events = []
        abort_gen = 0

    f = Fake()
    a, b, n = (daemon.Daemon.context(f, commands, s) for s in ("sa", "sb", ""))
    assert (a.group, b.group, n.group) == ("Lighting #1", "Lighting #2", "Lighting")
    del f.ctxs["sa"]
    assert daemon.Daemon.context(f, commands, "sc").group == "Lighting #1"
    sent, calls = [], []
    keep_browsers, keep_out = commands.browsers, browser.out
    commands.browsers = lambda ctx, cmd, args=None: sent.append((cmd, args)) or [{"kept": 1, "closed": 1, "shut": ["t5"]}]
    browser.out = lambda ctx, cmd, args, timeout: calls.append(args) or "closed t5"
    try:
        b.tabs, b.owned = ["t5", "t6"], {"t5"}
        assert commands.cmd_keep(b, [], {}).startswith("kept 1 tab") and sent[-1][1]["only"] == ["t5"]
        browser.cmd_close(b, [], {})
        assert calls[-1]["force"] is True and b.tabs == ["t6"]
        b.tabs, b.owned = ["t5", "t6"], {"t5"}
        commands.cmd_done(b, [], {"quiet": True})
        assert b.tabs == ["t6"] and b.owned == set()
    finally:
        commands.browsers, browser.out = keep_browsers, keep_out
    try:
        desktop.cmd_video(b, ["e5"], {"seconds": "3"})
        assert False, "a video of a page ref must fail"
    except Exception as e:
        assert "screen or a window" in str(e)


def test_hints_need_fixed_text():
    from lighting import commands

    class F:
        events = []

    st = lambda c, *a: {"cmd": c, "args": list(a), "flags": {}}
    loose = {"name": "any-page", "params": {"page": "x"}, "steps": [st("open", "https://{page}"), st("click", "Go")]}
    site = {"name": "any-github", "params": {"page": "x"}, "steps": [st("open", "github.com/{page}"), st("click", "New")]}
    tight = {"name": "gh-issues", "params": {"repo": "x"},
             "steps": [st("open", "github.com/QDHShamiro/{repo}"), st("click", "Issues")]}
    assert routines.literal(loose["steps"][0]) == 0 and routines.literal(site["steps"][0]) == 0
    assert routines.literal(tight["steps"][0]) == 11 and routines.literal(st("click", "Issues")) == 6
    for r in (loose, site, tight):
        routines.save(r)
    try:
        c = commands.Context(F())
        c.ep = {"steps": [st("open", "github.com/QDHShamiro/Lighting")], "hinted": set(), "via": None}
        routines.hint(c)
        assert F.events == ["routine gh-issues can finish this in one call: lighting run gh-issues repo=Lighting"]
    finally:
        for r in (loose, site, tight):
            routines.path(r["name"]).unlink()


def test_suggest_needs_the_app():
    from lighting import recall
    stems = lambda text: {recall.stem(w) for w in recall.words(text)}
    r = {"name": "panel-schaue-host", "tags": ["schaue", "website", "udn", "alles"],
         "steps": [{"cmd": "open", "args": ["https://panel.dolphinsmp.net/app"]}]}
    assert not recall.fits(stems("schaue bei mir spotify udn website alles"), r)
    assert recall.fits(stems("schaue im panel nach dem host"), r)
    focus = {"name": "brave-orange-weg", "tags": ["design", "terminal"],
             "steps": [{"cmd": "focus", "args": ["app:brave"]}, {"cmd": "press", "args": ["{text}"]}]}
    assert recall.routine_words(focus)[1] == "brave"
    assert not recall.fits(stems("mache Lighting zu einer app mit terminal design"), focus)
    assert recall.fits(stems("in brave das orange design weg"), focus)


def test_desktop_alias_and_routing_without_a_tab():
    from lighting import commands
    seen = []
    keep = commands.app

    class App:
        def __getattr__(self, name):
            return lambda ctx, pos, flags: seen.append((name, pos, flags.get("on"))) or "ok"

    inst = App()
    commands.app = lambda: inst
    try:
        c = commands.Context(type("F", (), {"events": [], "hosts": []})(), "s1")
        commands.route(c, "desktop", [], {})
        commands.route(c, "desktop", ["snap", "w3"], {})
        assert seen == [("cmd_windows", [], None), ("cmd_snap", ["w3"], "app")]
        if WINDOWS:
            assert commands.side(c, ["space"], "press") is inst
        c.tabs, c.target = ["t1"], ("web", "brave")
        assert commands.side(c, ["space"], "press") is not inst
    finally:
        commands.app = keep


def test_quick_keys_wait_less():
    from lighting import desktop
    assert all(desktop.quick_key(k) for k in ("tab", "shift+tab", "End", "x", "pgdn", "backspace"))
    assert not any(desktop.quick_key(k) for k in ("enter", "ctrl+a", "alt+f4", "f5", "win+r", "ctrl+shift+tab"))


def test_repeated_failures_get_a_note():
    from lighting import commands
    c = commands.Context(type("F", (), {"events": []})(), "s1")
    outs = [commands.repeated(c, ["read", "w12"], "err: window w12 is gone (closed)") for _ in range(3)]
    assert "\n!" not in outs[0] and "\n!" not in outs[1] and "3 times in a row" in outs[2]
    assert "\n!" not in commands.repeated(c, ["read", "w12"], "err: other")


def test_repeated_failures_are_braked():
    from lighting import commands
    from lighting import defaults as D
    from lighting.common import Pending, Said
    c = commands.Context(type("F", (), {"events": []})(), "s2")
    outs = [commands.repeated(c, ["snap"], "err: this session has no tab yet") for _ in range(D.REPEAT_BRAKE_AT + 1)]
    assert all(isinstance(o, str) for o in outs[:D.REPEAT_BRAKE_AT - 1])
    first, second = outs[-2], outs[-1]
    assert isinstance(first, Pending) and isinstance(second, Pending)
    assert second.deadline - first.deadline > 0.5
    assert first.poll(False) is None
    try:
        first.poll(True)
    except Said as e:
        assert e.text().startswith("err: this session has no tab yet") and "STOP" in e.text()
    else:
        raise AssertionError("the braked failure did not answer")
    assert isinstance(commands.repeated(c, ["snap"], "err: other"), str)
    hooks = [commands.repeated(c, ["suggest"], "err: broken", False) for _ in range(D.REPEAT_BRAKE_AT + 2)]
    assert all(isinstance(o, str) for o in hooks)


def test_running_reads_bytes():
    from lighting import browser
    from lighting import defaults as D
    if not D.WIN:
        return
    keep = browser.subprocess.run

    class R:
        stdout = "brave.exe  1234 Console  1  20.000 K Geschäft".encode("cp850")

    browser.subprocess.run = lambda *a, **k: R()
    try:
        assert browser.running("brave.exe") and not browser.running("chrome.exe")
    finally:
        browser.subprocess.run = keep


def test_done_waits_while_the_session_still_works():
    import time
    from lighting import commands
    sent, timers = [], []

    class F:
        events, waits, hosts, started = [], [], [], 0.0

        def later(self, delay, fn):
            timers.append(fn)

    keep = commands.browsers
    commands.browsers = lambda ctx, cmd, args=None: sent.append(args) or [{"closed": 1, "shut": ["t5"]}]
    try:
        c = commands.Context(F(), "s1")
        c.tabs, c.owned = ["t5"], {"t5"}
        assert commands.cmd_done(c, [], {"quiet": True}) == "" and sent[-1]["close"] is False and c.tabs == ["t5"]
        c.closing = time.time() - 1
        timers.pop()()
        assert c.tabs == [] and c.owned == set() and "close" not in sent[-1]
        c.tabs, c.owned = ["t6"], {"t6"}
        commands.cmd_done(c, [], {"quiet": True})
        commands.run(c, {"argv": ["routines"]})
        timers.pop()()
        assert c.tabs == ["t6"] and c.owned == {"t6"}
    finally:
        commands.browsers = keep


def test_busy_input_becomes_a_wait():
    from lighting import commands, common
    calls = []
    keep = commands.route

    def route(ctx, name, pos, flags):
        calls.append(name)
        if len(calls) == 1:
            raise common.Busy("busy")
        return "ok sent"

    commands.route = route
    try:
        c = commands.Context(type("F", (), {"events": []})(), "s1")
        out, _, _ = commands.run_one(c, ["press", "enter"])
        assert isinstance(out, common.Pending)
        if WINDOWS:
            from lighting import win
            keep_idle = win.idle_ms
            win.idle_ms = lambda: 10_000
            try:
                assert out.poll(False) == "ok sent"
            finally:
                win.idle_ms = keep_idle
    finally:
        commands.route = keep


def test_gone_window_finds_its_successor():
    if not WINDOWS:
        return
    from lighting import commands, desktop, win
    keep = (win.windows, win.alive)
    win.windows = lambda owned=False: [{"hwnd": 77, "title": "Minecraft 1.21.11", "exe": "javaw.exe", "cls": "GLFW",
                                        "pid": 1, "min": False, "fg": False}]
    win.alive = lambda h: h == 77
    try:
        c = commands.Context(type("F", (), {"events": []})(), "s1")
        st = desktop.state(c)
        st.windows, st.info = {"w12": 55}, {"w12": ("javaw.exe", "Minecraft 1.21.11")}
        assert desktop.find_window(c, "w12") == 77 and st.windows["w12"] == 77
        assert c.notes and "w12 is now the new javaw.exe window" in c.notes[0]
        st.info["w12"] = ("other.exe", "x")
        st.windows["w12"] = 56
        try:
            desktop.find_window(c, "w12")
            assert False, "a closed window of another app must fail"
        except Exception as e:
            assert "is gone" in str(e)
    finally:
        win.windows, win.alive = keep


def test_front_window_only_after_the_agent_saw_it():
    if not WINDOWS:
        return
    from lighting import commands, desktop, win
    keep = (win.foreground, win.alive, win.class_of, win.pid_of, win.exe_of, win.elevated)
    win.foreground, win.alive, win.class_of = (lambda: 77), (lambda h: True), (lambda h: "GLFW30")
    win.pid_of, win.exe_of, win.elevated = (lambda h: 5), (lambda p: "javaw.exe"), (lambda p: False)
    try:
        c = commands.Context(type("F", (), {"events": []})(), "s1")
        assert desktop.front_app(c) is None
        desktop.state(c).seen_front = 77
        assert desktop.front_app(c) == 77
        win.exe_of = lambda p: "WindowsTerminal.exe"
        assert desktop.front_app(c) is None
    finally:
        win.foreground, win.alive, win.class_of, win.pid_of, win.exe_of, win.elevated = keep


def test_brain_page_keys():
    from lighting import brain
    assert brain.page_key("https://www.github.com/QDHShamiro/Lighting/pull/17/files?x=1") == "github.com/qdhshamiro/lighting/pull/#"
    assert brain.page_key("https://github.com/a/b/commit/84ab236e9f") == "github.com/a/b/commit/#"
    assert brain.page_key("file:///C:/x.html") == "" and brain.page_key("not a url") == ""
    assert brain.norm("  Checks3 (3) ") == "checks# (#)" and brain.norm("Long name...") == "long name"


def test_brain_trims_known_pages():
    import time
    from lighting import brain, commands
    brain._b["data"] = brain.empty()
    d, now = brain.data(), time.time()
    for _ in range(3):
        for name in ("Issues", "Pull requests", "Merge pull request", "Code"):
            brain.use(d, "github.com/qdhshamiro/lighting", name, "link", now)
    c = commands.Context(type("F", (), {"events": []})())
    lines = ["[t5] QDHShamiro/Lighting - github.com/QDHShamiro/Lighting", "iframe x.com (cross-origin, lighting snap --frame x.com)"]
    lines += ['e%d link "Other %d"' % (i, i) for i in range(1, 9)]
    lines += ['e20 link "Issues 3" ->./issues', 'e21 link "Pull requests" ->./pulls', 'e22 button "Code"',
              'e23 alert "Build failed"', "more: 40 below (scroll or snap --all)"]
    text = "\n".join(lines)
    out = brain.prune(c, text, "github.com/qdhshamiro/lighting")
    assert out.splitlines()[1].startswith("brain: 3 controls you use here, 8 others hidden")
    assert 'e20 link "Issues 3"' in out and "e23 alert" in out and "iframe x.com" in out and "Other 1" not in out
    assert out.endswith("(scroll or snap --all)") and brain.data()["stats"]["pruned"] == 1
    assert brain.prune(c, text, "example.com") == text
    delta = lines[0] + "\nchanged since your last snap: +2\n" + "\n".join(lines[2:])
    assert brain.prune(c, delta, "github.com/qdhshamiro/lighting") == delta
    c.cfg["brain"] = False
    assert brain.prune(c, text, "github.com/qdhshamiro/lighting") == text


def test_brain_learns_fixes_and_regains_refs():
    from lighting import brain, commands, common
    brain._b["data"] = brain.empty()
    c = commands.Context(type("F", (), {"events": []})(), "s1")
    c.where = {"kind": "web", "url": "https://shop.example/cart", "tab": "t9"}
    for _ in range(2):
        brain.observe(c, "click", ["Checkout"], {}, "", False, 'err: nothing matches "Checkout"')
        brain.observe(c, "click", ["Go to checkout"], {}, "ok", True)
    assert brain.fix_for(c, "click", "Checkout")[:2] == ["Go to checkout", 2]
    calls = []

    def route(ctx, name, pos, flags):
        calls.append((name, pos, flags.get("role")))
        return "ok e7"

    got = brain.retry(c, "click", ["Checkout"], {}, common.Fail('nothing matches "Checkout"'), route)
    assert got[0] == "ok e7" and calls[-1] == ("click", ["Go to checkout"], None)
    brain.remember_refs(c, '[t9] Cart - shop.example/cart\ne5 button "Pay now"\ne6 link "Very long name..."')
    got = brain.retry(c, "click", ["e5"], {}, common.Fail("e5 is gone (page changed)", "lighting snap"), route)
    assert got[1] == ["Pay now"] and calls[-1] == ("click", ["Pay now"], "button") and "e5 was gone" in c.notes[-1]
    assert brain.retry(c, "click", ["e6"], {}, common.Fail("e6 is gone (page changed)"), route) is None
    fresh = commands.Context(type("F", (), {"events": []})(), "s2")
    fresh.where = c.where
    brain.data()["fixes"]["shop.example/cart"]["click|checkout"][1] = 1
    better = brain.hint_for(fresh, "click", ["Checkout"], common.Fail('nothing matches "Checkout"'))
    assert 'worked here before after the same miss: "Go to checkout"' in str(better)


def test_self_check_and_weekly_tip():
    import json
    import time
    from lighting import brain
    from lighting import defaults as D
    now = time.time()
    stamp = lambda t: time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(t))
    with open(D.LOG, "a", encoding="utf-8") as f:
        for i in range(12):
            f.write(json.dumps({"t": stamp(now - 3600 + i * 10), "cmd": "snap", "ok": False, "ms": 0,
                                "e": "this session has no tab yet"}) + "\n")
        for i in range(30):
            f.write(json.dumps({"t": stamp(now - 7200 + i), "cmd": "open", "ok": True, "ms": 900}) + "\n")
    s = brain.summary(7, now)
    top = [p for p in s["patterns"] if "no tab yet" in p["error"]][0]
    assert top["n"] == 12 and top["burst"] == 12 and "open a page first" in top["advice"]
    out = brain.cmd_improve(type("C", (), {"cfg": {}})(), [], {})
    assert "12x this session has no tab yet (snap)" in out and "a loop kept going" in out
    D.IMPROVE.unlink(missing_ok=True)
    tip = brain.weekly_tip(now)
    assert tip.startswith("Lighting self-check (weekly)") and tip.endswith("Details: lighting improve")
    assert brain.weekly_tip(now + 60) == ""


def test_brain_boots_from_episodes():
    import json
    import time
    from lighting import brain
    from lighting import defaults as D
    step = lambda cmd, args, url, role=None: {"cmd": cmd, "args": args, "flags": {}, "role": role, "t": time.time(),
                                             "where": {"kind": "web", "url": url}}
    ep = {"t0": time.time(), "steps": [step("open", ["github.com/x/y/pull/3"], "https://github.com/x/y/pull/3"),
                                      step("click", ["Merge pull request"], "https://github.com/x/y/pull/3", "button"),
                                      step("fill", ["Email=a@b.c", "Name=Tom"], "https://shop.example/signup")]}
    with open(D.HOME / "episodes.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps(ep) + "\n")
    D.BRAIN.unlink(missing_ok=True)
    brain._b["data"] = None
    pages = brain.data()["pages"]
    assert pages["github.com/x/y/pull/#"]["els"]["merge pull request"][2] == "button"
    assert set(pages["shop.example/signup"]["els"]) == {"email", "name"} and "github.com" in pages
    brain.save()
    assert json.loads(D.BRAIN.read_text("utf-8"))["v"] == 1


def test_embeds_tabs_filter_and_shot_threshold():
    from PIL import Image
    from lighting import browser, commands
    assert browser.watch_url("https://www.youtube.com/embed/dQw4w9WgXcQ?start=42&rel=0") == \
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s"
    assert browser.watch_url("https://www.youtube-nocookie.com/embed/abcdefgh") == "https://www.youtube.com/watch?v=abcdefgh"
    assert browser.watch_url("https://www.youtube.com/watch?v=abcdefgh") is None
    c = commands.Context(type("F", (), {"events": []})())
    keep = browser.call
    browser.call = lambda ctx, cmd, args=None, timeout=0, want=None, tab=None: {
        "out": "t1 L1 GitHub - github.com/QDHShamiro\nt2 a YouTube - youtube.com\nt3 Spotify - open.spotify.com"}
    try:
        assert browser.cmd_tabs(c, [], {"f": "youtube"}) == "t2 a YouTube - youtube.com"
        assert browser.cmd_tabs(c, [], {"f": "zzz"}) == 'no tab matches "zzz"'
    finally:
        browser.call = keep
    a = Image.new("RGB", (480, 270), "white")
    b = a.copy()
    b.paste((0, 0, 0), (0, 0, 20, 10))
    assert browser.save_image(c, a, "k", True, 480).startswith("shot ")
    assert "no new image" in browser.save_image(c, a.copy(), "k", True, 480)
    assert browser.save_image(c, b, "k", True, 480).startswith("shot ")
    assert "under --threshold 2" in browser.save_image(c, a, "k", False, 480, 2.0)
    assert browser.threshold({"threshold": True}) == 2.0 and browser.threshold({"threshold": "5%"}) == 5.0


def test_doctor():
    import json
    import time
    from lighting import brain, doctor, procs
    from lighting import defaults as D
    py = lambda pid, ppid, cmd, age=5.0: {"pid": pid, "ppid": ppid, "exe": "pythonw.exe", "cmd": cmd, "age": age}
    cl = lambda pid, args, age: {"pid": pid, "ppid": 1, "exe": "lighting.exe", "cmd": '"C:\\x\\lighting.exe" ' + args, "age": age}
    ps = [py(10, 1, "pythonw.exe -X utf8 -m lighting daemon"), py(11, 10, "pythonw.exe -X utf8 -m lighting daemon"),
          py(20, 1, "pythonw.exe -m lighting daemon"), py(21, 20, "pythonw.exe -m lighting daemon"),
          py(30, 2, "python.exe host.py chrome-extension://x/", 500), py(31, 30, "python.exe host.py", 500),
          py(40, 3, "python.exe host.py chrome-extension://y/", 500),
          cl(50, "done --quiet", 300), cl(51, "mcp", 90000), cl(52, "open x", 20), cl(53, "doctor", 2), cl(54, "wait x", 4000)]
    c = procs.classify(ps, 11, [31])
    assert c["extra"] == [20] and c["orphans"] == [40] and c["hosts"] == 2
    assert sorted(p for p, _, _ in c["hung"]) == [50, 54]
    assert procs.classify(ps, 11, None)["orphans"] == []
    assert procs.client_args('"C:\\a b\\lighting.exe" done --quiet') == ["done", "--quiet"]
    assert doctor.newer("2.0.0", "1.10.3") and not doctor.newer("1.1.0", "1.1.0") and doctor.newer("v1.2", "1.1.9")
    if D.WIN:
        text = json.dumps({"mcpServers": {"lighting": {"command": "C:\\Users\\a\\.lighting\\bin\\lighting.exe"}}})
        assert [x.replace("\\\\", "\\") for x in doctor.EXE_PATH.findall(text)] == ["C:\\Users\\a\\.lighting\\bin\\lighting.exe"]
    else:
        text = json.dumps({"mcpServers": {"lighting": {"command": "/home/a/.lighting/bin/lighting"}}})
        assert doctor.EXE_PATH.findall(text) == ["/home/a/.lighting/bin/lighting"], doctor.EXE_PATH.findall(text)
    rows = [(doctor.OK, "daemon", "fine", "", ()), (doctor.WARN, "procs", "stuck", "ends them", ("kill",)),
            (doctor.FAIL, "browser", "not connected", "lighting setup brave", ())]
    out = doctor.render(rows).splitlines()
    assert out[2] == "warn procs   stuck -> lighting doctor --fix (ends them)"
    assert out[3] == "FAIL browser not connected -> fix: lighting setup brave"
    assert out[-1] == "1 problem, 1 warning, 1 ok | lighting doctor --fix repairs 1"
    D.DAEMON_LOG.write_text("Traceback (most recent call last)\nOldError: x\n%s 1.1.0\nTraceback (most recent call last)\n"
                            "  File \"a.py\"\nKeyError: 'tab'\n" % D.DAEMON_STARTED, "utf-8")
    assert doctor.daemon_errors() == (1, "KeyError: 'tab'")
    now = time.time()
    stamp = lambda s: time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(now - s))
    D.LOG.write_text("\n".join(json.dumps({"t": stamp(s), "cmd": "click", "ok": s < 100}) for s in (30 * 86400, 9 * 86400, 50, 10)), "utf-8")
    assert [r["cmd"] for r in brain.read_log(now - 14 * 86400)] == ["click"] * 3
    s = brain.summary(7, now)
    assert (s["runs"], s["failed"], s["prev_runs"]) == (2, 0, 1)
    assert doctor.plural(2, "process") == "2 processes" and doctor.span(7300) == "2 h"


def test_skill_and_mcp_budget():
    from lighting import mcp
    skill = (ROOT / "skills" / "lighting" / "SKILL.md").read_bytes()
    assert len(skill) <= 3500, "SKILL.md grew to %d bytes (budget 3500)" % len(skill)
    assert len(mcp.DESCRIPTION) <= 700, "MCP description grew to %d chars" % len(mcp.DESCRIPTION)


def test_listen_relative_file():
    import types
    from lighting import audio
    folder = tempfile.mkdtemp(prefix="lighting-listen-")
    Path(folder, "clip.mp3").write_bytes(b"x")
    ctx = types.SimpleNamespace(cwd=folder, cfg={})
    audio.start = lambda job, head: (job["file"], job["url"], head)
    assert audio.cmd_listen(ctx, ["clip.mp3"], {}) == (os.path.join(folder, "clip.mp3"), None, "[file] clip.mp3")
    assert audio.cmd_listen(ctx, [os.path.join(folder, "clip.mp3")], {})[0] == os.path.join(folder, "clip.mp3")


def test_watch_transcript():
    from lighting import watch
    segs = [(0.0, "hello and welcome"), (2.0, "and welcome to the video"), (4.0, "[Music]"), (31.0, "next part starts"),
            (40.0, "next part starts here")]
    assert watch.paragraphs(segs, None, None, 30) == [(0.0, "hello and welcome to the video"), (31.0, "next part starts here")]
    assert watch.paragraphs(segs, 30, None, 30) == [(31.0, "next part starts here")]
    long = [(i * 30.0, "sentence number %d with some more words to make it long enough to count" % i) for i in range(100)]
    text, window = watch.transcript(long, None, None, 2400)
    assert window and window > 30 and len(text) < 3000 and all(len(r) <= 160 for r in text.split("\n")), (window, len(text))
    short, none = watch.transcript(segs, None, None, 2400)
    assert none is None and short.startswith("0:00 hello and welcome to the video")


def test_watch_subtitle_order():
    from lighting import watch
    tracks = {"en": "u1", "de-DE": "u2", "en-orig~auto": "u3", "de~auto": "u4"}
    assert watch.sub_order(tracks, None, "en") == ["en", "de-DE", "en-orig~auto", "de~auto"]
    assert watch.sub_order(tracks, "de", "en") == ["de-DE", "en", "de~auto", "en-orig~auto"]
    info = {"language": "en", "subtitles": {"en": [{"ext": "vtt", "url": "a"}]},
            "automatic_captions": {"fr": [{"ext": "vtt", "url": "b"}], "en-orig": [{"ext": "vtt", "url": "c"}]}}
    assert watch.subs_of(info, None) == {"en": "a", "en-orig~auto": "c"}


def test_watch_storyboard_tiles():
    from PIL import Image
    from lighting import watch
    box = Path(tempfile.mkdtemp(prefix="lighting-watch-"))
    colors = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255)]
    for i in range(2):
        img = Image.new("RGB", (30, 12))
        for j, c in enumerate(colors):
            r, col = divmod(j, 3)
            img.paste(Image.new("RGB", (10, 6), c if i == 0 else tuple(255 - v for v in c)), (col * 10, r * 6))
        img.save(box / ("sb-%d.jpg" % i), "PNG")
    board = {"rows": 2, "cols": 3, "fps": 0.5, "frags": [["http://x/0", 12.0], ["http://x/1", 6.0]]}
    tiles = watch.board_tiles(board, [1.0, 5.0, 13.0, 99.0], box)
    got = [(t, img.getpixel((5, 3))) for t, img in tiles]
    assert got == [(1.0, (255, 0, 0)), (5.0, (0, 0, 255)), (13.0, (0, 255, 255)), (99.0, (255, 255, 0))], got
    sheet = watch.sheet(tiles, False, False)
    assert sheet.size == (1024, 154), sheet.size
    assert watch.sheet(tiles[:2], False, True).size == (1024, 307)


def test_watch_sources():
    import types
    from lighting import watch
    from lighting.common import Fail
    folder = tempfile.mkdtemp(prefix="lighting-watch-src-")
    Path(folder, "clip.mp4").write_bytes(b"x")
    ctx = types.SimpleNamespace(cwd=folder, cfg={}, d=types.SimpleNamespace(hosts_seen=set()))
    old = watch.start
    watch.start = lambda c, job: job
    try:
        job = watch.cmd_watch(ctx, ["clip.mp4"], {"from": "1:00", "at": "1:23,2:45"})
        assert (job["kind"], job["src"], job["lo"], job["at"]) == ("file", os.path.join(folder, "clip.mp4"), 60.0, [83.0, 165.0])
        job = watch.cmd_watch(ctx, ["youtu.be/abc"], {})
        assert (job["kind"], job["src"], job["count"]) == ("url", "https://youtu.be/abc", 12)
        try:
            watch.cmd_watch(ctx, ["missing.mp4"], {})
            assert False, "a missing file must not become https://missing.mp4"
        except Fail as e:
            assert "no video file" in e.text()
    finally:
        watch.start = old


def test_linux_key_names():
    try:
        import Xlib.XK
    except ImportError:
        return
    from lighting import win_linux as w
    assert w.keysym_of("Control_L") == 0xffe3 and w.keysym_of("ISO_Level3_Shift") == 0xfe03
    assert w.keysym_of("TAB") == 0xff09 and w.keysym_of("volup") == 0x1008ff13 and w.keysym_of("A") == 0x41
    assert all(w.keysym_of(w.MODSYMS.get(k, k)) for k in list(w.KEYSYMS) + list(w.MODSYMS))


def test_demo_offline():
    import types
    import urllib.request
    from lighting import browser, demo
    from lighting.common import Fail
    seen = []

    def fake(ctx, argv):
        seen.append(argv)
        if argv[0] == "open":
            page = urllib.request.urlopen(argv[1], timeout=5).read().decode("utf-8")
            assert "Start free trial" in page
            return "[t1] Lighting demo - %s" % argv[1], "open", argv[1:]
        if argv[0] == "check":
            raise Fail("nothing matches", "lighting snap")
        if argv[0] == "js":
            return "400", "js", argv[1:]
        return "ok", argv[0], argv[1:]

    old_run, old_any = commands.run_one, browser.any_browser
    commands.run_one, browser.any_browser = fake, lambda c: True
    try:
        ctx = types.SimpleNamespace(no_learn=False)
        out = demo.run(ctx, [], {"fast": True})
    finally:
        commands.run_one, browser.any_browser = old_run, old_any
    assert ctx.no_learn is False
    assert [a[0] for a in seen] == ["open", "snap", "fill", "select", "check", "click", "table", "js"], seen
    assert "$ lighting open <demo page> --new" in out and "127.0.0.1" not in out, out
    assert '$ lighting fill "Name=Ada Lovelace" Email=ada@example.com' in out, out
    assert out.splitlines()[0].startswith("Lighting demo: 7 commands on a made-up shop page") and "pauses" not in out.splitlines()[0]
    assert out.rstrip().endswith("the page itself is ~101 tokens of text; 1 failed (lighting doctor)"), out[-200:]


PHONE_SCREEN = """<?xml version='1.0' encoding='UTF-8'?>
<hierarchy rotation="0">
<node class="android.widget.FrameLayout" package="com.android.settings" bounds="[0,0][1080,2400]">
<node class="android.widget.TextView" text="Settings" bounds="[40,100][400,200]"/>
<node class="android.widget.ScrollView" scrollable="true" bounds="[0,200][1080,2000]">
<node class="android.widget.LinearLayout" clickable="true" bounds="[0,200][1080,400]">
<node class="android.widget.TextView" text="Wi-Fi" bounds="[40,220][300,300]"/>
<node class="android.widget.TextView" text="Connected" bounds="[40,300][300,380]"/>
<node class="android.widget.Switch" checkable="true" checked="true" clickable="true" bounds="[900,240][1040,360]"/>
</node>
<node class="android.widget.LinearLayout" clickable="true" bounds="[0,400][1080,600]">
<node class="android.widget.TextView" text="Bluetooth" bounds="[40,420][300,500]"/>
</node>
</node>
<node class="android.widget.EditText" hint="Search settings" clickable="true" bounds="[40,2000][1040,2100]"/>
</node>
</hierarchy>"""


def test_phone_snapshot():
    import types
    import xml.etree.ElementTree as ET
    from lighting import android
    ctx = types.SimpleNamespace()
    android.dump = lambda c: ET.fromstring(PHONE_SCREEN)
    out = android.snapshot(ctx, {}).splitlines()
    assert out[0] == "[phone None] com.android.settings (4 controls)", out[0]
    assert out[1:] == ['p1 text "Settings"', 'p2 checkbox "Wi-Fi" [x] - Connected', 'p3 button "Bluetooth"',
                       "p4 edit (Search settings)"], out
    assert android.state(ctx).refs["p2"][:2] == (970, 300), android.state(ctx).refs["p2"]
    assert android.state(ctx).refs["p3"][:2] == (540, 500), android.state(ctx).refs["p3"]
    only = android.snapshot(ctx, {"f": "wi-fi"}).splitlines()
    assert only[1:] == ['p2 checkbox "Wi-Fi" [x] - Connected'], only


def test_memory_store_and_search():
    import types
    from lighting import memcmd, memory as M
    ctx = types.SimpleNamespace(cwd=tempfile.mkdtemp(prefix="lighting-proj-"), sid="s-test", told={})
    here = M.project_of(ctx.cwd)
    nid = memcmd.remember(ctx, "The release needs JDK 21. token=abcdef1234567890")
    assert "[secret]" in (M.note(nid)["title"] + M.note(nid)["body"])
    M.add_note("other", "fact", "Brave is the only browser", source="session")
    out = memcmd.cmd_recall(ctx, ["jdk", "release"], {})
    assert out.startswith("#%d " % nid), out
    assert "#%d" % nid in memcmd.cmd_recall(ctx, [str(nid)], {})
    first, new = M.merge_note(here, "fact", "The release needs JDK 21 now", "longer body")
    assert first == nid and not new
    said = memcmd.on_prompt(ctx, "merk dir, dass Spotify nie geschlossen wird")
    assert said and "saved #" in said[0]
    assert memcmd.on_prompt(ctx, "Merkel war Kanzlerin") == []
    tid = int(memcmd.cmd_todo(ctx, ["add", "Ship", "it"], {"list": "rel"}).split("#")[1].split()[0])
    assert "Ship it" in memcmd.cmd_todo(ctx, [], {})
    assert "is done" in memcmd.cmd_todo(ctx, ["done", str(tid)], {})
    assert "Ship it" not in memcmd.cmd_todo(ctx, [], {})
    assert memcmd.cmd_forget(ctx, [str(nid)], {}) == "forgot #%d" % nid
    assert "Spotify" in memcmd.start_text(here)


def test_memory_condense_transcript():
    import json
    from lighting import memlearn
    rows = [
        {"type": "ai-title", "aiTitle": "Fix the build"},
        {"type": "user", "message": {"content": "<system-reminder>skip me</system-reminder>why does the build fail?"}},
        {"type": "user", "isMeta": True, "message": {"content": "meta text"}},
        {"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "hidden"},
                                                      {"type": "tool_use", "name": "Bash",
                                                       "input": {"command": "mvn package"}}]}},
        {"type": "user", "message": {"content": [{"type": "tool_result", "is_error": True,
                                                  "content": "release version 21 not supported"},
                                                 {"type": "tool_result", "content": "fine output"}]}},
        {"type": "assistant", "message": {"content": [{"type": "text", "text": "It needs JDK 21."}]}},
    ]
    path = Path(tempfile.mkdtemp(prefix="lighting-tr-")) / "t.jsonl"
    path.write_text("".join(json.dumps(r) + chr(10) for r in rows) + '{"type": "user", "mess', "utf-8")
    text, end, title = memlearn.condense(str(path), 0)
    assert title == "Fix the build"
    assert text.split(chr(10)) == ["USER: why does the build fail?", "TOOL Bash: mvn package",
                                   "ERROR: release version 21 not supported", "CLAUDE: It needs JDK 21."], text
    assert end < path.stat().st_size
    assert memlearn.condense(str(path), end)[0] == ""
    assert len(memlearn.chunks("a" * 10 + chr(10) + "b" * 10)) == 1
    assert memlearn.parse('noise {"notes": []} tail') == {"notes": []}


def test_memory_work_state_fold():
    from lighting import memlearn
    lists = memlearn.fold_work_state([
        ("p", "rel", '{"task": "a", "status": "todo"}', 1),
        ("p", "rel", '{"task": "a", "status": "doing", "note": "half"}', 2),
        ("p", "rel", '{"task": "b", "status": "done"}', 3),
        ("p", "old", '{"task": "c", "status": "todo"}', 4),
        ("p", "old", '{"status": "done"}', 5),
    ])
    assert lists[("p", "rel")]["items"]["a"]["status"] == "doing"
    assert lists[("p", "rel")]["items"]["a"]["note"] == "half"
    assert lists[("p", "old")]["closed"]


def test_games_paths():
    from lighting import games
    assert games.is_game("C:/Program Files (x86)/Steam/steamapps/common/Rust/RustClient.exe")
    assert games.is_game("D:/Games/FortniteClient-Win64-Shipping.exe")
    assert not games.is_game("C:/Program Files (x86)/Steam/steam.exe")
    assert not games.is_game("C:/Program Files/Epic Games/Launcher/Portal/EpicGamesLauncher.exe")
    assert not games.is_game("C:/Windows/explorer.exe")


def test_vault_index_and_projects():
    import types
    from lighting import defaults as D, memcmd, memory as M, vault
    base = Path(tempfile.mkdtemp(prefix="lighting-vault-"))
    (base / "Knowledge" / "Mistakes").mkdir(parents=True)
    (base / "Templates").mkdir()
    (base / "Projects").mkdir()
    (base / "Knowledge" / "Mistakes" / "jdk.md").write_text(
        "---\ntags: [java]\n---\n# Build needs JDK 21\nMaven fails with release 21 on JDK 17.", "utf-8")
    (base / "Projects" / "Aurora.md").write_text("# Aurora\nMusic app, Supabase.", "utf-8")
    (base / "Templates" / "mistake.md").write_text("# Template", "utf-8")
    cfg = D.config()
    cfg["vault"] = str(base)
    D.save_config(cfg)
    vault.claude_dirs, real_dirs = (lambda: []), vault.claude_dirs
    try:
        assert vault.index() == 2
        rows = {r["title"]: r for r in M.rows("select * from notes where source = 'vault'")}
        assert set(rows) == {"Build needs JDK 21", "Aurora"}
        assert rows["Build needs JDK 21"]["kind"] == "mistake" and rows["Aurora"]["project"] == "Aurora"
        assert vault.index() == 0
        ctx = types.SimpleNamespace(cwd=str(base), sid="s", told={})
        out = memcmd.cmd_recall(ctx, ["maven", "jdk"], {"all": True})
        assert "(vault)" in out and "JDK 21" in out, out
        nid = rows["Build needs JDK 21"]["id"]
        assert "Knowledge/Mistakes/jdk.md" in memcmd.cmd_recall(ctx, [str(nid)], {})
        try:
            memcmd.cmd_forget(ctx, [str(nid)], {})
            raise AssertionError("forget removed a vault file note")
        except commands.Fail:
            pass
        (base / "Projects" / "Aurora.md").unlink()
        assert vault.index() == 1 and not M.rows("select id from notes where title = 'Aurora'")
        record = {"title": "Fix: build/JDK?", "summary": "Raised release to 21\n- pom updated", "why": "",
                  "rejected": "JDK 17 toolchain", "next": "Deploy"}
        first = vault.write_session("abcdef1234", "Aurora", record, 1760000000)
        assert first.parent.name == "Aurora" and first.name.endswith(" abcdef12.md") and "/" not in first.stem
        text = first.read_text("utf-8")
        assert "## Done\n- Raised release to 21\n- pom updated" in text and "## Why" not in text
        assert "## Tried and rejected\n- JDK 17 toolchain" in text and "## Left open\n- Deploy" in text
        second = vault.write_session("abcdef1234", "Aurora", dict(record, title="Renamed"), 1760000000)
        assert not first.exists() and second.exists()
        old = Path(tempfile.mkdtemp(prefix="lighting-ce-")) / ".claude" / "memory" / "sessions"
        old.mkdir(parents=True)
        (old / "Session_Context_x_1.md").write_text("# Aurora: player fix\nSession: 1", "utf-8")
        from lighting import memlearn
        assert memlearn.import_context_engine([old]) == 1 and memlearn.import_context_engine([old]) == 0
        (base / "Templates" / "mistake.md").write_text("---\ntype: mistake\n---\n# <T>\n## Fehler\n## Ursache\n## Fix\n",
                                                      "utf-8")
        (base / "Knowledge" / "INDEX.md").write_text("# INDEX\n\n## Mistakes\n- [[a]] old\n\n## Patterns\n", "utf-8")
        p = vault.write_draft("mistake", "JDK 25 breaks Maven", {"Fehler": "CME", "Ursache": "JAVA_HOME 25", "Fix": ""},
                              "Aurora", "abcdef1234")
        text = p.read_text("utf-8")
        assert "draft: true" in text and "## Ursache\nJAVA_HOME 25" in text and "## Fix" not in text
        assert "1 mistake [Aurora]" in memcmd.cmd_wiki(ctx, [], {})
        assert "wiki drafts wait" in memcmd.start_text("", "")
        out = memcmd.cmd_wiki(ctx, ["accept", "1"], {})
        dest = base / "Knowledge" / "Mistakes" / "JDK 25 breaks Maven.md"
        assert "Knowledge/Mistakes/JDK 25 breaks Maven.md" in out and dest.exists() and not p.exists()
        assert "draft: true" not in dest.read_text("utf-8")
        index = (base / "Knowledge" / "INDEX.md").read_text("utf-8")
        assert index.index("[[Knowledge/Mistakes/JDK 25 breaks Maven]]") < index.index("## Patterns"), index
        q = vault.write_draft("rule", "Never Fable", {"Rule": "no"}, "", "abcdef1234")
        assert "dropped" in memcmd.cmd_wiki(ctx, ["drop", "1"], {}) and not q.exists()
        assert (base / ".trash" / q.name).exists() and memcmd.cmd_wiki(ctx, [], {}).startswith("no drafts")
    finally:
        vault.claude_dirs = real_dirs
        cfg["vault"] = ""
        D.save_config(cfg)
    assert M.project_of(str(Path.home())) == ""
    repo = Path(tempfile.mkdtemp(prefix="lighting-mono-"))
    (repo / ".git").mkdir()
    for name in ("A", "B", "C"):
        (repo / name / "src").mkdir(parents=True)
        (repo / name / "pom.xml").write_text("<project/>", "utf-8")
    assert M.project_of(str(repo / "B" / "src")) == "B"
    assert M.project_of_files([str(repo / "A" / "src" / "x.java")] * 2 + [str(repo / "C" / "y.java")]) == "A"


def test_nested_pending_keeps_waiting():
    import collections
    import types
    from lighting import commands
    from lighting.common import Pending
    inner = Pending(lambda last: "answer", 5, 0.0)
    outer = Pending(lambda last: inner, 5, 0.0)
    assert commands.drain(types.SimpleNamespace(d=types.SimpleNamespace(abort=__import__("threading").Event())),
                          outer) == "answer"
    d = types.SimpleNamespace(abort_gen=0, events=collections.deque())
    ctx = types.SimpleNamespace(d=d, target=None, where=None, last_target=None, image=None, notes=[], no_learn=True)
    p = Pending(lambda last: Pending(lambda last: "answer", 5, 0.0), 5, 0.0)
    p.meta = ("reply", ["reply", "x", "y", "--wait"], 0.0)
    assert commands.poll_pending(ctx, p) is None
    assert commands.poll_pending(ctx, p)["out"] == "answer"


def test_memory_download_resumes():
    import hashlib
    import http.server
    import threading
    from lighting import memmodel as MM
    blob = os.urandom(3 << 20)

    class Ranged(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            start = int((self.headers.get("Range") or "bytes=0-")[6:].rstrip("-"))
            self.send_response(206 if start else 200)
            self.send_header("Content-Length", str(len(blob) - start))
            self.end_headers()
            self.wfile.write(blob[start:])

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Ranged)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    running, tls = MM.games.running, MM.tls
    MM.games.running, MM.tls = (lambda: ""), (lambda: None)
    try:
        dest = Path(tempfile.mkdtemp(prefix="lighting-dl-")) / "m.gguf"
        dest.with_name("m.gguf.part").write_bytes(blob[: 1 << 20])
        url = "http://127.0.0.1:%d/m" % srv.server_address[1]
        MM.fetch(url, dest, len(blob), hashlib.sha256(blob).hexdigest())
        assert dest.read_bytes() == blob and MM._st["dl"] is None
    finally:
        MM.games.running, MM.tls = running, tls
        srv.shutdown()


if __name__ == "__main__":
    tests =[(n, f) for n, f in sorted(globals().items()) if n.startswith("test_") and callable(f)]
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
