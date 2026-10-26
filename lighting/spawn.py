import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from lighting import defaults as D
from lighting import win
from lighting.common import Fail, Pending

WT_CLASS = "CASCADIA_HOSTING_WINDOW_CLASS"
READY = re.compile(r"shift\s*\+\s*tab|for shortcuts|bypass permissions|auto mode|accept edits", re.I)
TRUST = re.compile(r"trust (the|this) (files|folder)|vertraust", re.I)
BACKGROUNDED = re.compile(r"backgrounded\W+([0-9a-f]{6,})")
SESSION_ENV = ("CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_SOCKET",
               "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_CODE_SESSION_ATTENDED", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_EFFORT",
               "CLAUDE_PID")


def claude_cmd():
    return (shutil.which("claude.cmd") or shutil.which("claude.exe")
            or str(Path(os.environ.get("APPDATA", "")) / "npm" / "claude.cmd"))


def claude_exe():
    if not D.WIN:
        return shutil.which("claude") or "claude"
    found = shutil.which("claude.exe")
    if found:
        return found
    shim = Path(claude_cmd())
    native = shim.parent / "node_modules" / "@anthropic-ai" / "claude-code" / "bin" / "claude.exe"
    return str(native) if native.exists() else str(shim)


def claude_out(args, timeout=30):
    env = {k: v for k, v in os.environ.items() if k not in SESSION_ENV}
    try:
        out = subprocess.run([claude_exe()] + args, capture_output=True, timeout=timeout, env=env, stdin=subprocess.DEVNULL,
                             creationflags=D.NO_WINDOW if D.WIN else 0)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Fail("claude %s failed: %s" % (args[0], e))
    text = out.stdout.decode("utf-8", "replace")
    if out.returncode and not text.strip():
        raise Fail("claude %s failed: %s" % (args[0], " ".join(out.stderr.decode("utf-8", "replace").split())[-200:]),
                   "lighting claude list shows the session ids")
    return text


def bg_list(flags):
    import json
    try:
        rows = json.loads(claude_out(["agents", "--json", "--all"]) or "[]")
    except ValueError:
        raise Fail("claude agents gave no list", "claude --version (needs 2.1.300+)")
    since = (time.time() - D.SPAWN_LIST_DAYS * 86400) * 1000
    bg = [r for r in rows if r.get("kind") == "background"]
    shown = [r for r in bg if flags.get("all") or (r.get("state") != "stopped" and (r.get("startedAt") or 0) >= since)]
    hidden = len(bg) - len(shown)
    more = "\n(%d stopped or older than %d days: lighting claude list --all)" % (hidden, D.SPAWN_LIST_DAYS) if hidden else ""
    if not shown:
        return "no background Claude sessions running" + more
    shown.sort(key=lambda r: -(r.get("startedAt") or 0))
    return "\n".join("%s %s%s \"%s\" %s, started %s" % (
        r.get("id"), r.get("state"), " (%s)" % r["waitingFor"] if r.get("waitingFor") else "", (r.get("name") or "")[:60],
        r.get("cwd"), time.strftime("%m-%d %H:%M", time.localtime((r.get("startedAt") or 0) / 1000))) for r in shown) + more


