# Changelog

Versions follow semver. Every release on GitHub carries `lighting.exe` built from the tag by CI,
a source zip and `SHA256SUMS.txt` (including the hash of `bin/lighting.exe` in the repo at that tag).

## [1.1.0] - 2026-09-27

### Added
- Sessions survive restarts: the daemon keeps each session's window group, tabs, window refs and launched apps in
  `~/.lighting/sessions.json`, and the extension keeps tab ids across its own reloads (they start over only when
  the browser restarts). An update or `lighting stop` no longer makes a session say "no tab yet".
- `wait "text" w3` (or after an app command) waits in any app: names and values of its controls, else OCR
  (terminals, games); `--gone`, `--timeout 60s`. It does not block other commands.
- A repeated `snap` of the same page shows only what changed since your last `snap` (`changed since your last snap:
  +2, -1 gone`); `snap --full` shows everything.
- `frames --small`: a 768 px contact sheet (about 45% fewer image tokens).

### Changed
- Routines learn cleaner: a typed value that differs between two runs becomes one whole parameter named after its
  field (`title`, `comment`, not `add_a`), and a task with a long fixed text (80+ characters) is not learned.

### Fixed
- The extension id check (1.0.0) only ran on later hellos, not when the browser connected.

## [1.0.0] - 2026-09-27

Built from the stumbles in real sessions since 0.9.0, and every session gets its own browser window.

### Added
- Each Claude session works in its own browser window (same logged-in profile) with its own tab group
  `Lighting #1`, `Lighting #2`, ...; the status colour belongs to that session and `tabs` shows `L2` for its tabs.
  `config window off` keeps the tabs in your window as before.
- `claude ["prompt"] --yes [--dir path] [--name x] [--no-remote]`: a new Windows Terminal tab with Claude Code and
  Remote Control; once Claude is ready the prompt is pasted and sent (any characters work). It stops when Claude
  asks whether to trust the folder. A prompt needs `--yes`: only when the user asked for it.
- `shot screen|w2 --seconds 10`: a short H.264 video (15 fps, 1280 px wide, up to 60 s) to send to a phone.

### Fixed
- After `done` a session lost track of the tabs it opened next: every command said "this session has no tab yet"
  and those tabs stayed open. From 0.9.0 on this would have hit after every reply.
- Clicks, typing and every other action on refs in cross-origin frames (`f134.e1` from `snap --frame`) never reached
  the frame ("is gone (page changed)"), so the PayPal button of a Shopify checkout could not be clicked. The action
  now runs in its frame, at the frame's position on the page.
- `keep` dropped the tab as the target ("no tab yet" on the next command), handed over the tabs of every session,
  and the session could no longer close its own kept tab without `--force`.
- After an extension reload or a browser restart tab ids start again at `t1`, so a session could act on another tab
  with an old id; sessions now forget their tab ids when that happens.
