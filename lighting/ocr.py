import asyncio
import collections
import hashlib

from lighting import defaults as D

_engines = {}
_cache = collections.OrderedDict()


def languages():
    if D.MAC:
        import Vision
        req = Vision.VNRecognizeTextRequest.alloc().init()
        langs, _ = req.supportedRecognitionLanguagesAndReturnError_(None)
        return sorted(str(l) for l in langs or [])
    if D.LINUX:
        import shutil
        import subprocess
        exe = shutil.which("tesseract")
        if not exe:
            raise RuntimeError("tesseract is not installed (sudo apt install tesseract-ocr)")
        out = subprocess.run([exe, "--list-langs"], capture_output=True, text=True, timeout=10).stdout
        return sorted(l.strip() for l in out.splitlines()[1:] if l.strip() and l.strip() != "osd")
    from winrt.windows.media.ocr import OcrEngine
    return sorted(l.language_tag for l in OcrEngine.available_recognizer_languages)


def recognize_mac(img, lang):
    import Quartz
    import Vision
    from Foundation import NSData
    import io
    buf = io.BytesIO()
    img.convert("RGB").save(buf, "PNG")
    data = NSData.dataWithBytes_length_(buf.getvalue(), len(buf.getvalue()))
    src = Quartz.CGImageSourceCreateWithData(data, None)
    cg = Quartz.CGImageSourceCreateImageAtIndex(src, 0, None)
    req = Vision.VNRecognizeTextRequest.alloc().init()
    req.setRecognitionLevel_(0)
    req.setUsesLanguageCorrection_(True)
    if lang and lang != "auto":
        req.setRecognitionLanguages_([{"en": "en-US", "de": "de-DE"}.get(lang, lang)])
    handler = Vision.VNImageRequestHandler.alloc().initWithCGImage_options_(cg, None)
    handler.performRequests_error_([req], None)
    w, h = img.width, img.height
    lines = []
    for obs in req.results() or []:
        cand = obs.topCandidates_(1)
        if not cand:
            continue
        box = obs.boundingBox()
        x, y = box.origin.x * w, (1 - box.origin.y - box.size.height) * h
        lines.append((str(cand[0].string()), x, y, box.size.width * w, box.size.height * h))
    lines.sort(key=lambda l: (round(l[2] / 8), l[1]))
    return lines


def recognize_linux(img, lang):
    import shutil
    import subprocess
    import tempfile
    exe = shutil.which("tesseract")
    if not exe:
        raise RuntimeError("tesseract is not installed (sudo apt install tesseract-ocr)")
    code = {"en": "eng", "de": "deu"}.get(lang or "", "eng" if not lang or lang == "auto" else lang)
    with tempfile.TemporaryDirectory() as tmp:
        path = tmp + "/in.png"
        img.convert("L").save(path)
        out = subprocess.run([exe, path, "stdout", "-l", code, "--psm", "11", "tsv"], capture_output=True, text=True,
                             timeout=60).stdout
    rows = {}
    for line in out.splitlines()[1:]:
        f = line.split("\t")
        if len(f) < 12 or not f[11].strip() or f[10] == "-1":
            continue
        key = (f[2], f[3], f[4])
        x, y, ww, hh = int(f[6]), int(f[7]), int(f[8]), int(f[9])
        r = rows.setdefault(key, [[], x, y, x + ww, y + hh])
        r[0].append(f[11])
        r[1], r[2], r[3], r[4] = min(r[1], x), min(r[2], y), max(r[3], x + ww), max(r[4], y + hh)
    lines = [(" ".join(r[0]), r[1], r[2], r[3] - r[1], r[4] - r[2]) for r in rows.values()]
    lines.sort(key=lambda l: (round(l[2] / 8), l[1]))
    return lines


def engine(lang):
    from winrt.windows.globalization import Language
    from winrt.windows.media.ocr import OcrEngine
    key = lang or "auto"
    if key in _engines:
        return _engines[key]
    eng = None
    if lang and lang != "auto":
        tag = {"en": "en-US", "de": "de-DE"}.get(lang, lang)
        if OcrEngine.is_language_supported(Language(tag)):
            eng = OcrEngine.try_create_from_language(Language(tag))
    if eng is None:
        tags = [l.language_tag for l in OcrEngine.available_recognizer_languages]
        pref = next((t for t in tags if t.lower().startswith("en")), None)
        eng = OcrEngine.try_create_from_language(Language(pref)) if pref else OcrEngine.try_create_from_user_profile_languages()
    if eng is None:
        raise RuntimeError("no Windows OCR language installed (Settings > Time & language > Language > add English)")
    _engines[key] = eng
    return eng


async def _run(eng, bitmap):
    return await eng.recognize_async(bitmap)


def recognize(img, lang=None):
    from PIL import Image
    scale = 2 if max(img.width, img.height) <= D.OCR_UPSCALE_BELOW else 1
    if scale != 1:
        img = img.resize((img.width * scale, img.height * scale), Image.LANCZOS)
    img = img.convert("RGBA")
    digest = hashlib.md5(img.tobytes()[::97]).hexdigest() + "%dx%d%s" % (img.width, img.height, lang)
    if digest in _cache:
        _cache.move_to_end(digest)
        return _cache[digest]
    if not D.WIN:
        raw = recognize_mac(img, lang) if D.MAC else recognize_linux(img, lang)
        lines = [(t, x / scale, y / scale, w / scale, h / scale) for t, x, y, w, h in raw]
        _cache[digest] = lines
        while len(_cache) > 8:
            _cache.popitem(last=False)
        return lines
    from winrt.windows.graphics.imaging import BitmapPixelFormat, SoftwareBitmap
    from winrt.windows.storage.streams import DataWriter
    writer = DataWriter()
    writer.write_bytes(img.tobytes())
    bitmap = SoftwareBitmap.create_copy_from_buffer(writer.detach_buffer(), BitmapPixelFormat.RGBA8, img.width, img.height)
    result = asyncio.run(_run(engine(lang), bitmap))
    lines = []
    for line in result.lines:
        words = list(line.words)
        if not words:
            continue
        xs = [w.bounding_rect.x for w in words]
        ys = [w.bounding_rect.y for w in words]
        x2 = [w.bounding_rect.x + w.bounding_rect.width for w in words]
        y2 = [w.bounding_rect.y + w.bounding_rect.height for w in words]
        lines.append((line.text, min(xs) / scale, min(ys) / scale, (max(x2) - min(xs)) / scale, (max(y2) - min(ys)) / scale))
    _cache[digest] = lines
    while len(_cache) > 8:
        _cache.popitem(last=False)
    return lines
