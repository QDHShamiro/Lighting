# Changelog

Versions follow semver. Every release on GitHub carries `lighting.exe` built from the tag by CI,
a source zip and `SHA256SUMS.txt` (including the hash of `bin/lighting.exe` in the repo at that tag).

## [0.7.1] - 2026-09-26

Made for how Claude actually uses it (each fix comes from a real stumble in a session).

### Fixed
- `js "const v = ...; v"` failed on the second call ("Identifier 'v' has already been declared"): code with
  its own `const`/`let`/`class` now runs in a block, the value of the last expression still comes back.
- `type focused` right after `launch` failed while Windows had not handed the new window the focus yet: it now
  brings the window forward once (only after you stopped typing) and asks again.

### Changed
- Merge buttons (`Merge pull request`, `Confirm merge`, squash/rebase) need `--yes` like buy/pay/delete:
  a routine learned from merging had been offered on every pull request page.
- The skill tells agents the three things they tripped over: refs belong to the page as it was, after an app
  command `snap/click` go to the app, big results sit in a file. It is 9% shorter anyway (~910 -> ~827 tokens).
- README: demo GIF of a real run, quick start at the top, "Built for Claude" with a snippet for `CLAUDE.md`,
  an honest comparison, what's new.

## [0.7.0] - 2026-09-26

Speed pass, every number measured before and after (details: HOW-TO-SETUP.md).

### Changed
- Every command appended to `log.jsonl` by opening and closing the file, and the virus scanner checked
  it on each close: 14.3 ms per command. The file now stays open. A browser command through MCP went
  from 17.5 ms to 2.9 ms.
- Screenshots wait for two painted frames after bringing the tab forward instead of a fixed 120 ms:
  `shot` 272 -> 172 ms, `shot e5` 219 -> 100 ms.
- `open`, `back`, `reload` wait for 150 ms of DOM quiet instead of 300 ms once the page is fully
  loaded: a light page opens in 330 ms instead of 442 ms.
- `lighting version` no longer starts Python: 157 -> 58 ms.
- The MCP tool description is shorter: 296 -> 219 tokens on every request of an MCP client.
- The Start menu list for `launch` is refreshed in the background when the daemon starts.

### Added
- `open <url> -f "a|b"` (also `-s`, `--media`) returns the filtered view right away: one call
  instead of `open` + `snap -f`.
- `~/.lighting/trace` (then `lighting stop`) writes per-command and per-browser-call timings to
  `daemon.log`.

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
