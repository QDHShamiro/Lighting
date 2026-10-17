import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KEEP = ["lighting", "extension", "skills", "requirements.txt", "LICENSE", "NOTICE", "README.md", ".claude-plugin/plugin.json"]
CLIENTS = ["lighting.exe", "lighting-macos"]


def main(out):
    stage = ROOT / "mcpb" / "stage"
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    for rel in KEEP:
        src = ROOT / rel
        if not src.exists():
            continue
        dst = stage / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        else:
            shutil.copy2(src, dst)
    (stage / "bin").mkdir()
    for name in CLIENTS:
        if not (ROOT / "bin" / name).exists():
            raise SystemExit("missing bin/%s" % name)
        shutil.copy2(ROOT / "bin" / name, stage / "bin" / name)
    version = json.loads((ROOT / ".claude-plugin" / "plugin.json").read_text("utf-8"))["version"]
    manifest = json.loads((ROOT / "mcpb" / "manifest.json").read_text("utf-8"))
    manifest["version"] = version
    (stage / "manifest.json").write_text(json.dumps(manifest, indent=2), "utf-8")
    icon = ROOT / "docs" / "icon.png"
    if icon.exists():
        shutil.copy2(icon, stage / "icon.png")
    npx = shutil.which("npx") or shutil.which("npx.cmd") or "npx"
    subprocess.run([npx, "-y", "@anthropic-ai/mcpb@2.1.2", "pack", str(stage), str(out)], check=True)
    shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    main(Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "mcpb" / "lighting.mcpb").resolve())
