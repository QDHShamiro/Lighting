import json
import re
import urllib.parse

from lighting import defaults as D

SEED = {
    "youtube.com": "https://www.youtube.com/results?search_query={q}",
    "github.com": "https://github.com/search?q={q}",
    "google.com": "https://www.google.com/search?q={q}",
    "duckduckgo.com": "https://duckduckgo.com/?q={q}",
    "de.wikipedia.org": "https://de.wikipedia.org/w/index.php?search={q}",
    "en.wikipedia.org": "https://en.wikipedia.org/w/index.php?search={q}",
    "amazon.de": "https://www.amazon.de/s?k={q}",
    "modrinth.com": "https://modrinth.com/plugins?q={q}",
    "spigotmc.org": "https://www.spigotmc.org/search/?q={q}",
    "tiktok.com": "https://www.tiktok.com/search?q={q}",
    "reddit.com": "https://www.reddit.com/search/?q={q}",
}
NAMES = {"yt": "youtube.com", "gh": "github.com", "ddg": "duckduckgo.com", "wiki": "de.wikipedia.org",
         "wikipedia": "de.wikipedia.org", "enwiki": "en.wikipedia.org", "amazon": "amazon.de", "spigot": "spigotmc.org"}


def host_of(url):
    m = re.match(r"^(?:[a-z][a-z0-9+.-]*://)?(?:www\.|m\.)?([^/:?#]+)", url or "", re.I)
    return m.group(1).lower() if m else ""


def load():
    try:
        return json.loads(D.SITES.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def named(word):
    w = (word or "").lower().strip()
    if w in NAMES:
        return NAMES[w]
    known = set(SEED) | set(load())
    if w in known:
        return w
    return next((h for h in sorted(known) if h.split(".")[0] == w or h == w + ".com"), None)


def template(host):
    host = host_of(host)
    data = load()
    for h in (host, ".".join(host.split(".")[-2:])):
        if h in data and data[h].get("search"):
            return data[h]["search"]
        if h in SEED:
            return SEED[h]
    return None


def build(tpl, words):
    base, _, query = tpl.partition("?")
    if "{q}" in query:
        return base + "?" + query.replace("{q}", urllib.parse.quote_plus(words))
    return tpl.replace("{q}", urllib.parse.quote(words))


def derive(url, value):
    value = (value or "").strip()
    if len(value) < 2 or not url:
        return None
    parts = urllib.parse.urlsplit(url)
    pairs = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
    hits = [i for i, (_, v) in enumerate(pairs) if v.strip().lower() == value.lower()]
    if len(hits) == 1:
        keep = [(k, "{q}" if i == hits[0] else v) for i, (k, v) in enumerate(pairs)
                if i == hits[0] or not re.match(r"^(utm_|ref|fbclid|gclid|sp$|si$)", k)]
        query = "&".join("%s=%s" % (urllib.parse.quote_plus(k), v if v == "{q}" else urllib.parse.quote_plus(v))
                         for k, v in keep)
        return urllib.parse.urlunsplit((parts.scheme, parts.netloc, parts.path, query, ""))
    segs = parts.path.split("/")
    hits = [i for i, s in enumerate(segs) if urllib.parse.unquote(s).lower() == value.lower()]
    if len(hits) == 1:
        segs[hits[0]] = "{q}"
        return urllib.parse.urlunsplit((parts.scheme, parts.netloc, "/".join(segs), "", ""))
    return None


def learn(url, value):
    tpl = derive(url, value)
    host = host_of(url)
    if not tpl or not host:
        return None
    data = load()
    if data.get(host, {}).get("search") == tpl:
        return None
    data.setdefault(host, {})["search"] = tpl
    D.HOME.mkdir(parents=True, exist_ok=True)
    D.SITES.write_text(json.dumps(data, indent=1, ensure_ascii=False), "utf-8")
    return tpl
