# Changelog

Versions follow semver. Every release on GitHub carries `lighting.exe` built from the tag by CI,
a source zip and `SHA256SUMS.txt` (including the hash of `bin/lighting.exe` in the repo at that tag).

## [0.6.0] - 2026-09-26

### Added
- Video analysis: `frames <video>` jumps to moments spread over the whole video instead of watching in
  real time (a 14-minute talk: 6 frames in 2.1 s), prints the captions at each moment, mutes the tab
  while it captures and puts the video back where it was (time, paused or playing, controls).
  `--scenes` keeps the most different frames of 4x as many samples, `--from`/`--to` pick a part,
  `--live` keeps the real-time mode for streams.
- A video that never played (YouTube shows a cover image over it) is started muted for a moment
  first; a video in a background tab waits for its metadata.

### Changed
- Page headers with 8 or more controls collapse to one line that still lists their names
  (GitHub, YouTube), so they stay clickable by text.
- The first look at a page (`open`, navigating clicks) stops at 1,600 characters as well as 40 lines:
  GitHub -7%, Wikipedia -11% tokens.
- Snapshots drop footnote links (`[1]`), keyboard hints in names (`(g then d)`) and cut long hashes
  in links to 7 characters: `snap` on GitHub -9%, Wikipedia -4%.
- Element screenshots (`shot e5`, `frames`) capture only the element through CDP instead of the whole
  viewport.

## [0.5.0] - 2026-09-26

### Added
- `lighting done` closes the tabs and apps Lighting opened and gives the foreground back. Claude Code
  runs it after every reply through a plugin Stop hook. A tab the user is looking at, an app that was
  open before and an app with unsaved work stay open.
- `lighting keep [t3|w2]` hands tabs or apps over to the user; `lighting config cleanup off` keeps
  everything open. `status` shows the setting.
- `lighting install bionic|lmstudio|cline|roo|zed`. Bionic and LM Studio also get the skill in
  `~/.lmstudio/skills/lighting`, refreshed on updates.
- Unit tests (`tests/test_units.py`, `cargo test`), CI on every push and pull request, releases with
  SHA-256 checksums from a tag.
- `bench --real` compares `text` with the page's own text and prints the saving.
- The "Lighting" tab group is a status light: orange while Lighting works, red when a step failed or
  was stopped, green after `done`.
- Images and video: `snap --media` lists images (caption as name), `frames <video>` turns a playing
  video into one contact sheet with timestamps, `read <image url|file>` saves an image to look at.

### Fixed
- The first `rtk lighting ...` of a session hung until the daemon stopped: the daemon inherited a pipe
  handle the client had inherited from rtk. The daemon is now started with an explicit handle list.
- A console window opened when the daemon started.
- `launch calc` opened OpenOffice Calc: Win+R names now come before the fuzzy Start menu match.
- `type focused` refused Store apps such as the Calculator.
- `close "app:Name"` was sent to the browser.
- `lighting stop` started a daemon when none was running.
- The copy of `lighting.exe` on PATH stayed old after a rebuild; the daemon refreshes it on start.
- `selftest` failed `mouse untouched` when the user moved the mouse; it now says SKIP.

## [0.4.0] - 2026-09-26
Routines: tasks that repeat are learned, replayed in one call, verified and repaired. Recording in
the browser and in any app. `launch` for Start menu and Store apps. Stale-page guard, media lines,
hints for nameless controls, `bench --real`.

## [0.3.2] - 2026-09-26
Whole-page filter, repeated headers collapse to one line.

## [0.3.1] - 2026-09-26
Codex plugin, `lighting install <ai>` for any MCP client, leaner output.

## [0.3.0] - 2026-09-26
Output and timing tuned on real sites.

## [0.2.0] - 2026-09-25
Speed, PDF reading, viewport, `shot --marks`.

## [0.1.0] - 2026-09-25
Browser and Windows desktop control from Claude Code.
