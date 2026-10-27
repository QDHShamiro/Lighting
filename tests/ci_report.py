import re
import sys

SUITES = ("unit tests", "selftest")


def esc(s):
    return s.replace("%", "%25").replace("\r", "").replace("\n", "%0A")


def main(path, label):
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        print("::error title=%s::no log" % label)
        return 1
    keep = [l for l in text.splitlines() if not l.startswith("PASS ")]
    chunks, cur = [], ""
    for line in keep:
        line = line[:400]
        if len(cur) + len(line) + 1 > 3800:
            chunks.append(cur)
            cur = ""
        cur += line + "\n"
    if cur:
        chunks.append(cur)
    for n, c in enumerate(chunks[:9], 1):
        print("::notice title=%s part %d/%d::%s" % (label, n, len(chunks), esc(c)))
    rows = re.findall(r"^(?:PASS|FAIL) (.+?)\s+(\d+) ms", text, re.M)
    slow = sorted(rows, key=lambda r: -int(r[1]))[:8]
    if slow:
        print("::notice title=%s slowest checks::%s" % (label, esc(", ".join("%s %s ms" % r for r in slow))))
    parts = re.split(r"^== (.+)$", text, flags=re.M)
    sections = dict(zip(parts[1::2], parts[2::2]))
    errors = []
    for name in SUITES:
        m = re.findall(r"^(\d+)/(\d+) passed", sections.get(name, ""), re.M)
        if not m:
            errors.append("%s: no result (%s)" % (name, last_exit(sections.get(name)) or "not run"))
            continue
        print("::notice title=%s %s::%s/%s passed" % (label, name, m[-1][0], m[-1][1]))
        if m[-1][0] != m[-1][1]:
            errors.append("%s: %s/%s passed" % ((name,) + m[-1]))
    if errors:
        print("::error title=%s::%s" % (label, "; ".join(errors)))
        return 1
    return 0


def last_exit(body):
    m = re.findall(r"^-- (exit .+)$", body or "", re.M)
    return m[-1] if m else ""


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
