import asyncio
import collections
import hashlib

from lighting import defaults as D

_engines = {}
_cache = collections.OrderedDict()


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
    from winrt.windows.graphics.imaging import BitmapPixelFormat, SoftwareBitmap
    from winrt.windows.storage.streams import DataWriter
    scale = 2 if max(img.width, img.height) <= D.OCR_UPSCALE_BELOW else 1
    if scale != 1:
        img = img.resize((img.width * scale, img.height * scale), Image.LANCZOS)
    img = img.convert("RGBA")
    digest = hashlib.md5(img.tobytes()[::97]).hexdigest() + "%dx%d%s" % (img.width, img.height, lang)
    if digest in _cache:
        _cache.move_to_end(digest)
        return _cache[digest]
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
