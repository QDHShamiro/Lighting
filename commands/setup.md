---
description: Set up Lighting and switch Claude Code over to it
---
Set up Lighting for this user and switch Claude Code over to it. Keep your report short.

1. Run setup with the client for this computer:
   - Windows: `lighting setup`
   - macOS: `"${CLAUDE_PLUGIN_ROOT}/bin/lighting-macos" setup`
   - Linux: `"${CLAUDE_PLUGIN_ROOT}/bin/lighting-linux-x64" setup` (ARM: `lighting-linux-arm64`)

   It copies its programs to `~/.lighting/bin`, registers the native messaging host for Brave, Chrome,
   Edge (and Chromium on macOS/Linux), copies the extension to `~/.lighting/extension`, puts `lighting` on the PATH
   (`~/.local/bin` on macOS/Linux) and loads the extension into the browser by itself through the accessibility API.
   Report the lines it prints. On macOS/Linux it also sets `LIGHTING_CLIENT` in `~/.claude/settings.json` so the MCP
   server and hooks find the right client: tell the user to restart Claude Code once.
2. macOS only: if it says Accessibility or Screen Recording is missing, tell the user to allow the terminal app (and
   Brave/Chrome for the browser link) in System Settings > Privacy & Security, then run setup again.
3. If a browser is connected, run `lighting selftest` and report only the last line (`N/M passed`) plus any FAIL lines.
4. Switch Claude Code over. Running this command is the user's request for it, so apply these and list what you changed:
   - Merge these allow rules into `~/.claude/settings.json` under `permissions.allow` (keep every existing rule, add only missing ones): `Bash(lighting *)`, `Bash(rtk lighting *)`, `mcp__plugin_lighting_lighting__lighting`.
   - Add this line to `~/.claude/CLAUDE.md` if no line mentions Lighting yet: `- **Browser/PC:** use the lighting skill (\`lighting\` command) for all browser and desktop UI work; Claude in Chrome only as a fallback.`
5. If no browser connected, show the manual fallback that `lighting setup` printed (Developer mode, Load unpacked, path) and stop.
