# Changelog

Versions follow semver. Every release on GitHub carries the clients (`lighting.exe`, `lighting-macos`,
`lighting-linux-x64`, `lighting-linux-arm64`) built from the tag by CI, `lighting.mcpb`, a source zip and
`SHA256SUMS.txt` (including the hash of each `bin/` client in the repo at that tag).

## [Unreleased]

### Added
- `lighting tokens claude [days]`: where your Claude Code tokens go, read from the local transcripts (no upload):
  requests, context read, the cost split (cache reads, writes, output), the thinking share, what fills the context
  (earlier thinking, Bash output, start context, file reads, summaries after compact) and thinking per effort level,
  then the biggest fixes first (auto-compact window, effort, tool output, start context, cache rebuilds), each with
  the setting to change. The content split uses how much each request grew, so it needs no characters-per-token
  guess (tool output runs ~2.2 characters per token, not 4). On 30 days of real use: earlier thinking 33% of all
  context, Bash output 21%, start context 19%. Measured with 1.9 GB of transcripts: 10 s the first time, under 1.5 s after that
  (per-file cache in `~/.lighting/claude-usage.json`).

## [2.1.0] - 2026-10-22

Lighting now does the jobs of four tools at once: the memory (claude-mem), the session notes (Context Engine),
the code map (Graphify) and the wiki upkeep (an Obsidian vault). Measured on real questions from the old tools'
own data: 19 of 20 knowledge questions answered with 13,511 tokens (claude-mem: 8 of 20 with 39,401), and 9 of
10 code questions (Graphify: 2 of 10).

### Added
- `lighting code`: a code map per project (tree-sitter). Without a name an overview (languages, folders, most used
  types, entry points); `code Name` shows where it is defined and who calls it, `code callers Name` all callers,
  `code "what it does"` searches by meaning. Only changed files are read again; the meaning index builds in the
  background for projects used in the last 7 days, never while a game runs.
- Session notes write themselves: after a session the local model writes done / why / tried and rejected / left
  open as Markdown into the vault (`Sessions/<project>/`). `PreCompact` learns the session so far.