def bg_logs(sid):
    text = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07", "", claude_out(["logs", sid]))
    lines = [x.rstrip() for x in text.splitlines() if x.strip() and x.count("─") + x.count("━") < len(x.strip()) // 2]
    return "\n".join(lines[-D.SPAWN_LOG_LINES:]) or "no output yet from %s" % sid


def background(prompt, folder, name, flags):
    args = [claude_exe(), "--bg", "--name", name]
    for f in ("model", "effort"):
        if flags.get(f):
            args += ["--" + f, flags[f]]
    if not flags.get("no-remote"):
        args += ["--remote-control", name]
    if prompt:
        args.append(prompt)
    D.OUT.mkdir(parents=True, exist_ok=True)
    fd, name_of_log = tempfile.mkstemp(prefix="claude-bg-", suffix=".txt", dir=str(D.OUT))
    log, out = Path(name_of_log), os.fdopen(fd, "wb")
    env = {k: v for k, v in os.environ.items() if k not in SESSION_ENV}
    kw = {"creationflags": D.NO_WINDOW | 0x200} if D.WIN else {"start_new_session": True}
    try:
        proc = subprocess.Popen(args, cwd=folder, env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT,
                                **kw)
    except OSError as e:
        out.close()
        raise Fail("could not start claude: %s" % e, "claude --version")
    worktree = ", edits go to a git worktree under .claude/worktrees" if any(
        (d / ".git").exists() for d in [Path(folder), *Path(folder).parents]) else ""

    def poll(last):
        if proc.poll() is None and not last:
            return None
        out.close()
        text = log.read_text("utf-8", errors="replace")
        m = BACKGROUNDED.search(text)
        if not m:
            tail = " ".join(text.split())[-200:]
            raise Fail("claude --bg did not start%s" % (": " + tail if tail else ""), "claude --version (needs 2.1.300+)")
        sid = m.group(1)
        return 'started Claude "%s" in the background (%s)%s, no window%s: claude attach %s | claude logs %s | ' \
               'claude stop %s' % (name, folder, ", prompt sent" if prompt else "", worktree, sid, sid, sid)

    return Pending(poll, D.SPAWN_TIMEOUT_S, 0.3)


def screen_text(hwnd):
    from lighting import desktop, ocr
    return " ".join(t for t, *_ in ocr.recognize(desktop.grab(win.rect(hwnd), hwnd), "auto"))


def ours(name, before=None):
    if before is not None:
        fresh = [w["hwnd"] for w in win.windows() if w["hwnd"] not in before and w["cls"] == "terminal"]
        return fresh[0] if fresh else None
    return next((w["hwnd"] for w in win.windows() if w["title"] == name and w["cls"] == WT_CLASS), None)


def shell_line(folder, flags, name):
    import shlex
    exe = shutil.which("claude") or "claude"
    args = [exe]
    for f in ("model", "effort"):
        if flags.get(f):
            args += ["--" + f, flags[f]]
    if not flags.get("no-remote"):
        args += ["--remote-control", name]
    return "cd %s && unset CLAUDE_CODE_CHILD_SESSION && %s" % (shlex.quote(folder), " ".join(shlex.quote(a) for a in args))


def open_unix(line, name):
    if D.MAC:
        import shlex
        app = win.terminal_app()

        def apple(s):
            return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'

        if app == "iTerm":
            cmd = "/bin/zsh -lc " + shlex.quote(line + "; exec zsh")
            lines = ['tell application "iTerm" to create window with default profile command ' + apple(cmd)]
        else:
            lines = ['tell application "Terminal"', "activate", "do script " + apple(line), "end tell"]
        args = ["osascript"]
        for l in lines:
            args += ["-e", l]
        subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return app
    for term, args in (("gnome-terminal", ["--title", name, "--", "bash", "-lc", line + "; exec bash"]),
                       ("ptyxis", ["--", "bash", "-lc", line + "; exec bash"]),
                       ("konsole", ["-e", "bash", "-lc", line + "; exec bash"]),
                       ("x-terminal-emulator", ["-e", "bash", "-lc", line + "; exec bash"]),
                       ("xterm", ["-T", name, "-e", "bash", "-lc", line + "; exec bash"])):
        exe = shutil.which(term)
        if exe:
            subprocess.Popen([exe] + args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            return term
    raise Fail("no terminal app found", "sudo apt install gnome-terminal")


def cmd_claude(ctx, pos, flags):
    if not flags.get("yes") and pos and pos[0].lower() == "list" and len(pos) == 1:
        return bg_list(flags)
    if not flags.get("yes") and pos and pos[0].lower() == "logs":
        if len(pos) != 2:
            raise Fail("which session?", "lighting claude list shows the ids, then lighting claude logs <id>")
        return bg_logs(pos[1])
    prompt = " ".join(pos).strip()
    if prompt and not flags.get("yes"):
        raise Fail("a Claude session with a prompt starts working on its own: only when the user asked for it",
                   'lighting claude "the task" --yes')
    folder = flags.get("dir") or ctx.cwd or str(Path.home())
    if not os.path.isdir(folder):
        raise Fail("no folder %s" % folder)
    name = re.sub(r"[^\w #.-]", "", flags.get("name") or "Claude %s" % (Path(folder).name or "home"))[:40].strip() or "Claude"
    if flags.get("bg"):
        return background(prompt, folder, name, flags)
    wt = shutil.which("wt") if D.WIN else None
    if D.WIN and not wt:
        raise Fail("needs Windows Terminal", "winget install Microsoft.WindowsTerminal")
    if D.WIN and ours(name):
        name = "%s %d" % (name, int(time.time()) % 1000)
    remote = "" if flags.get("no-remote") else ", remote control on (claude.ai/code or the Claude app)"
    if not D.WIN:
        before, found = {w["hwnd"] for w in win.windows()}, {}

        def find():
            if not found.get("w"):
                found["w"] = ours(name, before)
            return found["w"]

        term = open_unix(shell_line(folder, flags, name), name)
        head = 'started Claude "%s" in a new %s window (%s)%s' % (name, term, folder, remote)
        return Pending(lambda last: poll_ready(ctx, head, prompt, last, find), D.SPAWN_TIMEOUT_S, 1.0)
    args = [wt, "-w", "0", "new-tab", "--title", name, "--suppressApplicationTitle", "-d", folder, claude_cmd()]
    for f in ("model", "effort"):
        if flags.get(f):
            args += ["--" + f, flags[f]]
    if not flags.get("no-remote"):
        args += ["--remote-control", name]
    env = {k: v for k, v in os.environ.items() if k not in SESSION_ENV}
    subprocess.Popen(args, close_fds=True, env=env)
    head = 'started Claude "%s" in a new Windows Terminal tab (%s)%s' % (name, folder, remote)
    return Pending(lambda last: poll_ready(ctx, head, prompt, last, lambda: ours(name)), D.SPAWN_TIMEOUT_S, 1.0)


def poll_ready(ctx, head, prompt, last, find):
    hwnd = find()
    text = screen_text(hwnd) if hwnd and not win.minimized(hwnd) else ""
    if TRUST.search(text):
        raise Fail(head + ", but Claude asks whether to trust this folder: the user answers in the terminal")
    if not READY.search(text):
        if last:
            raise Fail(head + ", but it did not get ready within %d s" % D.SPAWN_TIMEOUT_S)
        return None
    if not prompt:
        return head
    if find() != hwnd:
        raise Fail(head + ", but its tab is no longer in front: prompt not sent")
    from lighting import desktop
    desktop.state(ctx).hwnd = hwnd
    desktop.cmd_type(ctx, ["focused", prompt], {"enter": True})
    return "%s, prompt sent (%d chars)" % (head, len(prompt))
