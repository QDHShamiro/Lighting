import json
import os
import queue
import re
import subprocess
import sys
import threading
import time
import warnings
from pathlib import Path

HOME = Path.home() / ".lighting" / "voice"
CONFIG = HOME / "config.json"
PERSONAS = HOME / "personas"
STATE = HOME / "state.json"
SAY = HOME / "say"
LOG = HOME / "voice.log"
RATE = 48000
ASR_RATE = 16000
BLOCK = 960
HIDDEN = 0x08000000
GEMINI = "https://generativelanguage.googleapis.com/v1beta/models/%s:streamGenerateContent?alt=sse"
GROQ_CHAT = "https://api.groq.com/openai/v1/chat/completions"
GROQ_ASR = "https://api.groq.com/openai/v1/audio/transcriptions"
JUNK = re.compile(r"untertitel|amara\.org|vielen dank f(ü|ue)rs zuschauen|copyright|swr \d|zdf|^\W*$", re.I)
SENTENCE = re.compile(r"(.+?[^\d\s][.!?…])(\s+|$)", re.S)
QUESTION = re.compile(r"\?|^(wer|was|wann|wie|wo|warum|wieso|welche[rsnm]?|weißt du|kennst du|stimmt es)\b", re.I)
DEFAULTS = {
    "voice": "de-DE-FlorianMultilingualNeural",
    "rate": "-3%",
    "models": "groq:openai/gpt-oss-120b,gemini-2.5-flash,gemini-2.5-flash-lite,groq:openai/gpt-oss-20b,groq:qwen/qwen3.8-27b",
    "rpm": "groq:openai/gpt-oss-120b=10,gemini-2.5-flash=8,gemini-2.5-flash-lite=12,groq:openai/gpt-oss-20b=12,groq:qwen/qwen3.8-27b=10",
    "search_model": "openai/gpt-oss-120b",
    "search_filler": "Moment, ich schau kurz nach.",
    "listen_device": "",
    "speak_device": "CABLE Input",
    "monitor_device": "",
    "owner_mic": "",
    "home_out": "",
    "owner_name": "",
    "discord_mic": "CABLE Output",
    "discord_out": "",
    "greeting": "",
    "disclosure": "Hi, hier spricht eine KI. Ich höre mit, damit ich antworten kann, und speichere nichts davon. "
                  "Sagt KI raus, dann gehe ich sofort.",
    "bye": "Okay, bin raus.",
    "stop": r"\b(ki|kai|bot)\W+(?:\w+\W+){0,2}?(raus|stopp?|verschwinde|leave)\b",
    "names": "Discord, KI",
    "wake": r"\b(ki|kai|k\.i\.|bot|lighting)\b",
    "silence_s": 20,
    "level": 0.012,
    "end_pause_s": 0.5,
    "settle_s": 0.35,
    "open_settle_s": 1.2,
    "barge_in_s": 0.8,
    "max_utterance_s": 15,
    "min_speech_s": 0.6,
    "local_asr": "base",
    "history": 16,
    "min_words": 4,
    "auto_answer": True,
    "aliases": {},
}
NEUTRAL = "Du bist eine freundliche KI in einem Discord-Call."
RULES = """
Du bist live in einem Discord-Sprachcall. Alles was du schreibst wird sofort vorgelesen.
Du bekommst Gesprochenes als Text aus einer Spracherkennung: Wörter können falsch erkannt sein, Sprecher sind nicht getrennt.
Antworte nur mit dem Text, den du sagen willst. Willst du still bleiben, antworte genau mit: -
So redest du:
- wie ein Kumpel, 1 bis 2 kurze Sätze, keine Emojis, keine Listen, kein Markdown, Zahlen so wie man sie spricht.
- schreib wie gesprochen, nicht wie geschrieben: "hab", "nen", "is", "ne", "halt", "ey", "naja", "also", "mhm",
  ruhig mal mit einem kurzen "achso" oder "ja safe" anfangen. Kommas setzen, wo man beim Reden kurz Luft holt.
- keine Schriftsprache wie "Das ist nicht etwas, worüber ich sprechen kann" oder "Tut mir leid, aber".
- geh auf das Konkrete ein, was gerade gesagt wurde (Namen, Dinge, Spiele), statt allgemein zu antworten.
- wiederhol dich nicht, fang nicht jeden Satz gleich an, stell nicht nach jeder Antwort eine Gegenfrage.
- wenn du etwas akustisch nicht verstanden hast, frag kurz nach, statt zu raten.
Wann du redest:
- wirst du angesprochen (KI, Bot, Lighting%s), antworte immer.
- reden die anderen gerade untereinander, misch dich nur ein, wenn du wirklich was Witziges oder Hilfreiches beitragen kannst. Sonst: -
- hast du gerade erst geredet und keiner reagiert auf dich, bleib still.
- brauchst du für eine gute Antwort aktuelle oder genaue Fakten (Updates, Patches, Release-Daten, Preise, Wetter,
  Sport, News, Stats), antworte NUR mit: SUCHE: <kurze Suchanfrage>. Rate nie Daten oder Zahlen.
Was du nie tust:
- Streit, Vorfälle, Gerüchte oder Privates über Leute ansprechen, auch nicht aus deiner Rolle oben. Nur wenn jemand im
  Call selbst damit anfängt, darfst du kurz und freundlich darauf eingehen.
- Leute beleidigen, bloßstellen oder gegeneinander aufbringen.
- weiterreden, wenn jemand sagt, du sollst still sein oder gehen.
"""
OWNER = """Du bist die KI von %(o)s, nicht %(o)s selbst.
Zeilen, die mit "%(o)s:" anfangen, kommen von %(o)s selbst (%(o)s ist im Call und hört dich). Die anderen Zeilen sind
andere Leute im Call. Reden die anderen mit %(o)s, misch dich nicht rein. Spricht %(o)s dich an, antworte.
"""


