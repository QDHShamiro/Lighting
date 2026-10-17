import os
import re
import shutil
import subprocess
import time
from pathlib import Path

from lighting import defaults as D
from lighting import win
from lighting.common import Fail, Pending

WT_CLASS = "CASCADIA_HOSTING_WINDOW_CLASS"
READY = re.compile(r"shift\s*\+\s*tab|for shortcuts|bypass permissions|auto mode|accept edits", re.I)
TRUST = re.compile(r"trust (the|this) (files|folder)|vertraust", re.I)


def claude_cmd():
    return (shutil.which("claude.cmd") or shutil.which("claude.exe")
            or str(Path(os.environ.get("APPDATA", "")) / "npm" / "claude.cmd"))


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
    prompt = " ".join(pos).strip()
    if prompt and not flags.get("yes"):
        raise Fail("a Claude session with a prompt starts working on its own: only when the user asked for it",
                   'lighting claude "the task" --yes')
    wt = shutil.which("wt") if D.WIN else None
    if D.WIN and not wt:
        raise Fail("needs Windows Terminal", "winget install Microsoft.WindowsTerminal")
    folder = flags.get("dir") or ctx.cwd or str(Path.home())
    if not os.path.isdir(folder):
        raise Fail("no folder %s" % folder)
    name = re.sub(r"[^\w #.-]", "", flags.get("name") or "Claude %s" % (Path(folder).name or "home"))[:40].strip() or "Claude"
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
    env = {k: v for k, v in os.environ.items() if k != "CLAUDE_CODE_CHILD_SESSION"}
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
