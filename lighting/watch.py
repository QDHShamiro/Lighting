import concurrent.futures
import hashlib
import io
import json
import os
import re
import shutil
import threading
import time
import urllib.parse
import urllib.request
from pathlib import Path

from lighting import audio
from lighting import defaults as D
from lighting.common import Fail, Pending, cap, is_url

CACHE = D.HOME / "watch"
SINGLE = re.compile(r"^https?://(?:www\.|m\.)?(?:youtube\.com/(?:watch|shorts/|live/)|youtu\.be/|tiktok\.com/@[^/]+/video/|"
                    r"vimeo\.com/\d|(?:x|twitter)\.com/[^/]+/status/|instagram\.com/(?:reels?|p)/|twitch\.tv/[^/]+/clip/|"
                    r"clips\.twitch\.tv/)", re.I)
KEEP = ("title", "uploader", "duration", "chapters", "webpage_url", "width", "height", "language")
FILE = re.compile(r"\.(mp4|mov|mkv|webm|avi|m4v|wmv|flv|mpe?g|ts|gif)$", re.I)
LOCAL = re.compile(r"^([a-zA-Z]:|\.{1,2}[\\/]|[\\/~])")


def clock(s):
    return audio.clock(s or 0)


def ffmpeg_exe():
    exe = audio.ffmpeg()
    if exe:
        return exe
    try:
        import imageio_ffmpeg
    except ImportError:
        audio.install("imageio-ffmpeg")
        import imageio_ffmpeg
    return imageio_ffmpeg.get_ffmpeg_exe()


def fetch(url, timeout=30):
    from lighting import browser
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 lighting"})
    with urllib.request.urlopen(req, timeout=timeout, context=browser.tls()) as r:
        return r.read()


def load(path):
    try:
        return json.loads(path.read_text("utf-8"))
    except (OSError, ValueError):
        return None


def save(path, data):
    path.write_text(json.dumps(data), "utf-8")


def prune():
    if not CACHE.exists():
        return
    limit = time.time() - D.WATCH_CACHE_HOURS * 3600
    for box in CACHE.iterdir():
        try:
            if box.is_dir() and box.stat().st_mtime < limit:
                shutil.rmtree(box, ignore_errors=True)
        except OSError:
            pass


def board_of(info):
    best = None
    for f in info.get("formats") or []:
        if f.get("format_note") != "storyboard" or not f.get("fragments") or not f.get("columns") or not f.get("rows"):
            continue
        if best is None or (f.get("width") or 0) > (best.get("width") or 0):
            best = f
    if not best:
        return None
    base = best.get("fragment_base_url") or best.get("url") or ""
    frags = [[fr.get("url") or urllib.parse.urljoin(base, fr.get("path") or ""), float(fr.get("duration") or 0)]
             for fr in best["fragments"]]
    return {"rows": int(best["rows"]), "cols": int(best["columns"]), "fps": best.get("fps"), "frags": frags}


def subs_of(info, lang):
    want = {lang, info.get("language"), "de", "en"} - {None}
    out = {}
    for kind, tag in (("subtitles", ""), ("automatic_captions", "~auto")):
        for code, tracks in (info.get(kind) or {}).items():
            if tag and code not in want and not code.endswith("-orig"):
                continue
            url = next((t.get("url") for t in tracks or [] if t.get("ext") == "vtt" and t.get("url")), None)
            if url:
                out[code + tag] = url
    return out


def sub_order(tracks, lang, original):
    first = [x for x in (lang, original, "de", "en") if x]
    manual = [k for k in tracks if not k.endswith("~auto")]
    auto = [k for k in tracks if k.endswith("~auto")]
    ranked = [k for x in first for k in manual if k == x or k.startswith(x + "-")]
    ranked += [k for k in manual if k not in ranked]
    ranked += [k for k in auto if k.endswith("-orig~auto") and (not lang or k.startswith(lang))]
    ranked += [k for x in first for k in auto if k == x + "~auto"]
    ranked += [k for k in auto if k.endswith("-orig~auto")]
    return list(dict.fromkeys(ranked))


def stream_of(info):
    best = None
    for f in info.get("formats") or []:
        h = f.get("height") or 0
        if f.get("vcodec") in (None, "none") or f.get("protocol") not in ("https", "http") or not h or h > 720:
            continue
        if best is None or h > (best.get("height") or 0):
            best = f
    if not best:
        return None
    heads = "".join("%s: %s\r\n" % kv for kv in (best.get("http_headers") or {}).items())
    return [best["url"], heads]


