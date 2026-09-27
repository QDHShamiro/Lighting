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


def ours(name):
    return next((w["hwnd"] for w in win.windows() if w["title"] == name and w["cls"] == WT_CLASS), None)


def cmd_claude(ctx, pos, flags):
    prompt = " ".join(pos).strip()
    if prompt and not flags.get("yes"):
        raise Fail("a Claude session with a prompt starts working on its own: only when the user asked for it",
                   'lighting claude "the task" --yes')
    wt = shutil.which("wt")
    if not wt:
        raise Fail("needs Windows Terminal", "winget install Microsoft.WindowsTerminal")
    folder = flags.get("dir") or ctx.cwd or str(Path.home())
    if not os.path.isdir(folder):
        raise Fail("no folder %s" % folder)
    name = re.sub(r"[^\w #.-]", "", flags.get("name") or "Claude %s" % (Path(folder).name or "home"))[:40].strip() or "Claude"
    if ours(name):
        name = "%s %d" % (name, int(time.time()) % 1000)
    args = [wt, "-w", "0", "new-tab", "--title", name, "--suppressApplicationTitle", "-d", folder, claude_cmd()]
    if not flags.get("no-remote"):
        args += ["--remote-control", name]
    subprocess.Popen(args, close_fds=True)
    remote = "" if flags.get("no-remote") else ", remote control on (claude.ai/code or the Claude app)"
    head = 'started Claude "%s" in a new Windows Terminal tab (%s)%s' % (name, folder, remote)

    def poll(last):
        hwnd = ours(name)
        text = screen_text(hwnd) if hwnd and not win.user32.IsIconic(hwnd) else ""
        if TRUST.search(text):
            raise Fail(head + ", but Claude asks whether to trust this folder: the user answers in the terminal")
        if not READY.search(text):
            if last:
                raise Fail(head + ", but it did not get ready within %d s" % D.SPAWN_TIMEOUT_S)
            return None
        if not prompt:
            return head
        if ours(name) != hwnd:
            raise Fail(head + ", but its tab is no longer in front: prompt not sent")
        from lighting import desktop
        desktop.state(ctx).hwnd = hwnd
        desktop.cmd_type(ctx, ["focused", prompt], {"enter": True})
        return "%s, prompt sent (%d chars)" % (head, len(prompt))

    return Pending(poll, D.SPAWN_TIMEOUT_S, 1.0)