def rules(cfg):
    owner = cfg.get("owner_name") or ""
    return RULES % (", " + owner if owner else "") + (OWNER % {"o": owner} if owner else "")


def persona_file(target):
    slug = re.sub(r"[^a-z0-9]+", "-", (target or "").lower()).strip("-")
    return PERSONAS / ((slug or "open-chat") + ".md")


def disclosure(cfg):
    return (cfg.get("disclosure") or "").strip() or DEFAULTS["disclosure"]


def config():
    cfg = dict(DEFAULTS)
    try:
        cfg.update(json.loads(CONFIG.read_text("utf-8")))
    except (OSError, ValueError):
        pass
    return cfg


def save_config(cfg):
    HOME.mkdir(parents=True, exist_ok=True)
    CONFIG.write_text(json.dumps({k: v for k, v in cfg.items() if DEFAULTS.get(k) != v}, indent=1), "utf-8")


def keys():
    from lighting import audio
    found = {k: os.environ[k] for k in ("GROQ_API_KEY", "GEMINI_API_KEY") if os.environ.get(k)}
    try:
        for line in audio.WATCH_ENV.read_text("utf-8").splitlines():
            k, sep, v = line.strip().partition("=")
            if sep and k in ("GROQ_API_KEY", "GEMINI_API_KEY") and k not in found and v.strip():
                found[k] = v.strip().strip("'\"")
    except OSError:
        pass
    return found


def log(line):
    HOME.mkdir(parents=True, exist_ok=True)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(time.strftime("%H:%M:%S ") + line + "\n")


LOCK = threading.Lock()


def state(**kv):
    with LOCK:
        try:
            cur = json.loads(STATE.read_text("utf-8"))
        except (OSError, ValueError):
            cur = {}
        cur.update(kv)
        STATE.write_text(json.dumps(cur), "utf-8")
        return cur


