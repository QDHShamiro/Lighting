import time

from lighting.common import Fail

_state = {"on": False, "name": None, "t0": 0.0, "web": False, "desk": None}


def cmd_record(ctx, pos, flags):
    sub = pos[0].lower() if pos else "status"
    rest = pos[1:]
    if sub == "start":
        return start(ctx, rest, flags)
    if sub == "stop":
        return stop(ctx, rest, flags, keep=True)
    if sub == "cancel":
        return stop(ctx, rest, flags, keep=False)
    if sub == "status":
        if not _state["on"]:
            return "not recording"
        desk = len([s for s in _state["desk"].steps if s["cmd"] != "#"]) if _state["desk"] else 0
        return 'recording "%s" for %ds, %d app steps so far' % (_state["name"], time.time() - _state["t0"], desk)
    raise Fail("unknown record command %s" % sub, "lighting record start <name> | stop [name=value ...] | cancel | status")


def start(ctx, rest, flags):
    from lighting import browser
    from lighting import pointer
    if _state["on"]:
        raise Fail('already recording "%s"' % _state["name"], "lighting record stop")
    name = rest[0] if rest else time.strftime("rec-%m%d-%H%M")
    web_line = None
    if not flags.get("desktop") and browser.any_browser(ctx):
        try:
            web_line = browser.call(ctx, "record-start", {}).get("out")
        except Fail:
            web_line = None
    desk = None
    if not flags.get("web"):
        from lighting.deskrec import DeskRecorder
        desk = DeskRecorder(minutes=int(flags.get("max") or 15), injected=bool(flags.get("injected"))).start()
    if web_line is None and desk is None:
        raise Fail("nothing to record", "open a page first (lighting open <url>) or leave out --web")
    pointer.recording(True)
    _state.update(on=True, name=name, t0=time.time(), web=web_line is not None, desk=desk)
    parts = []
    if web_line:
        parts.append("browser tab " + web_line)
    if desk:
        parts.append("Windows apps")
    return 'recording "%s" (%s) -> do the steps now, then: lighting record stop [name=value ...]' % (name, " + ".join(parts))


def tidy(steps):
    out = []
    for s in steps:
        s = {"t": s.get("t", 0), "cmd": s["cmd"], "args": list(s.get("args") or []), "flags": dict(s.get("flags") or {})}
        if s["cmd"] in ("click", "fill", "press", "scroll", "reload", "type") and "on" not in s["flags"]:
            s["flags"]["on"] = "web"
        if s["flags"].get("secret"):
            s["flags"].pop("secret")
        if out and s["cmd"] == "open" and out[-1]["cmd"] == "open":
            out[-1] = s
            continue
        if out and s["cmd"] == "focus" and out[-1]["cmd"] == "focus" and out[-1]["args"] == s["args"]:
            continue
        out.append(s)
    return out


def stop(ctx, rest, flags, keep):
    from lighting import browser
    from lighting import pointer
    from lighting import routines
    if not _state["on"]:
        raise Fail("not recording", "lighting record start <name>")
    steps = []
    if _state["web"]:
        try:
            steps += browser.call(ctx, "record-stop", {}).get("steps") or []
        except Fail:
            pass
    if _state["desk"]:
        steps += _state["desk"].stop()
    pointer.recording(False)
    name = _state["name"]
    _state.update(on=False, desk=None, web=False)
    if not keep:
        return "recording cancelled, nothing saved"
    steps.sort(key=lambda s: s.get("t", 0))
    notes = [s["args"][0] for s in steps if s["cmd"] == "#"]
    steps = tidy([s for s in steps if s["cmd"] != "#"])
    if not steps:
        return "recorded nothing" + (" (%s)" % notes[0] if notes else "")
    kv = [p for p in rest if "=" in p]
    words = [p for p in rest if "=" not in p]
    if words:
        name = words[0]
    r = routines.from_steps(name, steps, kv, 0, "recorded")
    lines = ["saved routine %s (%d steps) -> %s" % (r["name"], len(r["steps"]), routines.usage(r))]
    lines += ["%d %s" % (i, routines.step_line(s)) for i, s in enumerate(r["steps"], 1)]
    lines += ["! " + n for n in notes[:5]]
    return "\n".join(lines)
