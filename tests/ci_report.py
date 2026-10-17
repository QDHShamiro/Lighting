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
    blocks = re.split(r"(?m)^== ", text)
    chunk, size, n = [], 0, 0
    for b in blocks:
        if not b.strip():
            continue
        part = "== " + b
        if size + len(part) > 30000 and chunk:
            n += 1
            print("::notice title=%s part %d::%s" % (label, n, esc("".join(chunk))))
            chunk, size = [], 0
        chunk.append(part)
        size += len(part)
    if chunk:
        n += 1
        print("::notice title=%s part %d::%s" % (label, n, esc("".join(chunk))))
    m = re.findall(r"(\d+)/(\d+) passed", text)
    if m:
        print("::notice title=%s selftest::%s/%s passed" % (label, m[-1][0], m[-1][1]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