def alive(pid):
    if not pid:
        return False
    out = subprocess.run(["tasklist", "/FI", "PID eq %d" % pid, "/NH"], capture_output=True, text=True,
                         encoding="utf-8", errors="replace", creationflags=HIDDEN).stdout or ""
    return str(pid) in out


def device(kind, name):
    import soundcard as sc
    pool = sc.all_microphones(include_loopback=True) if kind == "loop" else sc.all_speakers()
    for d in pool:
        if name.lower() in d.name.lower() and (kind != "loop" or d.isloopback):
            return d
    raise RuntimeError("no %s device matching '%s'" % (kind, name))


def request(url, body, headers, timeout=20):
    import urllib.request
    from lighting import browser
    req = urllib.request.Request(url, data=body, method="POST", headers=dict(headers, **{"User-Agent": "lighting"}))
    return urllib.request.urlopen(req, timeout=timeout, context=browser.tls())


ASR_COOL = {"until": 0.0, "model": None}


def local_asr(pcm, cfg):
    import numpy as np
    if ASR_COOL["model"] is None:
        from faster_whisper import WhisperModel
        ASR_COOL["model"] = WhisperModel(cfg["local_asr"], device="cpu", compute_type="int8", cpu_threads=4)
        log("local asr loaded (%s)" % cfg["local_asr"])
    audio16 = np.frombuffer(pcm, dtype="<i2").astype("float32") / 32768
    segs, _ = ASR_COOL["model"].transcribe(audio16, language="de", beam_size=1, vad_filter=True,
                                           initial_prompt=cfg["names"], condition_on_previous_text=False)
    return " ".join(x.text.strip() for x in segs if x.no_speech_prob < 0.6).strip()


def transcribe(pcm, k, cfg):
    if time.time() < ASR_COOL["until"]:
        return clean(local_asr(pcm, cfg), cfg)
    try:
        return groq_asr(pcm, k, cfg)
    except Exception as e:
        if "429" not in str(e):
            raise
        ASR_COOL["until"] = time.time() + 60
        log("asr: groq limit, using the local model for a minute")
        return clean(local_asr(pcm, cfg), cfg)


def clean(text, cfg):
    bare = text.strip(" .,!?").lower()
    return "" if len(text) < 2 or JUNK.search(text) or bare in cfg["names"].lower() else text


def groq_asr(pcm, k, cfg):
    import io
    import wave
    from lighting import audio
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(ASR_RATE)
        w.writeframes(pcm)
    body, ctype = audio.multipart({"model": "whisper-large-v3-turbo", "language": "de", "response_format": "verbose_json",
                                   "temperature": "0", "prompt": cfg["names"]}, "file", "u.wav", buf.getvalue())
    with request(GROQ_ASR, body, {"Authorization": "Bearer " + k["GROQ_API_KEY"], "Content-Type": ctype}, 15) as r:
        data = json.loads(r.read().decode("utf-8"))
    segs = data.get("segments") or [{"text": data.get("text") or ""}]
    text = " ".join((s.get("text") or "").strip() for s in segs if audio.spoken(s) or "no_speech_prob" not in s).strip()
    return clean(text, cfg)