def probe_url(url):
    res = audio.run(audio.ytdlp() + ["-J", "--no-playlist", "--no-warnings", url], 120)
    try:
        info = json.loads(res.stdout)
    except ValueError:
        why = ((res.stderr or "").strip().splitlines() or ["unknown error"])[-1][:160]
        raise Fail("could not read the video: " + why, "lighting open <url>, then lighting watch e40 (plays it in the browser)")
    meta = {k: info.get(k) for k in KEEP}
    meta.update(board=board_of(info), subs=subs_of(info, None), stream=stream_of(info), at=time.time())
    return meta


def probe_file(path):
    res = audio.run([ffmpeg_exe(), "-hide_banner", "-i", path], 60)
    text = res.stderr or ""
    m = re.search(r"Duration: (\d+):(\d\d):(\d\d(?:\.\d+)?)", text)
    size = re.search(r"Video:.*?(\d{2,5})x(\d{2,5})", text)
    if not m and not size:
        raise Fail("not a video ffmpeg can read: " + Path(path).name)
    dur = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3)) if m else 0
    return {"title": Path(path).name, "duration": dur, "width": int(size.group(1)) if size else None,
            "height": int(size.group(2)) if size else None, "audio": "Audio:" in text, "at": time.time()}


def meta_of(job, box):
    meta = load(box / "meta.json")
    stale = meta and job["kind"] == "url" and time.time() - meta.get("at", 0) > D.WATCH_STREAM_S
    if not meta or stale:
        meta = probe_url(job["src"]) if job["kind"] == "url" else probe_file(job["src"])
        save(box / "meta.json", meta)
    return meta


def merge_overlap(words, new):
    for n in range(min(len(words), len(new), 12), 0, -1):
        if words[-n:] == new[:n]:
            return words + new[n:]
    return words + new


def paragraphs(segs, lo, hi, window):
    out, cur, start = [], [], None
    for t, text in segs:
        if (lo is not None and t < lo - 1) or (hi is not None and t > hi):
            continue
        words = (text or "").split()
        if not words or audio.NOISE.match(" ".join(words)):
            continue
        if start is None or t - start >= window:
            if cur:
                out.append((start, " ".join(cur)))
            cur, start = [], t
        cur = merge_overlap(cur, words)
    if cur:
        out.append((start, " ".join(cur)))
    return out


