import os
import sys
import time

from lighting import defaults as D


def pull_secret(argv):
    out, secret, i = [], None, 0
    while i < len(argv):
        if argv[i] == "--env" and i + 1 < len(argv):
            name = argv[i + 1]
            if name not in os.environ:
                raise SystemExit("err: environment variable " + name + " is not set")
            secret = os.environ[name]
            i += 2
            continue
        out.append(argv[i])
        i += 1
    return out, secret


def execute(argv):
    from lighting import boot
    argv, secret = pull_secret(argv)
    boot.ensure_venv()
    payload = {"argv": argv, "cwd": os.getcwd()}
    if secret is not None:
        payload["secret"] = secret
    return boot.request(payload)


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass
    argv = sys.argv[1:]
    if not argv or argv[0] in ("help", "-h", "--help"):
        from lighting.helptext import HELP
        print(HELP)
        return
    cmd = argv[0]
    if cmd in ("version", "--version"):
        print(D.version())
        return
    if cmd == "daemon":
        from lighting import daemon
        daemon.main()
        return
    if cmd == "host":
        from lighting import host
        host.main()
        return
    if cmd == "mcp":
        from lighting import mcp
        mcp.main()
        return
    if cmd in ("setup", "uninstall", "install"):
        from lighting import install
        sys.exit(install.main(argv))
    stats = "--stats" in argv
    if stats:
        argv = [a for a in argv if a != "--stats"]
    t = time.perf_counter()
    res = execute(argv)
    out = res.get("out", "")
    if stats:
        out += "\n[%d ms, ~%d tokens]" % ((time.perf_counter() - t) * 1000, len(out) // 4 + 1)
    sys.stdout.write(out + "\n")
    sys.stdout.flush()
    sys.exit(res.get("code", 0))