class Brain:
    def __init__(self, cfg, k, target=""):
        self.cfg, self.k, self.turns, self.calls, self.cool = cfg, k, [], {}, {}
        self.rpm = {m: int(n) for m, _, n in (x.partition("=") for x in cfg["rpm"].split(",")) if n}
        self.usage = {"in": 0, "out": 0, "requests": 0, "searches": 0}
        self.file = persona_file(target)
        try:
            persona = self.file.read_text("utf-8").strip() or NEUTRAL
        except OSError:
            persona = NEUTRAL
        self.system = persona + "\n" + rules(cfg)
        self.prompt_mtime = self.file.stat().st_mtime if self.file.exists() else 0

    def reload(self):
        try:
            mtime = self.file.stat().st_mtime
        except OSError:
            return
        if mtime != self.prompt_mtime:
            self.prompt_mtime = mtime
            self.system = (self.file.read_text("utf-8").strip() or NEUTRAL) + "\n" + rules(self.cfg)
            log("persona reloaded")

    def budget(self, model):
        now = time.time()
        recent = [t for t in self.calls.get(model, []) if now - t < 60]
        self.calls[model] = recent
        return self.cool.get(model, 0) < now and len(recent) < self.rpm.get(model, 10)

    def remember(self, role, text):
        if self.turns and self.turns[-1]["role"] == role:
            self.turns[-1]["parts"][0]["text"] += "\n" + text
        else:
            self.turns.append({"role": role, "parts": [{"text": text}]})
        self.turns = self.turns[-self.cfg["history"]:]
        while self.turns and self.turns[0]["role"] != "user":
            self.turns.pop(0)

    def research(self, query):
        persona = self.system.split("Du bist live")[0][:1500]
        for model in (self.cfg["search_model"], "openai/gpt-oss-20b"):
            body = {"model": model, "tools": [{"type": "browser_search"}], "tool_choice": "required",
                    "reasoning_effort": "low", "max_tokens": 900, "messages": [
                        {"role": "system", "content": persona + "\nDu bist im Discord-Voice-Call. Antworte in 1 bis 2 kurzen "
                         "gesprochenen Sätzen auf Deutsch mit dem Ergebnis, ohne Quellen, Links, Listen oder Markdown."},
                        {"role": "user", "content": "Such im Web und beantworte: " + query}]}
            try:
                with request(GROQ_CHAT, json.dumps(body).encode(), {"Authorization": "Bearer " + self.k["GROQ_API_KEY"],
                                                                    "Content-Type": "application/json"}, 40) as r:
                    data = json.loads(r.read().decode("utf-8"))
                break
            except Exception as e:
                if "429" not in str(e) or model != self.cfg["search_model"]:
                    raise
                log("research: %s busy, trying the small model" % model)
        u = data.get("usage") or {}
        self.usage["in"] += u.get("prompt_tokens", 0)
        self.usage["out"] += u.get("completion_tokens", 0)
        self.usage["searches"] += 1
        return re.sub(r"【[^】]*】|\[[^\]]*\]\([^)]*\)|https?://\S+|[*#_`]", "", data["choices"][0]["message"]["content"]).strip()

    def gemini(self, model):
        body = {"systemInstruction": {"parts": [{"text": self.system}]}, "contents": self.turns,
                "generationConfig": {"temperature": 0.9, "maxOutputTokens": 160, "thinkingConfig": {"thinkingBudget": 0}}}
        with request(GEMINI % model, json.dumps(body).encode(), {"x-goog-api-key": self.k["GEMINI_API_KEY"],
                                                                  "Content-Type": "application/json"}) as r:
            for raw in r:
                line = raw.decode("utf-8").strip()
                if not line.startswith("data:"):
                    continue
                data = json.loads(line[5:])
                meta = data.get("usageMetadata") or {}
                if meta:
                    self.last_usage = (meta.get("promptTokenCount", 0), meta.get("candidatesTokenCount", 0))
                cand = (data.get("candidates") or [{}])[0]
                if cand.get("groundingMetadata", {}).get("webSearchQueries"):
                    self.usage["searches"] += 1
                for part in (cand.get("content") or {}).get("parts") or []:
                    if part.get("text"):
                        yield part["text"]

    def groq(self, model):
        msgs = [{"role": "system", "content": self.system}] + [
            {"role": "assistant" if t["role"] == "model" else "user", "content": t["parts"][0]["text"]} for t in self.turns]
        body = {"model": model, "messages": msgs, "temperature": 0.9, "max_tokens": 400, "stream": True,
                "reasoning_effort": "low", "include_reasoning": False, "stream_options": {"include_usage": True}}
        with request(GROQ_CHAT, json.dumps(body).encode(), {"Authorization": "Bearer " + self.k["GROQ_API_KEY"],
                                                            "Content-Type": "application/json"}) as r:
            for raw in r:
                line = raw.decode("utf-8").strip()
                if not line.startswith("data:") or line.endswith("[DONE]"):
                    continue
                data = json.loads(line[5:])
                if data.get("usage") or (data.get("x_groq") or {}).get("usage"):
                    u = data.get("usage") or data["x_groq"]["usage"]
                    self.last_usage = (u.get("prompt_tokens", 0), u.get("completion_tokens", 0))
                for ch in data.get("choices") or []:
                    if (ch.get("delta") or {}).get("content"):
                        yield ch["delta"]["content"]

    def stream(self, event):
        self.reload()
        self.remember("user", event)
        for model in [m for m in self.cfg["models"].split(",") if m]:
            if not self.budget(model):
                continue
            self.calls[model].append(time.time())
            self.last_usage, said, buf, t0 = (0, 0), [], "", time.time()
            try:
                src = self.groq(model[5:]) if model.startswith("groq:") else self.gemini(model)
                for piece in src:
                    buf += piece
                    if len(buf.lstrip()) < 7 or buf.lstrip().upper().startswith("SUCHE"):
                        continue
                    while True:
                        m = SENTENCE.match(buf)
                        if not m or len(m.group(1)) < 12:
                            break
                        said.append(m.group(1).strip())
                        buf = buf[m.end():]
                        yield model, t0, m.group(1).strip()
                rest = buf.strip()
                if rest.upper().startswith("SUCHE"):
                    query = re.split(r"[\n\-–]|\s{2,}", rest.split(":", 1)[-1].strip())[0].strip()
                    log("research (%d words)" % len(query.split()))
                    yield model, t0, self.cfg["search_filler"]
                    try:
                        rest = self.research(query)
                    except Exception as e:
                        log("research failed: %s" % str(e)[:120])
                        rest = "Hab grad nichts Sicheres dazu gefunden, sorry."
                    for m in SENTENCE.finditer(rest + " "):
                        said.append(m.group(1).strip())
                        yield model, t0, m.group(1).strip()
                    rest = rest if not said else ""
                if rest and rest != "-":
                    said.append(rest)
                    yield model, t0, rest
                text = " ".join(said) or "-"
                self.remember("model", text)
                self.usage["in"] += self.last_usage[0]
                self.usage["out"] += self.last_usage[1]
                self.usage["requests"] += 1
                state(usage=self.usage)
                return
            except Exception as e:
                if said:
                    self.remember("model", " ".join(said))
                    return
                if "429" in str(e):
                    self.cool[model] = time.time() + 60
                log("brain %s failed: %s" % (model, str(e)[:120]))
        log("brain: every model is busy, staying quiet")


