import base64
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

from lighting import defaults as D
from lighting.common import Fail, Pending, cap, is_url

GROQ = ("GROQ_API_KEY", "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo", "groq")
OPENAI = ("OPENAI_API_KEY", "https://api.openai.com/v1/audio/transcriptions", "whisper-1", "openai")
HIDDEN = 0x08000000
WATCH_ENV = Path.home() / ".config" / "watch" / ".env"
TAG = re.compile(r"<[^>]+>")
NOISE = re.compile(r"^\W*(?:(?:intro |outro |background |upbeat |sad |soft )?(?:music|musik|applause|applaus|laughter|"
                   r"lachen|silence|stille)|♪+)\W*$", re.I)


def keys():
    found = {k: os.environ[k] for k in (GROQ[0], OPENAI[0]) if os.environ.get(k)}
    try:
        for line in WATCH_ENV.read_text("utf-8").splitlines():
            k, sep, v = line.strip().partition("=")
            v = v.strip().strip("'\"")
            if sep and k in (GROQ[0], OPENAI[0]) and v and k not in found:
                found[k] = v
    except OSError:
        pass
    return found


def clock(s):
    s = int(max(0, s))
    m, s = divmod(s, 60)
    h, m = divmod(m, 60)
    return "%d:%02d:%02d" % (h, m, s) if h else "%d:%02d" % (m, s)


def stamp(t):
    parts = [float(p) for p in t.replace(",", ".").split(":")]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


def vtt_segments(text):
    segs, last = [], None
    for block in re.split(r"\n\s*\n", text.replace("\r", "")):
        rows = block.split("\n")
        at = next((i for i, r in enumerate(rows) if "-->" in r), None)
        m = re.match(r"\s*(\d+:\d\d:\d\d[.,]\d+|\d+:\d\d[.,]\d+)\s+-->", rows[at]) if at is not None else None
        if not m:
            continue
        for line in rows[at + 1:]:
            line = " ".join(TAG.sub("", line).split())
            if line and line != last:
                segs.append((stamp(m.group(1)), line))
                last = line
    return segs


def lines_of(segs, offset=0.0, lo=None, hi=None):
    out, last = [], None
    for start, text in segs:
        t = start + offset
        if (lo is not None and t < lo - 1) or (hi is not None and t > hi):
            continue
        text = " ".join((text or "").split())
        if NOISE.match(text):
            continue
        if text and text != last:
            out.append("%s %s" % (clock(t), text))
            last = text
    return out


def run(args, timeout):
    try:
        res = subprocess.run(args, capture_output=True, text=True, timeout=timeout, creationflags=HIDDEN,
                             encoding="utf-8", errors="replace")
    except subprocess.TimeoutExpired:
        raise Fail("%s took longer than %d s" % (Path(args[0]).name, timeout))
    except OSError as e:
        raise Fail("could not start %s: %s" % (Path(args[0]).name, e))
    return res


def ffmpeg():
    return shutil.which("ffmpeg") or next((p for p in (r"C:\ffmpeg\bin\ffmpeg.exe",) if os.path.exists(p)), None)


def ytdlp():
    exe = shutil.which("yt-dlp")
    if exe:
        return [exe]
    try:
        import yt_dlp
        return [sys.executable, "-m", "yt_dlp"]
    except ImportError:
        install("yt-dlp")
        return [sys.executable, "-m", "yt_dlp"]


def install(package):
    from lighting import boot
    res = run([boot.uv(), "pip", "install", "--quiet", "--python", str(D.PY), package], 900)
    if res.returncode:
        raise Fail("installing %s failed: %s" % (package, (res.stderr or "").strip()[-200:]))
    import importlib
    importlib.invalidate_caches()


def captions(url, lang, tmp):
    langs = ",".join(x for x in ((lang + ".*") if lang else "", "de.*", "en.*") if x)
    run(ytdlp() + ["--skip-download", "--write-subs", "--write-auto-subs", "--sub-langs", langs, "--sub-format", "vtt",
                   "--no-playlist", "--quiet", "--no-warnings", "-o", str(tmp / "sub"), url], 120)
    files = sorted(tmp.glob("sub*.vtt"))
    order = ([".%s." % lang] if lang else []) + ["-orig.", ".en.", ".de.", ".en-", ".de-"]
    files.sort(key=lambda p: next((i for i, part in enumerate(order) if part in p.name), 99))
    for f in files:
        segs = vtt_segments(f.read_text("utf-8", errors="replace"))
        if segs:
            return segs, f.name.split(".")[1] if f.name.count(".") >= 2 else (lang or "?")
    return [], None


def fetch_audio(url, tmp, lo, hi):
    args = ytdlp() + ["-f", "bestaudio/best", "--no-playlist", "--quiet", "--no-warnings", "-o", str(tmp / "audio.%(ext)s")]
    if (lo or hi) and ffmpeg():
        args += ["--download-sections", "*%s-%s" % (lo or 0, hi if hi else "inf")]
    res = run(args + [url], 240)
    files = [p for p in tmp.glob("audio.*") if p.stat().st_size > 0]
    if not files:
        raise Fail("could not download the audio: " + ((res.stderr or "").strip().splitlines() or ["unknown error"])[-1][:160])
    return files[0]


