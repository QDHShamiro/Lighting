---
description: Check every part of Lighting and repair what is broken (like /doctor, for Lighting)
---
Run `lighting doctor` and show its lines unchanged.
If its last line says `lighting doctor --fix repairs N`, run `lighting doctor --fix` and show that output unchanged too.
For every line that still says `warn` or `FAIL`:
- `-> fix: lighting setup ...`: run it and report its last line.
- any other `-> fix:` (installing ffmpeg or uv, updating the plugin, Windows settings, `/lighting:setup`): tell the user that exact step in one line, do not do it yourself.
End with one line: healthy, or what is still open.
