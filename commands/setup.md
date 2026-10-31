---
description: Set up Wisply and switch Claude Code over to it
---
Set up Wisply for this user and switch Claude Code over to it. Keep your report short.

0. The plugin folder carries no programs. If `wisply --version` fails and `${CLAUDE_PLUGIN_ROOT}/bin/` has no client
   for this computer, download it from the latest release into `${CLAUDE_PLUGIN_ROOT}/bin/`:
   `https://github.com/QDHShamiro/Lighting/releases/latest/download/<client>` with `<client>` = `wisply.exe`
   (Windows), `wisply-macos`, `wisply-linux-x64` or `wisply-linux-arm64`, plus `SHA256SUMS.txt` from the same
   place. Compare the file's SHA-256 with its line in `SHA256SUMS.txt` and stop if it differs; on macOS/Linux
   `chmod +x` it. The full archive of the release (`wisply-<version>-<system>`) also has the window (`wisply-ui`).
1. Run setup with the client for this computer (`wisply setup` once it is on the PATH):
   - Windows: `"${CLAUDE_PLUGIN_ROOT}/bin/wisply.exe" setup`
   - macOS: `"${CLAUDE_PLUGIN_ROOT}/bin/wisply-macos" setup`
   - Linux: `"${CLAUDE_PLUGIN_ROOT}/bin/wisply-linux-x64" setup` (ARM: `wisply-linux-arm64`)

   It copies its programs to `~/.wisply/bin`, registers the native messaging host for Brave, Chrome,
   Edge (and Chromium on macOS/Linux), copies the extension to `~/.wisply/extension`, puts `wisply` on the PATH
   (`~/.local/bin` on macOS/Linux) and loads the extension into the browser by itself through the accessibility API.
   Report the lines it prints. It also sets `WISPLY_BIN` (and `WISPLY_CLIENT` on macOS/Linux) in
   `~/.claude/settings.json`, so the MCP server and hooks start from `~/.wisply/bin`: tell the user to restart
   Claude Code once.
2. macOS only: if it says Accessibility or Screen Recording is missing, tell the user to allow the terminal app (and
   Brave/Chrome for the browser link) in System Settings > Privacy & Security, then run setup again.
3. If a browser is connected, run `wisply selftest` and report only the last line (`N/M passed`) plus any FAIL lines.
4. Switch Claude Code over. Running this command is the user's request for it, so apply these and list what you changed:
   - Merge these allow rules into `~/.claude/settings.json` under `permissions.allow` (keep every existing rule, add only missing ones): `Bash(wisply *)`, `Bash(rtk wisply *)`, `mcp__plugin_wisply_wisply__wisply`.
   - Add this line to `~/.claude/CLAUDE.md` if no line mentions Wisply yet: `- **Browser/PC:** use the wisply skill (\`wisply\` command) for all browser and desktop UI work; Claude in Chrome only as a fallback.`
5. If no browser connected, show the manual fallback that `wisply setup` printed (Developer mode, Load unpacked, path) and stop.