- Wiki drafts: a mistake with its cause, a rule or a pattern from a session becomes a draft in the vault inbox,
  filled along the vault's own templates; `lighting wiki`, `wiki accept <n> [folder]` (files it and adds the INDEX
  line), `wiki drop <n>` (to the vault's .trash).
- `recall` also searches the vault (`config vault <folder>`, plain Markdown, Obsidian keeps working) and Claude
  Code's own memory files, and says where a note comes from.
- `lighting memory import context-engine` copies old Context Engine session notes into the vault.
- `code outline <file>` (signatures with their lines instead of the whole file), `code body <Name|Class.name>` (just
  that function) and `code impact [--base main]` (every function the changes touch, with its callers two levels
  deep, callers in other languages left out).
- Batch: `do` is also called `batch`; `?step` is optional (a cookie banner that only comes sometimes),
  `--keep-going` runs every step and counts the failures, `--quiet` prints only the last answer, and steps can
  stand one per line (a heredoc, no `;` quoting).
- `lighting claude "task" --yes --bg` starts Claude as a background session: no window, nothing typed, so it works
  while a game runs and never takes the screen; `claude attach <id>` shows it, and in a git repo its edits land in
  a worktree under `.claude/worktrees/`. `claude list` (with the reason a session waits) and `claude logs <id>`
  (only the last lines, without terminal codes) keep an eye on them.
- A mistyped or guessed command names the right one: `find` -> `snap -f`, `code` or `recall`; `clik` -> `click`.
- A repeated `snap` of an app window shows only what changed, like pages do (`changed since your last snap: +2,
  -1 gone`); `--full` shows everything. Since 10-01, 92% of the app snapshot output (~685k tokens) was the same
  window again within two minutes, mostly chats and terminals with a line or two new.
- `lighting tokens` (and `/lighting:tokens`): what your agent read from Lighting, what the same steps would
  have cost a screenshot agent (the measured README rows: ~15,850 to open a page, ~2,350 per other look or
  action, ~14,100 per video) and the difference, per group, per day or week, plus routines and the brain
  filter. Page text, memory and code count only as used. Your log of the last 30 days: 20,654 commands, 3.9M
  tokens read, ~35M without Lighting. From now on the log also keeps how much Lighting cut from its own
  answers (snap deltas, long answers to a file, brain filter, code outlines), shown as an exact line. A snap
  delta now says what the full snapshot would cost (`all 312 lines, ~4,100 tokens: lighting snap --full`).
- After `/compact` the start info is short: the session's tabs (with the current page) and windows, whose refs
  still work, plus the project's rules and remembered notes. ~28 tokens instead of 313-461 on this PC.
- `recall` marks a note that is big in full with its size (`~1.8kt`), so opening `recall 42` is a choice.
- Text in a page, app, chat or transcript that speaks to an AI ("ignore all previous instructions", "if you are an
  AI assistant", "Hey Claude,", "do not tell the user", the same in German) gets one line that names where it is
  and that it is not a request from the user. Nothing is removed. None of 1,788 real outputs from the last 30
  days was marked by mistake.

### Changed
- The start info names the project, the open to-dos, the last 3 sessions with what is open, the last 5 commits,
  the project's rules and the drafts that wait.
- A project is the folder with its own build file in a repo with several projects, else the git repo; the home
  folder is no project, and a session belongs to the project whose files it touched.
- Hints while you type come only from the same project, need matching words and meaning, and are at most 2.
- The session note covers every part of a long session, not only the last one; empty points are left out.

### Fixed
- The memory never waits long for the model. `recall` took 2 s and the start info 2-3 s (up to 15 s) whenever the
  model server was off or busy: Windows needs 2 s to refuse a connection to a closed local port. A closed port is
  now found in 0.25 s, the start info does not ask the model at all, hints wait at most 0.8 s and `recall` at most
  4 s (then it searches by words and says so), and after a failed answer the model is left alone for 30 s.
- Hints, the start info, `recall`, `todo` and `code` no longer wait behind browser commands of other sessions:
  they run on their own lane. A prompt waited up to 5 s while another session ran `frames` or `selftest` (239
  commands of 2 s or more from 10-18 to 10-25); with four commands queued on the browser lane, `todo` took
  2.8-3.1 s and now takes 0.02-0.04 s.
- `lighting code "what it does"` no longer fails while the model is paused (a game runs) or its meaning index is
  still building: it lists the code whose names contain the words.
- Hints while you type need meaning and words also when the model is paused; before, words alone showed unrelated
  notes during games. Text that Claude Code adds by itself (task notifications) gets no hints or routines.
- Tabs closed again after a reply: the Stop hook's memory part cancelled the 20 s cleanup in 66% of the replies
  (238 of 360 from 10-18 to 10-25), so tabs of a session's last reply stayed open.
- Code map: a file indexed again kept the meaning vectors of its old version (SQLite reused the ids), so the search
  by meaning was wrong exactly where you work; all vectors are rebuilt once. Two threads no longer index the same
  project at once, `lighting code` never waits for the background index, the first call in a big repo reads for
  at most 2 s and says what is left, and files with umlauts in their path are no longer skipped.
- `config learn off`, `brain off` and `cleanup off` apply to running sessions at once (before: after up to 12 h).
- Learning a long session reads the transcript line by line and continues where it stopped; before, it loaded the
  whole rest into memory and silently dropped everything before the last 8 parts. A session that went on while
  it was being learned stays open, so its end is learned too.
- Sessions are learned again: a window that only looked fullscreen (the NVIDIA overlay, Discord filling a second
  screen) counted as a running game, so the learner stopped every minute; from 10-22 to 10-25 it learned 1 session
  and never tried the other 55. Windows' "fullscreen" state now counts only when the window in front fills its
  monitor (Direct3D fullscreen always counts, known games are still found by their process). The newest sessions
  are learned first, `lighting memory` names what learning waits for ("minecraft", or "fullscreen <program>" so a
  wrong fullscreen shows at once), and `doctor` warns when sessions pile up for a day.
- A missing vault folder (drive not ready, renamed) no longer removes all vault notes from the memory; a session
  note moves along when the session's project changes; unchecked wiki drafts are never offered as hints;
  `wiki accept` keeps the line endings of the INDEX file.
- `doctor` no longer claims Claude Code loads an old Lighting when the plugin comes from a folder marketplace:
  Claude Code loads it live from that folder, only `installed_plugins.json` keeps the first version.
- Old `listen` recordings are deleted after a day like other output (181 folders had piled up), the local model's
  logs are kept small, a stop by hotkey can no longer leave the browser channel hanging, and the routine files are
  read from disk only when they changed (10 ms -> 1.5 ms after every action).

## [2.0.0] - 2026-10-22

Built from 1,831 failed commands in the log since 1.1.0: most came from a few loops that kept hitting the same wall.

### Added
- Memory: Lighting remembers across sessions, on this PC only. `remember "fact"`, `recall "words"` (words and meaning
  mixed, one line per note, `recall 42` in full), `forget 42`, to-do lists (`todo add "task" --list x`,
  `todo doing|done|drop 12`, `todo note 12 "..."`). A session starts with a summary of about 500 tokens (open
  to-dos, the last sessions, what the user asked to keep). "merk dir ..." or "remember ..." in a prompt is saved by
  the hook at once, and up to 2 notes of the same project that clearly fit a prompt are named. Finished sessions are learned in the background by
  a local model: Lighting downloads llama.cpp, Qwen3.5 (4B, or 2B under 12 GB RAM) and Qwen3-Embedding with
  checksums, runs them at low priority and never while a game runs (full screen, Minecraft, Steam and Epic games).
  Any OpenAI-compatible server works instead (`config memory_api`). `memory import claude-mem` moves notes, session
  summaries and open to-dos over. `lighting.exe` and the extension carry the Lighting icon.
- Lighting never takes the screen from the user (Windows): no window comes to the front, the cursor does not move,
  not even out of a full-screen game. Clicks, typing, keys and the wheel go to the app window in the background as
  window messages; Ctrl+A/C/V/X/Z work there too, other shortcuts say they need the window in front. Launched apps
  and the self-test windows open without taking focus, and if an app grabs it anyway Lighting gives it back.
  `config focus away` allows the old way while the user has been away for a minute, `config focus idle` always.
- `lighting voice` (Windows): an AI joins a Discord call when the user asks (`voice join "Friends"`), hears with Groq,
  answers with Gemini and speaks with edge-tts through VB-Cable. It always says that it is an AI and listens, leaves
  when someone says "KI raus", keeps no transcript of the others and uses a persona only in the call it was written for.
- `lighting watch <link|file|e40>`: Claude watches a video in one call. The first answer is an overview: title,
  length, chapters, the speech (captions first, else speech-to-text) as a skim with times, the text on screen and
  12 scenes in one image (~50 image tokens per scene instead of one image per frame). On YouTube the scenes come from
  the preview images, so nothing is downloaded or played. `--from 4:05 --to 6:00` looks closer (denser frames, all
  speech there), `--at 1:23,2:45` gives those exact frames large enough to read code, `--text` reads the screen text
  from the real frames, `--full` prints all speech. Works for links (yt-dlp), video files (ffmpeg, which Lighting
  installs when missing) and videos on any page; results are kept for 24 h, so the next question about the same video
  answers at once.
- macOS and Linux (X11) work like Windows: desktop apps through the accessibility APIs (AX, AT-SPI) with OCR as a
  fallback, windows, keys, video of one window, a Unix socket instead of the named pipe, native host manifests per
  OS, the process list without `tasklist`, and `uv` installs itself. A Rust client is built for macOS (universal),
  Linux x64 and Linux arm64. Every push is tested on GitHub's macOS and Linux runners.
- `lighting phone ...` controls an Android phone over adb: `devices`, `snap [-f x]` (the screen as `p1`, `p2` refs;
  a settings row with a switch shows as one `checkbox` and a tap hits the switch), `tap`, `type`, `key`, `swipe`,
  `shot`, `launch`, `open`, `apps`. `phone setup` downloads Google's platform-tools.
- `lighting.mcpb`, a one-click bundle for Claude Desktop, and an entry in the official MCP registry (`server.json`),
  both published by the release workflow.
- Lighting is licensed under Apache-2.0 (was MIT), with `NOTICE`, `TRADEMARKS.md` (name and logo stay reserved) and
  `SECURITY.md`.
- `routine schedule <name> daily 08:00` (or `weekly mon 09:30`, `hourly`, `every 30m`) runs a routine on a
  schedule through the Windows Task Scheduler, launchd on macOS or a systemd user timer on Linux, without a window;
  `routine schedules` lists them, `routine unschedule <name>` (and `routine rm`) removes them.
- `lighting install antigravity`: Lighting's MCP server and skill for Google Antigravity (IDE, app and CLI share
  `~/.gemini/config/mcp_config.json`; older config and skill folders are updated when they exist).
- The brain: Lighting learns on this PC which controls are used on each page and app (it starts from the recorded
  episodes). On known pages `open`, `snap`, `back`, `forward`, `reload` and app snapshots show only those (up to 15,
  `config snap_top`) plus notes and alerts, with one line saying how many are hidden; `snap --full` shows all.
  github.com/QDHShamiro/Lighting went from ~450 to ~117 tokens.
- Self-correction when it is sure: a ref that is gone after the page changed is retried once by its role and name
  (only if that element is still there once), and a name that missed is replaced by the one that worked right after
  the same miss twice before (once: the error names it).
- `lighting brain` (what it learned), `brain forget <site|app:x.exe>`, `brain reset --yes`, `config brain off`.
- `lighting improve [days]`: a self-check of the last days (top errors with what to do, loops, what the brain saved);
  once a week a 3-line summary comes with the first prompt.
- `lighting demo [--fast]`: a ~30 s run for new users and screen recordings on a made-up shop page served from this
  computer (open, find, fill, select, check, click, table), with the time and token size of every answer and the
  page's own text size for comparison. Nothing is learned from it.
- `upload` works on file inputs inside cross-origin frames (`f134.e2`, files up to 8 MB).
- `claude --model m --effort e` starts the new Claude session with that model and effort.
- `windows -f x` with exactly one match chooses that window, so `press`/`type`/`click` without a ref go there.
- `press` says which window got the keys, and `press` and desktop `click` name new windows that opened (dialogs),
  so no extra `windows` call is needed.
- `desktop` is an alias: `lighting desktop` lists windows, `lighting desktop <cmd>` runs `<cmd>` on the desktop.
- The third identical failure of the same command in a row says that repeating it will not help.
- `lighting doctor` and `/lighting:doctor`: one line per part of Lighting (packages, client, browser link, daemon,
  stuck processes, browser, MCP, Claude Code plugin with hooks and permissions, other AIs, OCR, sound, data files,
  disk, memory, errors, focus, updates), each problem with its fix. `--fix` repairs what is safe to repair: packages,
  browser link files and registry, the client copy, PATH, unreadable data files, stuck Lighting processes, the
  daemon, the extension and other AIs' entries. `--offline` skips the update check. Works through MCP too.
- `tabs -f x` lists only the matching tabs.
- `shot --if-changed --threshold 5` counts a screenshot as unchanged when less than 5% of it moved (default 2%).
- `bench --real --runs 3` reports the median of several runs; plain `bench` also measures a call through MCP.

### Changed
- `shot w3 --seconds 10` films the window itself, also while other windows cover it (frames from the window, not
  the screen); `--screen-area` films the screen area as before. Works the same on Windows, macOS and Linux (X11).
- `done` from the Stop hook paints the group green at once and closes the session's tabs and apps 20 s later;
  any command of that session in between keeps them. A background loop no longer loses its tab when a reply ends.
- When the user is typing or moving the mouse, desktop actions wait up to 10 s for a pause (without blocking other
  sessions) instead of failing after 3 s.
- `press`, `type`, `scroll` and `drag` of a session that has no tab go to the desktop instead of failing with
  "this session has no tab yet".
- Without a chosen window, an app command uses the window in front only if the session's last `windows` showed it
  in front (never a terminal, editor, browser, chat app or admin window).
- Routine hints and the prompt `suggest` hook are stricter: a hint needs fixed text in the steps it matched (a
  routine that starts with `open https://{page}` was offered after every page), and a suggestion needs the
  routine's app or site in the prompt plus one more word.
- Desktop `type` waits 0.3 s for the UI to react (0.6 s with `--enter`/`--tab`).
- `open`, `back`, `forward` and `reload` wait at most 1.5 s for the page to calm down (was 3 s) and say so when it
  was still changing; a second `snap` then shows only what is new.
- `open` of a YouTube embed link opens the watch page (embeds opened directly show "Error 153").
- The skill text is 9% shorter (3,296 -> 3,002 bytes); it is loaded in every session that uses Lighting.
- `improve` and the weekly self-check read only the last 14 days of the log.
- A command that fails the same way 5 times in a row is answered after a growing pause (1 s, 2 s, 4 s ... up to
  30 s) with a STOP line. One agent loop on 15.10. had asked 583 times in an hour for a tab that was gone.
- `snap`, `text`, `shot`, `frames` and `expect` reopen the page when Lighting itself closed the session's tab after
  an earlier reply, and say so. Clicks and typing still stop with the hint, since the page state would be lost.
- Link targets that only repeat the link's name (`link "Issues" ->./issues`, `->/wiki/Mojang_Studios`), the home
  page, the page itself and cut-off queries are left out; external and other targets stay. `snap --links` shows all.
  Link targets were 30-46% of a snapshot: on GitHub, Wikipedia, Hugging Face and Modrinth `snap` got 29% shorter
  (11,711 -> 8,310 characters) and the first look of `open` 19% (6,866 -> 5,589).
- The skill and the "no window chosen" hint teach `snap app:spotify`, which picks the app's window without a
  `windows` call first (`windows` was the most used command: 3,011 calls in two weeks).
- Desktop `press` with keys that move or type (tab, arrows, home/end, page up/down, backspace, letters) stops waiting
  for a change after 0.25 s when nothing changed (was 0.6 s): ~880 -> ~525 ms on the test app. Shortcuts like
  `ctrl+a` still wait up to 0.6 s, and changes are still reported.

### Fixed
- `upload` on a label, menu item or button now finds its file input (the label's control, a nearby hidden
  input, or the only one on the page) instead of failing with "Node is not a file input element"; this also
  works inside same-origin frames. Without one nearby it says so.
- The daemon crashed in `open`/`tabs` when `tasklist` printed non-ASCII text (umlauts in the OEM code page).
- `snap --frame host` right after `open` failed because the frame was not there yet; it now waits up to 3 s,
  accepts the start of the host and lists the frames that exist when none matches.
- A `w` ref whose window closed and reopened (an app restart) now moves to the new window of the same app.
- `scroll w5` scrolled the last chosen window instead of w5; without a d-ref it now scrolls in the background
  when the app supports it.
- Links to files (`/file=` paths) are no longer shortened, so they can be opened as shown.
- "this session has no tab" after a cleanup names the page that was closed.
- `lighting done --quiet` and `lighting suggest` hung forever when their input stayed open without hook data (an
  agent running them in its shell); they wait at most 1 s for it now.
- `downloads` printed the whole address of a failed download, a `data:` image of 30,000+ characters; it is cut to
  80 now.
- `js` with `await` in more than one statement (`const r = await fetch(u); r.status`) failed with "await is only
  valid in async functions".
- A routine could be learned from page opens alone (two runs that opened the same test pages); a learned routine
  now needs at least one step that does something on the page.
- `listen` on a paused video recorded nothing ("the recording is empty", 13 times in a week): it now starts the
  video with a user gesture, so the browser allows it, and pauses it again afterwards.
- `listen clip.mp3` looked for the file in the daemon's folder instead of the caller's.
- Routine hints read the app of a routine that starts with `focus app:brave` as "app", so every prompt with the word
  "app" got two unrelated routines.
- `search` learned no search URL when the result page loaded slowly; it waits up to 1.5 s for the new address.
- A click in a cross-origin frame that something on the page covered (a cookie banner) went to the cover; it is made
  inside the frame now and says so.
- Linux: key combinations with Ctrl, Shift, Alt or Super failed ("unknown key 'Control_L'"), AltGr and media keys
  were unknown; the input recorder could miss the first click.
- Linux: `done` kept the session's tabs open because the window manager focuses new windows, which looked like the
  user was looking at them. Only a focus that comes later counts now.
- macOS: when the system will not bring an app to the front, keys go to the app directly instead of failing.

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
