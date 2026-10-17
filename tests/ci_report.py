import re
import sys


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
    m = re.findall(r"(\d+)/(\d+) passed", text)
    if m:
        print("::notice title=%s selftest::%s/%s passed" % (label, m[-1][0], m[-1][1]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