def transcript(segs, lo, hi, budget):
    paras = paragraphs(segs, lo, hi, 30)
    text = "\n".join("%s %s" % (clock(t), p) for t, p in paras)
    if len(text) <= budget or not paras:
        return text, None
    span = max(1.0, paras[-1][0] - paras[0][0])
    count = max(4, budget // 150)
    window = max(30.0, span / count)
    rows = []
    for t, p in paragraphs(segs, lo, hi, window):
        cut = p if len(p) <= 140 else p[:140].rsplit(" ", 1)[0] + " ..."
        rows.append("%s %s" % (clock(t), cut))
    return "\n".join(rows), window


def subs(meta, lang):
    tracks = meta.get("subs") or {}
    for key in sub_order(tracks, lang, meta.get("language")):
        try:
            segs = audio.vtt_segments(fetch(tracks[key]).decode("utf-8", "replace"))
        except Exception:
            continue
        if segs:
            return segs, key.split("~")[0].replace("-orig", ""), "captions" + (" (auto)" if key.endswith("~auto") else "")
    return [], None, None


def speech(job, meta, box, lo, hi):
    zoomed = job["lo"] is not None or job["hi"] is not None
    whole = box / "speech.json"
    got = load(whole)
    if got:
        return got
    if job["kind"] == "url":
        segs, lang, engine = subs(meta, job["lang"])
        if not segs:
            try:
                segs, lang = audio.captions(job["src"], job["lang"], box)
                engine = "captions"
            except Fail:
                segs = []
        if segs:
            save(whole, [segs, lang, engine])
            return [segs, lang, engine]
    elif not meta.get("audio", True):
        return [[], None, "no sound track"]
    part = box / ("speech-%d-%d.json" % (int(lo or 0), int(hi or 0)))
    if zoomed:
        got = load(part)
        if got:
            return got
    tmp = box / ("a-%d-%d" % (int(lo or 0), int(hi or 0)) if zoomed else "a-all")
    tmp.mkdir(exist_ok=True)
    if job["kind"] == "file":
        path = audio.shrink(Path(job["src"]), tmp)
        offset = 0.0
    else:
        path = audio.shrink(audio.fetch_audio(job["src"], tmp, lo if zoomed else None, hi if zoomed else None), tmp)
        offset = (lo or 0.0) if zoomed else 0.0
    segs, lang, engine = audio.transcribe(path, job["lang"], job["local"])
    got = [[(t + offset, x) for t, x in segs], lang, engine]
    save(part if zoomed and job["kind"] == "url" else whole, got)
    return got


def spread(lo, hi, n):
    span = max(hi - lo, 0.1)
    return [lo + span * (i + 0.5) / n for i in range(n)]


def board_tiles(board, times, box):
    from PIL import Image
    frags, rows, cols = board["frags"], board["rows"], board["cols"]
    starts, t = [], 0.0
    for _, d in frags:
        starts.append(t)
        t += d
    need = {}
    for at in times:
        i = max([k for k, s in enumerate(starts) if s <= at] or [0])
        need.setdefault(i, []).append(at)

    def sprite(i):
        path = box / ("sb-%d.jpg" % i)
        if not path.exists():
            path.write_bytes(fetch(frags[i][0]))
        return i, Image.open(path).convert("RGB")

    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        sheets = dict(pool.map(sprite, list(need)))
    step = 1.0 / board["fps"] if board.get("fps") else None
    tiles = []
    for i, ats in need.items():
        img = sheets[i]
        w, h = img.width // cols, img.height // rows
        per = step or (frags[i][1] / (rows * cols))
        have = max(1, min(rows * cols, int(round(frags[i][1] / per)) if per else rows * cols))
        for at in ats:
            j = max(0, min(int((at - starts[i]) / per) if per else 0, have - 1))
            r, c = divmod(j, cols)
            tiles.append((at, img.crop((c * w, r * h, (c + 1) * w, (r + 1) * h))))
    return sorted(tiles, key=lambda x: x[0])


def media_of(job, meta, box, stream=True):
    if job["kind"] == "file":
        return job["src"], None
    dur = meta.get("duration") or 0
    if stream and meta.get("stream") and dur > D.WATCH_SHORT_S:
        return meta["stream"][0], meta["stream"][1]
    if dur <= D.WATCH_DOWNLOAD_MAX_S:
        have = [p for p in box.glob("video.*") if p.stat().st_size > 0 and not p.name.endswith(".part")]
        if have:
            return str(have[0]), None
        args = audio.ytdlp() + ["-f", "bv*[height<=720][ext=mp4]/bv*[height<=720]/b[height<=720]/b", "--no-playlist",
                                "--quiet", "--no-warnings", "-o", str(box / "video.%(ext)s")]
        if audio.ffmpeg():
            args += ["--ffmpeg-location", audio.ffmpeg()]
        res = audio.run(args + [job["src"]], 300)
        have = [p for p in box.glob("video.*") if p.stat().st_size > 0 and not p.name.endswith(".part")]
        if have:
            return str(have[0]), None
        raise Fail("could not download the video: " + ((res.stderr or "").strip().splitlines() or ["unknown error"])[-1][:160])
    if not meta.get("stream"):
        raise Fail("no video stream to take frames from", "lighting open <url>, then lighting watch e40")
    return meta["stream"][0], meta["stream"][1]


def grab(src, heads, times, width, box):
    from PIL import Image
    exe = ffmpeg_exe()

    def one(at):
        path = box / ("f-%d-%d.jpg" % (int(at * 1000), width))
        if not path.exists():
            args = [exe, "-y", "-loglevel", "error", "-ss", "%.3f" % at]
            if heads:
                args += ["-headers", heads]
            args += ["-i", src, "-frames:v", "1", "-vf", "scale=%d:-2" % width, "-q:v", "4", str(path)]
            audio.run(args, 90)
        if not path.exists():
            return at, None
        return at, Image.open(path).convert("RGB")

    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        got = list(pool.map(one, times))
    return [(at, img) for at, img in got if img is not None]


def pick(tiles, n):
    if len(tiles) <= n:
        return tiles
    from lighting import browser
    keep = browser.distinct([img for _, img in tiles], n)
    return [tiles[i] for i in keep]


def frames_at(job, meta, box, times):
    src, heads = media_of(job, meta, box)
    got = grab(src, heads, times, D.WATCH_READ_WIDTH, box)
    if not got and heads is not None:
        src, heads = media_of(job, meta, box, stream=False)
        got = grab(src, heads, times, D.WATCH_READ_WIDTH, box)
    if not got:
        raise Fail("ffmpeg got no frames from the video", "lighting open <url>, then lighting watch e40")
    return got


def pictures(job, meta, box, lo, hi):
    end = hi or meta.get("duration") or 0
    if job["at"]:
        got = frames_at(job, meta, box, job["at"])
        return got, got, "at"
    if not end:
        raise Fail("the video has no known length (a live stream?)", "lighting watch <url> --at 0:10,0:20")
    zoomed = job["lo"] is not None or job["hi"] is not None
    if meta.get("board") and not zoomed and not job["text"]:
        return pick(board_tiles(meta["board"], spread(lo, end, job["count"] * 3), box), job["count"]), [], "storyboard"
    got = pick(frames_at(job, meta, box, spread(lo, end, job["count"] * (1 if zoomed else 2))), job["count"])
    return got, got, "video"


def read_text(tiles):
    from lighting import ocr
    seen, rows = set(), []
    for at, img in tiles:
        try:
            found = ocr.recognize(img)
        except Exception:
            break
        new = []
        for text, *_ in found:
            line = " ".join(text.split())
            key = re.sub(r"\W+", "", line.lower())
            if len(key) >= 3 and key not in seen:
                seen.add(key)
                new.append(line)
        if new:
            rows.append("%s %s" % (clock(at), " | ".join(new)))
    return rows


def sheet(tiles, portrait, exact):
    from PIL import Image, ImageDraw
    cols, tw = (2, D.WATCH_TILE_AT) if exact else ((6, D.WATCH_TILE_PORTRAIT) if portrait else (4, D.WATCH_TILE_WIDE))
    cols = min(cols, len(tiles))
    first = tiles[0][1]
    th = max(1, round(first.height * tw / first.width))
    out = Image.new("RGB", (cols * tw, -(-len(tiles) // cols) * th), "black")
    draw = ImageDraw.Draw(out)
    for i, (at, img) in enumerate(tiles):
        x, y = i % cols * tw, i // cols * th
        out.paste(img.resize((tw, th)), (x, y))
        label = clock(at)
        draw.rectangle((x, y, x + 8 + 7 * len(label), y + 16), fill="black")
        draw.text((x + 4, y + 2), label, fill="white")
    return out


def work(job):
    box = CACHE / hashlib.sha1(job["key"].encode("utf-8")).hexdigest()[:16]
    box.mkdir(parents=True, exist_ok=True)
    os.utime(box)
    meta = meta_of(job, box)
    dur = meta.get("duration") or 0
    lo = max(0.0, job["lo"] or 0.0)
    hi = min(x for x in (job["hi"], dur) if x) if (job["hi"] or dur) else None
    res = {"meta": meta, "lo": lo, "hi": hi, "notes": []}
    with concurrent.futures.ThreadPoolExecutor(2) as pool:
        heard = pool.submit(speech, job, meta, box, lo, hi)
        seen = pool.submit(pictures, job, meta, box, lo, hi)
        for name, fut in (("speech", heard), ("pictures", seen)):
            try:
                res[name] = fut.result()
            except Fail as e:
                res["notes"].append("%s: %s" % ("sound" if name == "speech" else "frames", e.text()))
            except Exception as e:
                res["notes"].append("%s: %s: %s" % (name, type(e).__name__, str(e)[:160]))
    tiles, readable, how = res.get("pictures") or ([], [], None)
    res["text"] = read_text(readable) if readable else []
    res["how"] = how
    res["portrait"] = bool(meta.get("width") and meta.get("height") and meta["height"] > meta["width"]) or bool(
        tiles and tiles[0][1].height > tiles[0][1].width)
    return res


def short(url):
    return re.sub(r"^https?://(www\.)?", "", url)[:70]


def compose(ctx, job, res):
    from lighting import browser
    meta = res["meta"]
    lines = ["[watch] %s%s - %s%s" % ((meta.get("title") or "video")[:90], " - " + meta["uploader"][:40] if meta.get("uploader") else "",
                                      clock(meta.get("duration")), " - " + short(job["src"]) if job["kind"] == "url" else "")]
    chapters = meta.get("chapters") or []
    if chapters:
        shown = " | ".join("%s %s" % (clock(c.get("start_time")), (c.get("title") or "")[:40]) for c in chapters[:15])
        lines.append("chapters: " + shown + (" | +%d more" % (len(chapters) - 15) if len(chapters) > 15 else ""))
    tiles, _, how = res.get("pictures") or ([], [], None)
    if tiles:
        saved = browser.save_image(ctx, sheet(tiles, res["portrait"], how == "at"), "watch", False, D.SHOT_WIDTH)
        kind = {"storyboard": "scenes from the video's preview images", "at": "frames at the asked times"}.get(how, "scenes")
        lines.append("%s: %d %s at %s" % (saved.replace("shot ", "frames ", 1), len(tiles), kind, " ".join(clock(t) for t, _ in tiles)))
    if res["text"]:
        text = "\n".join(res["text"])
        if len(text) > D.WATCH_TEXT_CHARS:
            text = text[:D.WATCH_TEXT_CHARS].rsplit("\n", 1)[0] + "\n..."
        lines += ["text on screen:", text]
    elif how == "storyboard":
        lines.append("text on screen: not read from preview images (lighting watch ... --text reads the real frames)")
    heard = res.get("speech")
    if heard:
        segs, lang, engine = heard
        segs = [tuple(s) for s in segs]
        budget = D.WATCH_FULL_CHARS if job["full"] or job["lo"] is not None or job["hi"] is not None or job["at"] else D.WATCH_SKIM_CHARS
        lo, hi = res["lo"], res["hi"]
        if job["at"]:
            lo, hi = max(0, min(job["at"]) - 15), max(job["at"]) + 15
        text, window = transcript(segs, lo, hi, budget)
        how_long = "skim: start of every %s; all of it: --full or --from/--to" % clock(window) if window else ""
        lines.append("transcript (%s, %s%s):" % (lang or "?", engine, "; " + how_long if how_long else ""))
        lines.append(text or "(no speech: only music or silence)")
    lines += ["! " + n for n in res["notes"]]
    if job["lo"] is None and job["hi"] is None and not job["at"]:
        src = job["src"] if job["kind"] == "url" else Path(job["src"]).name
        lines.append('closer: lighting watch "%s" --from 1:00 --to 2:00 | exact frames: --at 1:23,2:45' % src)
    return cap("\n".join(lines), "watch", chars=D.WATCH_CHARS)


def start(ctx, job):
    state = {}

    def run():
        try:
            state["res"] = work(job)
        except Fail as e:
            state["err"] = e
        except Exception as e:
            state["err"] = Fail("watch failed: %s: %s" % (type(e).__name__, str(e)[:200]))

    threading.Thread(target=run, daemon=True).start()

    def poll(last):
        if "err" in state:
            raise state["err"]
        if "res" in state:
            return compose(ctx, job, state["res"])
        if last:
            raise Fail("watch took longer than %d s" % D.WATCH_TIMEOUT_S, "try a part with --from/--to")
        return None

    return Pending(poll, D.WATCH_TIMEOUT_S, 0.5)


def times(text):
    from lighting import browser
    return [browser.seconds(x.strip()) for x in str(text).split(",") if x.strip()]


def cmd_watch(ctx, pos, flags):
    from lighting import browser
    target = pos[0] if pos else None
    try:
        lo, hi = browser.seconds(flags.get("from")), browser.seconds(flags.get("to"))
        at = times(flags["at"]) if flags.get("at") not in (None, True, "") else []
    except ValueError:
        raise Fail("--from, --to and --at take seconds or m:ss", "lighting watch <link> --from 1:30 --to 2:00")
    job = {"lo": lo, "hi": hi, "at": at, "full": bool(flags.get("full")), "text": bool(flags.get("text")),
           "lang": (flags.get("lang") or "").lower() or None,
           "local": bool(flags.get("local")) or ctx.cfg.get("audio") == "local",
           "count": max(4, min(int(flags.get("count") or D.WATCH_FRAMES), 24))}
    if target and not is_url(target) and not os.path.isabs(target) and ctx.cwd and os.path.exists(os.path.join(ctx.cwd, target)):
        target = os.path.join(ctx.cwd, target)
    if target and not os.path.isfile(target) and not is_url(target) and FILE.search(target) and (
            LOCAL.match(target) or not re.search(r"[\\/]", target)):
        raise Fail("no video file %s" % target, "check the path (a relative path starts in the folder lighting runs from)")
    if target and os.path.isfile(target):
        st = os.stat(target)
        full = os.path.abspath(target)
        job.update(kind="file", src=full, key="%s|%d|%d" % (full, st.st_size, int(st.st_mtime)))
    elif target and not browser.REF_RE.match(target) and is_url(browser.normalize_url(target)):
        url = browser.normalize_url(target)
        browser.leak_check(ctx, url, flags)
        job.update(kind="url", src=url, key=url)
    else:
        where = browser.call(ctx, "where").get("where") or {}
        url = where.get("url") or ""
        if SINGLE.match(url):
            job.update(kind="url", src=url, key=url)
        elif target and browser.REF_RE.match(target):
            keep = {k: flags[k] for k in ("from", "to", "lang", "local", "count") if flags.get(k) is not None}
            return browser.cmd_frames(ctx, [target], dict(keep, scenes=True, audio=True))
        else:
            raise Fail("watch needs a link, a video file or a video on the page",
                       "lighting watch https://youtu.be/x | watch clip.mp4 | snap --media, then watch e40")
    prune()
    return start(ctx, job)