- `snap -f "word"` said "no match on the whole page" when the word was only in text or headings; it shows those lines.
- `wait "text"` did not see names of controls (GitHub's "3 / 3 checks OK" is a button label).
- `done` without a session id (by hand, older clients) closed the tabs and apps of every session; now only its own.

## [0.9.0] - 2026-09-27

From a research round (Katalog v3): sound in videos, sessions that stay apart, fewer tokens for chat apps,
search URLs that are learned, data behind pages, routines that use routines, WebMCP.

### Added
- `listen [ref|url|file]`: what a video says, TikTok and YouTube included. Captions first (page, then yt-dlp), then
  the audio via yt-dlp, then a live recording in the tab (`captureStream` + `MediaRecorder`, tab muted, time and
  pause restored). Speech-to-text with the Groq or OpenAI key (`GROQ_API_KEY`/`OPENAI_API_KEY` or
  `~/.config/watch/.env`), or on this PC with faster-whisper (`--local`, `config audio local`, installed once).
  Segments Whisper marks as non-speech are dropped; music-heavy clips say so. `frames e40 --audio` adds the words.
- Sessions stay apart: the client sends `CLAUDE_CODE_SESSION_ID` (protocol L2; the Stop hook's `session_id` from
  stdin as a fallback). Each session has its own tabs, window refs and routine recording; a session without a tab
  gets an error instead of using another session's tab; `done` closes only that session's tabs and apps, and
  returns at once when the session opened nothing.
- `unread`: unread DMs, mentions and channels of Discord (and other chat apps) in one call.
- `search youtube "lofi"` and learned search URLs: known sites (seeds + learned in `~/.lighting/sites.json`) jump
  straight to the result page; an unknown site is searched once through its field and remembered.
- `net [n] [-f x]`: the JSON answers (GET) a Lighting tab loaded, with a compact shape (`items[] (40) {id, name}`),
  then `fetch --pick`. `open` says `data: 3 JSON calls (lighting net)`.
- WebMCP: `tools` lists the tools a page offers, `call <tool> '{json}'` runs one (`--yes` unless read-only);
  `open` says `webmcp: N tools`.
- Routines use routines: `run X k=v` as a step (3 deep, no loops), a `run` inside a task is one step, and a newly
  learned routine that contains an existing one calls it.

### Changed
- Desktop snapshots fold controls that repeat in every item into one line (`(x13, same in each item)`): the
  Discord window went from 10,235 to 6,582 characters. `--all` shows everything.
- A unit test keeps SKILL.md at 3,300 bytes or less and the MCP description at 700 characters or less.

### Fixed
- Tab aliases (`t14`) were never resolved when passed as the target, so `wait --reload` could reload the wrong tab.
- `type` into a terminal sent Ctrl+A first, which arrived as `^A`.
- Review round: `listen` through MCP gave up after 180 s (now waits as long as the listen may take); `wait 3000`
  needed a tab; a task with the same steps as a saved routine was learned again as `-2`; one-letter key hints like
  YouTube's `Pause (k)` stayed in names and were not remembered as shortcuts; smaller network buffers per tab.

## [0.8.0] - 2026-09-27

Built from what went wrong in real sessions: fewer rounds per task, fewer wrong turns, chats, shortcuts and
routines that find themselves.

### Added
- Chats: `inbox "@Name"` waits until someone else writes and prints the full message without OCR (Discord,
  other chat apps); `reply "@Name" "text" [--wait]` sends it and waits for the answer in the same call. Message
  ids come from the app (Discord `chat-messages-...`), so an answer that arrives while sending is never missed;
  own messages are skipped. Links need `--yes`, at most 5 messages without an answer.
- Long waits no longer block Lighting: `inbox`, `reply --wait` and `wait --reload` wait inside the daemon while
  other commands (and other sessions) keep running. `--timeout` takes `90s`, `10m` or milliseconds.
- `wait "text" --reload 15s --timeout 10m` reloads until the text is there (CI checks), no sleep loops.
- `-f "word"` after any action (`click`, `type`, `press`, `do`, `run`, ...) shows the matching lines of the result
  in the same call.
- `search "words"` finds the page's search field, types and submits. `open <url> --text` opens and reads.
- `open` with an app link (`obsidian://`, `spotify:`, `mailto:`) starts the app. Risky links (`file:`,
  `ms-msdt:`, `search-ms:`, Office links) need `--yes`, also for `launch`.
- `launch` answers with the app's first look (`-f` narrows it).
- Desktop `click`/`type`/`press` answer `ok (+N new, -M gone)` with the new lines, after the app went quiet;
  a repeated `snap` of an unchanged window answers `unchanged since last snap`.
- Keyboard shortcuts: Lighting keeps them per app and site (shown by the app, learned from a `press` that opened
  something, a few well-known ones), prints one `keys:` line on the first look (every 2 h at most) and lists them
  with `lighting keys [app]`, `keys add|rm`. More key names: media keys without a window (`playpause`,
  `volumeup`, ...), `numpad0-9`, `pgup`/`pgdn`, `arrowdown`, `plus`/`minus`, `altgr`, `strg`/`umschalt`,
  `f13`-`f24`; `ctrl++` works.
- Routines find themselves: a `UserPromptSubmit` hook names a fitting routine in the prompt (nothing when none
  fits, 0 tokens). Routines keep the words of the request as tags; `run discord markieren user=Tom` picks one by
  words.
- `lighting log stats`: runs, errors, time, output tokens and "snap right after the action" per command.

### Changed
- The Stop hook (`done --quiet`) runs async: replies no longer wait for the clean-up.
- `routines` prints one short line per routine; tasks that only navigate are not learned, the same steps on the
  same site update a routine instead of adding `-2`, unused learned routines are deleted after 14 days.
- A page without controls shows its visible text; a page that draws nothing yet says so and names `shot`.
- SKILL.md covers all of it in fewer bytes (3,310 -> 3,168).

### Fixed
- `launch obsidian` picked a terminal whose title mentioned Obsidian; the app's exe now wins over titles.
- `open obsidian://...` ended as an empty browser tab.
- Typing into Electron apps (Discord, Obsidian) used a background value the app never saw, so the quick switcher
  opened the wrong note; it pastes now, and `--tab`/`--enter` wait until the autocomplete has appeared.
- `type --append` into a field without a value pattern pasted the old text twice.
- `localhost:3000` was treated as a URL scheme.

## [0.7.2] - 2026-09-26

### Fixed
- Tasks with a confirmed step (`--yes`: merge, buy, delete) are no longer learned as routines by themselves:
  `run` stops before such steps anyway, so the offer "can finish this in one call" was wrong. Saving one on
  purpose with `routine save` still works.

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