def shrink(path, tmp):
    exe = ffmpeg()
    if not exe:
        return path
    out = tmp / "speech.ogg"
    res = run([exe, "-y", "-loglevel", "error", "-i", str(path), "-vn", "-ac", "1", "-ar", "16000", "-c:a", "libopus",
               "-b:a", "24k", str(out)], 300)
    return out if res.returncode == 0 and out.exists() and out.stat().st_size > 0 else path


def multipart(fields, name, filename, data):
    boundary = "lighting" + uuid.uuid4().hex
    parts = []
    for k, v in fields.items():
        parts.append(('--%s\r\nContent-Disposition: form-data; name="%s"\r\n\r\n%s\r\n' % (boundary, k, v)).encode("utf-8"))
    parts.append(('--%s\r\nContent-Disposition: form-data; name="%s"; filename="%s"\r\nContent-Type: application/octet-stream'
                  '\r\n\r\n' % (boundary, name, filename)).encode("utf-8") + data + b"\r\n")
    parts.append(("--%s--\r\n" % boundary).encode("utf-8"))
    return b"".join(parts), "multipart/form-data; boundary=" + boundary


def cloud(path, lang, found):
    import urllib.error
    import urllib.request
    from lighting import browser
    env, url, model, name = GROQ if GROQ[0] in found else OPENAI
    fields = {"model": model, "response_format": "verbose_json", "temperature": "0"}
    if lang:
        fields["language"] = lang
    body, ctype = multipart(fields, "file", path.name, path.read_bytes())
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "Authorization": "Bearer " + found[env], "Content-Type": ctype, "User-Agent": "lighting"})
    try:
        with urllib.request.urlopen(req, timeout=300, context=browser.tls()) as r:
            data = json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        why = {401: "the %s key was refused" % name, 413: "the audio is too big for %s" % name,
               429: "%s rate limit reached" % name}.get(e.code, "%s answered %d" % (name, e.code))
        raise Fail(why)
    raw = data.get("segments") or []
    segs = [(float(s.get("start") or 0), s.get("text") or "") for s in raw if spoken(s)]
    if not raw:
        segs = [(0.0, data.get("text") or "")]
    unsure = [s for s in raw if not spoken(s) or float(s.get("no_speech_prob") or 0) > 0.3]
    note = ", mostly music: sung words may be misheard" if raw and len(unsure) * 2 >= len(raw) else ""
    return segs, data.get("language") or lang, "%s %s%s" % (name, model, note)


def spoken(seg):
    no_speech = float(seg.get("no_speech_prob") or 0)
    logprob = float(seg.get("avg_logprob") or 0)
    return no_speech < 0.6 and not (logprob < -1.0 and no_speech > 0.3) and float(seg.get("compression_ratio") or 0) < 2.4


