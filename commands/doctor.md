---
description: Check every part of Wisply and repair what is broken (like /doctor, for Wisply)
---
Run `wisply doctor` and show its lines unchanged.
If its last line says `wisply doctor --fix repairs N`, run `wisply doctor --fix` and show that output unchanged too.
For every line that still says `warn` or `FAIL`:
- `-> fix: wisply setup ...`: run it and report its last line.
- any other `-> fix:` (installing ffmpeg or uv, updating the plugin, Windows settings, `/wisply:setup`): tell the user that exact step in one line, do not do it yourself.
End with one line: healthy, or what is still open.