class Mouth:
    def __init__(self, cfg):
        self.cfg = cfg
        outs = [device("speaker", cfg["speak_device"])]
        if cfg["monitor_device"]:
            outs.append(device("speaker", cfg["monitor_device"]))
        self.lines, self.cut = queue.Queue(), threading.Event()
        self.speaking, self.pending = threading.Event(), 0
        self.blocks = [queue.Queue() for _ in outs]
        self.first_audio, self.cache, self.recent = None, {}, []
        for d, q in zip(outs, self.blocks):
            threading.Thread(target=self.player, args=(d, q), daemon=True).start()
        threading.Thread(target=self.voice, daemon=True).start()
        threading.Thread(target=self.warm, daemon=True).start()

    def warm(self):
        import queue as q
        for text in (self.cfg["search_filler"], "Mhm.", "Ja safe."):
            keep = []
            try:
                self.synth(text, q.Queue(), keep)
                self.cache[text] = keep
            except Exception as e:
                log("tts warmup failed: %s" % str(e)[:120])

    def player(self, dev, q):
        with dev.player(RATE, channels=1, blocksize=BLOCK) as p:
            while True:
                p.play(q.get())

    def synth(self, text, out, keep=None):
        import asyncio
        import edge_tts
        import numpy as np
        from lighting import audio
        step = RATE // 20
        proc = subprocess.Popen([audio.ffmpeg(), "-v", "quiet", "-probesize", "32", "-analyzeduration", "0", "-f", "mp3",
                                 "-i", "pipe:0", "-f", "f32le", "-ac", "1", "-ar", str(RATE), "pipe:1"],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE, creationflags=HIDDEN)

        def read():
            while True:
                data = proc.stdout.read(step * 4)
                if not data:
                    return
                block = np.frombuffer(data[:len(data) // 4 * 4], dtype="float32")
                out.put(block)
                if keep is not None:
                    keep.append(block)

        reader = threading.Thread(target=read, daemon=True)
        reader.start()

        async def feed():
            async for chunk in edge_tts.Communicate(text, self.cfg["voice"], rate=self.cfg["rate"]).stream():
                if self.cut.is_set():
                    break
                if chunk["type"] == "audio":
                    proc.stdin.write(chunk["data"])
                    proc.stdin.flush()

        try:
            asyncio.run(feed())
        finally:
            try:
                proc.stdin.close()
            except OSError:
                pass
            reader.join(10)
            proc.kill()

    def voice(self):
        pcm = queue.Queue()
        end = object()

        def synth():
            while True:
                text = self.lines.get()
                if text in self.cache and not self.cut.is_set():
                    for block in self.cache[text]:
                        pcm.put(block)
                elif not self.cut.is_set():
                    keep = [] if len(text) <= 45 else None
                    try:
                        self.synth(text, pcm, keep)
                        if keep and not self.cut.is_set():
                            self.cache[text] = keep
                    except Exception as e:
                        log("tts failed: %s" % str(e)[:120])
                pcm.put(end)

        threading.Thread(target=synth, daemon=True).start()
        while True:
            item = pcm.get()
            if item is end:
                self.pending -= 1
                if self.pending <= 0:
                    time.sleep(0.1)
                    self.speaking.clear()
                    self.cut.clear()
                continue
            if self.cut.is_set():
                continue
            self.speaking.set()
            if self.first_audio is None:
                self.first_audio = time.time()
            for q in self.blocks:
                q.put(item)
            time.sleep(len(item) / RATE * 0.9)

    def say(self, text):
        key = re.sub(r"\W+", " ", text or "").strip().lower()
        if key and key in self.recent:
            log("skipped repeat: " + text)
            return
        if key:
            self.recent = (self.recent + [key])[-6:]
        if text and text.strip() != "-":
            log("say: " + text)
            state(last_said=text)
            self.pending += 1
            self.lines.put(text)

    def stop(self):
        if self.speaking.is_set() or self.pending > 0:
            self.cut.set()
            for q in self.blocks:
                while not q.empty():
                    q.get_nowait()

    def busy(self):
        return self.speaking.is_set() or self.pending > 0


def ears(cfg, utterances, stop, mouth, talk, owner=False):
    import numpy as np
    if owner:
        import soundcard as sc
        src = next(m for m in sc.all_microphones() if cfg["owner_mic"].lower() in m.name.lower())
        fwd = None
    else:
        src = device("loop", cfg["listen_device"])
        fwd = device("speaker", cfg["monitor_device"]) if cfg["monitor_device"] else None
    per_s = RATE / BLOCK
    pause, longest, barge = int(cfg["end_pause_s"] * per_s), int(cfg["max_utterance_s"] * per_s), int(cfg["barge_in_s"] * per_s)
    speech, quiet, floor, loud_run, pre = [], 0, 0.004, 0, []
    player = fwd.player(RATE, channels=2, blocksize=BLOCK) if fwd else None
    with src.recorder(RATE, channels=2, blocksize=BLOCK) as rec:
        if player:
            player.__enter__()
        try:
            while not stop.is_set():
                block = rec.record(BLOCK)
                if player:
                    player.play(block)
                mono = block.mean(axis=1)
                rms = float(np.sqrt(np.mean(mono ** 2)))
                loud = rms > max(cfg["level"], floor * 3)
                if not loud and not speech:
                    floor = floor * 0.995 + rms * 0.005
                strong = rms > max(cfg["level"] * 1.8, floor * 4)
                loud_run = loud_run + 1 if strong else max(0, loud_run - 2)
                talk.set() if speech and len(speech) - len(pre) > 0.45 * per_s else talk.clear()
                if loud_run == barge and mouth.busy():
                    log("barge-in: someone talks, I stop")
                    mouth.stop()
                if not speech:
                    pre = (pre + [mono[::3]])[-8:]
                if loud or speech:
                    if not speech:
                        speech = list(pre)
                    speech.append(mono[::3])
                    quiet = 0 if loud else quiet + 1
                if speech and (quiet >= pause or len(speech) >= longest):
                    if len(speech) - quiet - len(pre) > int(cfg["min_speech_s"] * per_s):
                        pcm = (np.clip(np.concatenate(speech), -1, 1) * 32767).astype("<i2").tobytes()
                        utterances.put((time.time(), pcm, owner))
                    speech, quiet = [], 0
        finally:
            if player:
                player.__exit__(None, None, None)


def respond(brain, mouth, talk, event, addressed, t_heard, asr_s):
    if mouth.busy():
        if not addressed:
            brain.remember("user", event)
            return False
        for _ in range(200):
            if not mouth.busy():
                break
            time.sleep(0.05)
    mouth.first_audio = None
    spoke = False
    for model, t0, sentence in brain.stream(event):
        if not spoke and talk.is_set() and not addressed:
            log("dropped (someone started talking): " + sentence)
            brain.remember("model", "-")
            return False
        if not spoke:
            log("timing: asr %.2fs, first sentence %.2fs (%s)" % (asr_s, time.time() - t0, model))
        mouth.say(sentence)
        spoke = True
    if spoke:
        for _ in range(60):
            if mouth.first_audio:
                state(latency=round(mouth.first_audio - t_heard, 2))
                log("timing: end of speech to first audio %.2fs" % (mouth.first_audio - t_heard))
                break
            time.sleep(0.05)
    return spoke


def loop(target):
    warnings.filterwarnings("ignore")
    cfg, k = config(), keys()
    missing = [n for n in ("GROQ_API_KEY", "GEMINI_API_KEY") if n not in k]
    if missing:
        raise SystemExit("missing keys: " + ", ".join(missing))
    SAY.mkdir(parents=True, exist_ok=True)
    state(pid=os.getpid(), target=target, started=time.time(), status="running", last_heard="", last_said="")
    brain, mouth = Brain(cfg, k, target), Mouth(cfg)
    owner = cfg["owner_name"] or "Owner"
    leave = re.compile(cfg["stop"] or DEFAULTS["stop"], re.I)
    utterances, heard, stop, talk = queue.Queue(), queue.Queue(), threading.Event(), threading.Event()
    threading.Thread(target=ears, args=(cfg, utterances, stop, mouth, talk), daemon=True).start()
    if cfg["owner_mic"]:
        threading.Thread(target=ears, args=(cfg, utterances, stop, mouth, talk, True), daemon=True).start()
    wake = re.compile(cfg["wake"], re.I)

    def asr():
        while True:
            t_end, pcm, mine = utterances.get()
            t0 = time.time()
            try:
                text = transcribe(pcm, k, cfg)
            except Exception as e:
                log("asr failed: %s" % str(e)[:120])
                continue
            if text:
                heard.put((t_end, time.time() - t0, ("%s: " % owner if mine else "") + text))

    threading.Thread(target=asr, daemon=True).start()
    threading.Thread(target=lambda: local_asr(b"\0\0" * 16000, cfg), daemon=True).start()
    mouth.say(" ".join(x for x in (disclosure(cfg), (cfg["greeting"] or "").strip()) if x))
    last_voice, last_me, nudged = time.time(), 0.0, 0
    while True:
        for f in sorted(SAY.glob("*.txt")):
            text = f.read_text("utf-8").strip()
            f.unlink(missing_ok=True)
            if text == "__leave__":
                stop.set()
                state(status="stopped")
                return
            if text.startswith("?"):
                respond(brain, mouth, talk, "[%s schreibt dir privat: %s] Sag dazu was in den Call." % (owner, text[1:].strip()),
                        True, time.time(), 0)
            else:
                brain.remember("model", text)
                mouth.say(text)
            last_voice = last_me = time.time()
        try:
            t_end, asr_s, text = heard.get(timeout=0.2)
        except queue.Empty:
            quiet_for = time.time() - max(last_voice, last_me)
            if not talk.is_set() and not mouth.busy() and quiet_for > cfg["silence_s"] * (2 ** nudged) and nudged < 3:
                nudged += 1
                if respond(brain, mouth, talk, "[Seit %d Sekunden sagt keiner was. Bring locker ein Thema rein, das zu "
                                               "vorher passt.]" % int(quiet_for), False, time.time(), 0):
                    last_me = time.time()
            continue
        batch, t_first = [text], t_end
        unfinished = not re.search(r"[.!?…]\s*$", text) or re.search(r"\b(und|also|weil|dass|aber|dann|oder)\W*$", text, re.I)
        deadline = time.time() + (cfg["open_settle_s"] if unfinished else cfg["settle_s"])
        while time.time() < deadline or talk.is_set():
            try:
                t_end, a, more = heard.get(timeout=0.05)
                batch.append(more)
                asr_s = max(asr_s, a)
                unfinished = not re.search(r"[.!?…]\s*$", more) or re.search(r"\b(und|also|weil|dass|aber|dann|oder)\W*$", more, re.I)
                deadline = time.time() + (cfg["open_settle_s"] if unfinished else cfg["settle_s"])
            except queue.Empty:
                if time.time() - t_first > 12:
                    break
        said = " ".join(batch)
        log("heard %d words" % len(said.split()))
        state(last_heard="%d words" % len(said.split()))
        last_voice, nudged = time.time(), 0
        plain = said.replace(owner + ": ", "")
        if leave.search(plain):
            log("left: someone said the stop word")
            mouth.say(cfg["bye"] or DEFAULTS["bye"])
            until = time.time() + 6
            while mouth.busy() and time.time() < until:
                time.sleep(0.1)
            stop.set()
            state(status="stopped", reason="stop word")
            return
        addressed = bool(wake.search(plain)) or bool(QUESTION.search(said) and re.search(r"\b(du|dir|dich|dein\w*)\b", plain, re.I))
        if not addressed and len(plain.split()) < cfg["min_words"] and not QUESTION.search(said):
            brain.remember("user", said)
            continue
        ago = int(time.time() - last_me) if last_me else -1
        event = "%s\n[angesprochen: %s, du hast zuletzt vor %s geredet]" % (
            said, "ja" if addressed else "nein", "%d s" % ago if ago >= 0 else "noch nie")
        if respond(brain, mouth, talk, event, addressed, t_end, asr_s):
            last_me = time.time()


if __name__ == "__main__" and sys.argv[1:2] == ["run"]:
    try:
        loop(sys.argv[2] if len(sys.argv) > 2 else "")
    except Exception as e:
        log("crashed: %s: %s" % (type(e).__name__, e))
        state(status="crashed: %s" % str(e)[:120])
        raise
