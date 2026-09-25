import re
from html import unescape
from html.parser import HTMLParser
from urllib.parse import urljoin, urlsplit

SKIP = {"script", "style", "noscript", "svg", "template", "iframe", "canvas", "form", "button", "select", "nav",
        "footer", "aside", "head", "object", "dialog"}
BLOCK = {"p", "div", "section", "article", "main", "header", "li", "ul", "ol", "table", "tr", "blockquote", "figure",
         "figcaption", "dl", "dt", "dd", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "hr", "br"}
VOID = {"br", "hr", "img", "input", "meta", "link", "source", "area", "base", "col", "embed", "param", "track", "wbr"}
HIDE_STYLE = re.compile(r"display\s*:\s*none|visibility\s*:\s*hidden|font-size\s*:\s*0(?![.\d]*[1-9])|opacity\s*:\s*0(?![.\d]*[1-9])"
                        r"|(?:width|height)\s*:\s*[01]px|clip\s*:\s*rect\(\s*0|left\s*:\s*-\d{3,}px|text-indent\s*:\s*-\d{3,}px", re.I)
HIDE_CLASSES = {"hidden", "sr-only", "visually-hidden", "d-none", "hide", "invisible", "screen-reader-text", "offscreen"}


def hidden_classes(html):
    found = set(HIDE_CLASSES)
    for block in re.findall(r"<style[^>]*>(.*?)</style>", html, re.I | re.S):
        for selectors, body in re.findall(r"([^{}]+)\{([^{}]*)\}", block):
            if HIDE_STYLE.search(body):
                for sel in selectors.split(","):
                    m = re.fullmatch(r"\s*\.([\w-]+)\s*", sel)
                    if m:
                        found.add(m.group(1))
    return found


class Converter(HTMLParser):
    def __init__(self, base, scope, hide):
        super().__init__(convert_charrefs=True)
        self.base = base
        self.scope = scope
        self.hide = hide
        self.inside = 0 if scope else 1
        self.skip = 0
        self.stack = []
        self.out = []
        self.cur = []
        self.pre = 0
        self.href = None
        self.title = ""
        self.in_title = False
        self.cells = None

    def is_hidden(self, tag, a):
        if tag in SKIP or "hidden" in a or a.get("aria-hidden") == "true":
            return True
        if a.get("style") and HIDE_STYLE.search(a["style"]):
            return True
        classes = (a.get("class") or "").split()
        return any(c in self.hide for c in classes)

    def flush(self, prefix=""):
        text = "".join(self.cur)
        self.cur = []
        if not self.pre:
            text = re.sub(r"\s+", " ", text).strip()
        if text:
            self.out.append(prefix + text)

    def handle_starttag(self, tag, attrs):
        a = {k: (v or "") for k, v in attrs}
        if tag == "title":
            self.in_title = True
        if tag in VOID:
            if not self.skip and self.inside:
                if tag in ("br", "hr"):
                    self.flush()
                elif tag == "img" and a.get("alt") and not self.is_hidden(tag, a):
                    self.cur.append("[img: %s]" % a["alt"])
            return
        hidden = self.is_hidden(tag, a)
        self.stack.append((tag, hidden, self.scope is not None and tag == self.scope))
        if self.scope and tag == self.scope:
            self.inside += 1
        if hidden:
            self.skip += 1
            return
        if self.skip or not self.inside:
            return
        if tag in BLOCK:
            self.flush()
        if tag in ("h1", "h2", "h3", "h4", "h5", "h6"):
            self.cur.append("#" * int(tag[1]) + " ")
        elif tag == "li":
            self.cur.append("- ")
        elif tag == "pre":
            self.pre += 1
            self.out.append("```")
        elif tag == "code" and not self.pre:
            self.cur.append("`")
        elif tag == "a":
            self.href = a.get("href")
            self.cur.append("[")
        elif tag == "tr":
            self.cells = []
        elif tag in ("td", "th") and self.cells is not None:
            self.flush()

    def close_tag(self, tag, hidden, scoped):
        if scoped:
            self.flush()
            self.inside = max(0, self.inside - 1)
        if hidden:
            self.skip = max(0, self.skip - 1)
            return
        if self.skip or not self.inside:
            return
        if tag == "a":
            link = self.href or ""
            self.href = None
            if link and not link.startswith(("#", "javascript:")):
                full = urljoin(self.base, link)
                parts = urlsplit(full)
                short = parts.path if parts.netloc == urlsplit(self.base).netloc else parts.netloc + parts.path
                self.cur.append("](%s)" % short[:80])
            else:
                self.cur.append("]")
        elif tag == "code" and not self.pre:
            self.cur.append("`")
        elif tag == "pre":
            self.flush()
            self.pre = max(0, self.pre - 1)
            self.out.append("```")
        elif tag in ("td", "th") and self.cells is not None:
            self.cells.append(re.sub(r"\s+", " ", "".join(self.cur)).strip())
            self.cur = []
        elif tag == "tr" and self.cells is not None:
            if any(self.cells):
                self.out.append(" | ".join(self.cells))
            self.cells = None
        elif tag in BLOCK:
            self.flush()

    def handle_endtag(self, tag):
        if tag == "title":
            self.in_title = False
        for i in range(len(self.stack) - 1, -1, -1):
            if self.stack[i][0] == tag:
                for t, hidden, scoped in reversed(self.stack[i:]):
                    self.close_tag(t, hidden, scoped)
                del self.stack[i:]
                return

    def handle_data(self, data):
        if self.in_title:
            self.title += data
        if self.skip or not self.inside:
            return
        self.cur.append(data)


def run(html, base, scope, hide):
    c = Converter(base, scope, hide)
    try:
        c.feed(html)
        c.close()
    except Exception:
        pass
    c.flush()
    return c


def convert(html, base):
    hide = hidden_classes(html)
    lower = html.lower()
    scope = "main" if "<main" in lower else "article" if "<article" in lower else None
    c = run(html, base, scope, hide)
    if scope and len("".join(c.out)) < 200:
        c = run(html, base, None, hide)
    text = "\n".join(line for line in c.out if line.strip() not in ("[", "]", "[]", "- "))
    text = re.sub(r"\[\]\([^)]*\)", "", text)
    text = re.sub(r"\n{3,}", "\n\n", text).strip()
    return unescape(c.title.strip()), text


if __name__ == "__main__":
    page = ('<html><head><title>T</title><style>.x{display:none}</style></head><body><main><h1>Hi</h1>'
            '<p class="x">SECRET1</p><p style="font-size:0">SECRET2</p><p hidden>SECRET3</p>'
            '<p aria-hidden="true">SECRET4</p><p>Visible <a href="/a">link</a></p><ul><li>one</li><li>two</li></ul>'
            '<table><tr><td>a</td><td>b</td></tr></table></main></body></html>')
    title, text = convert(page, "https://example.com/")
    assert title == "T", title
    assert "SECRET" not in text, text
    assert "# Hi" in text and "Visible [link](/a)" in text and "- one" in text and "a | b" in text, text
    print("html2md ok")