def local(path, lang):
    try:
        import faster_whisper
    except ImportError:
        install("faster-whisper")
    env = dict(os.environ, PYTHONPATH=str(D.ROOT), KMP_DUPLICATE_LIB_OK="TRUE", HF_HUB_DISABLE_SYMLINKS_WARNING="1",
               HF_HUB_DISABLE_TELEMETRY="1")
    exe = str(D.PY) if D.PY.exists() else sys.executable
    try:
        res = subprocess.run([exe, "-m", "lighting.audio", "--local-asr", str(path), lang or ""], capture_output=True,
                             text=True, encoding="utf-8", errors="replace", timeout=D.LISTEN_TIMEOUT_S, env=env,
                             creationflags=HIDDEN)
    except subprocess.TimeoutExpired:
        raise Fail("local speech-to-text took longer than %d s" % D.LISTEN_TIMEOUT_S, "try a shorter part with --from/--to")
    try:
        data = json.loads(res.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        raise Fail("local speech-to-text failed: " + ((res.stderr or "").strip().splitlines() or ["no output"])[-1][:160])
    return [tuple(s) for s in data["segments"]], data.get("language"), "local whisper base"


def local_main(path, lang):
    from faster_whisper import WhisperModel
    model = WhisperModel("base", device="cpu", compute_type="int8", download_root=str(D.HOME / "models"))
    segments, info = model.transcribe(path, language=lang or None, vad_filter=True)
    print(json.dumps({"segments": [[s.start, s.text] for s in segments], "language": info.language}))


def transcribe(path, lang, local_only):
    found = {} if local_only else keys()
    if found:
        try:
            return cloud(path, lang, found)
        except Fail as e:
            if "refused" in str(e):
                raise
        except OSError:
            pass
    return local(path, lang)


def capture(host, tab, ref, ms, lo, tmp):
    msg = host.call("media-audio", {"ref": ref, "op": "record", "ms": int(ms), "from": lo}, tab=tab, timeout=ms / 1000 + 45)
    if not msg.get("ok"):
        raise Fail(str(msg.get("error") or "recording failed"))
    if msg.get("error"):
        raise Fail(msg["error"])
    raw = base64.b64decode(msg.get("audio") or "")
    if len(raw) < 1000:
        raise Fail("the recording is empty (the video did not play)", "click play once in the browser, then listen again")
    path = tmp / "live.webm"
    path.write_bytes(raw)
    return path, float(msg.get("start") or 0), bool(msg.get("blocked"))


def pipeline(job):
    tmp = D.OUT / ("listen-" + uuid.uuid4().hex[:8])
    tmp.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    lo, hi, lang = job["lo"], job["hi"], job["lang"]
    if job["file"]:
        path, offset, note = shrink(Path(job["file"]), tmp), 0.0, ""
    else:
        if job["url"] and not job["live"]:
            try:
                segs, got = captions(job["url"], lang, tmp)
            except Fail:
                segs, got = [], None
            if segs:
                return "transcript (%s, captions, %.1f s):\n%s" % (got, time.time() - t0, "\n".join(lines_of(segs, 0, lo, hi)))
        path, offset, note = None, lo or 0.0, ""
        if job["url"] and not job["live"]:
            try:
                path = fetch_audio(job["url"], tmp, lo, hi)
            except Fail as e:
                note = str(e)
        if path is None:
            if job["live"] and not job["host"]:
                raise Fail("--live records the video in the current tab", "lighting open <url>, then lighting listen --live")
            if not job["host"]:
                raise Fail(note or "no audio source", "lighting listen <url> or open the page first")
            if hi:
                span = hi - (lo or 0)
            elif job["duration"]:
                span = max(job["duration"] - (lo or 0), 5)
            else:
                span = 60
            path, offset, blocked = capture(job["host"], job["tab"], job["ref"], min(span, D.LISTEN_MAX_S) * 1000, lo, tmp)
            note = "recorded live%s" % (", the page kept it muted" if blocked else "")
        path = shrink(path, tmp)
    segs, language, engine = transcribe(path, lang, job["local"])
    lines = lines_of(segs, offset)
    text = "\n".join(lines) if lines else "(no speech: only music or silence; on-screen text: lighting frames)"
    return "transcript (%s, %s%s, %.1f s):\n%s" % (language or "?", engine, ", " + note if note else "", time.time() - t0, text)


def start(job, head):
    state = {}

    def work():
        try:
            state["out"] = pipeline(job)
        except Fail as e:
            state["err"] = e
        except Exception as e:
            state["err"] = Fail("listen failed: %s: %s" % (type(e).__name__, str(e)[:200]))

    threading.Thread(target=work, daemon=True).start()

    def poll(last):
        if "err" in state:
            raise state["err"]
        if "out" in state:
            return cap(head + "\n" + state["out"], "listen", chars=D.LISTEN_CHARS)
        if last:
            raise Fail("listen took longer than %d s" % D.LISTEN_TIMEOUT_S, "try a shorter part with --from/--to")
        return None

    return Pending(poll, D.LISTEN_TIMEOUT_S, 0.5)


def cmd_listen(ctx, pos, flags):
    from lighting import browser
    target = pos[0] if pos else None
    try:
        lo, hi = browser.seconds(flags.get("from")), browser.seconds(flags.get("to"))
    except ValueError:
        raise Fail("--from and --to take seconds or m:ss", "lighting listen --from 1:30 --to 2:00")
    job = {"lo": lo, "hi": hi, "lang": (flags.get("lang") or "").lower() or None, "live": bool(flags.get("live")),
           "local": bool(flags.get("local")) or ctx.cfg.get("audio") == "local", "file": None, "url": None,
           "host": None, "tab": None, "ref": None, "duration": 0}
    if target and os.path.exists(target):
        job["file"] = target
        head = "[file] " + Path(target).name
    elif target and not browser.REF_RE.match(target) and is_url(browser.normalize_url(target)):
        job["url"] = browser.normalize_url(target)
        head = "[url] " + job["url"][:90]
    else:
        where = browser.call(ctx, "where").get("where") or {}
        job.update(url=where.get("url"), tab=where.get("tab"), ref=target if target and browser.REF_RE.match(target) else None)
        head = "[%s] %s - %s" % (where.get("tab") or "t?", (where.get("title") or "")[:60], re.sub(r"^https?://(www\.)?", "", job["url"] or "")[:70])
        if not job["live"]:
            got = browser.call(ctx, "media-audio", {"ref": job["ref"], "op": "cues"}, 20)
            job["ref"], job["duration"] = got.get("ref"), float(got.get("duration") or 0)
            segs = [(float(s), t) for s, t in got.get("cues") or []]
            if segs:
                return cap("%s\ntranscript (%s, captions on the page):\n%s" % (head, got.get("lang") or "?", "\n".join(
                    lines_of(segs, 0, lo, hi))), "listen", chars=D.LISTEN_CHARS)
        job["host"] = browser.pick_host(ctx, quiet=True)
    return start(job, head)


if __name__ == "__main__" and sys.argv[1:2] == ["--local-asr"]:
    local_main(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "")
