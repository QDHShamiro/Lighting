---
description: Set up Lighting and switch Claude Code over to it
---
Set up Lighting for this user and switch Claude Code over to it. Keep your report short.

1. Run `lighting setup`. It installs the Python dependencies with uv, registers the native messaging host for Brave, Chrome and Edge, copies the extension to `~/.lighting/extension` and loads it into the browser by itself through Windows UI Automation. Report the lines it prints.
2. If a browser is connected, run `lighting selftest` and report only the last line (`N/M passed`) plus any FAIL lines.
3. Switch Claude Code over. Running this command is the user's request for it, so apply these and list what you changed:
   - Merge these allow rules into `~/.claude/settings.json` under `permissions.allow` (keep every existing rule, add only missing ones): `Bash(lighting *)`, `Bash(rtk lighting *)`, `mcp__plugin_lighting_lighting__lighting`.
   - Add this line to `~/.claude/CLAUDE.md` if no line mentions Lighting yet: `- **Browser/PC:** use the lighting skill (\`lighting\` command) for all browser and desktop UI work; Claude in Chrome only as a fallback.`
4. If no browser connected, show the manual fallback that `lighting setup` printed (Developer mode, Load unpacked, path) and stop.
