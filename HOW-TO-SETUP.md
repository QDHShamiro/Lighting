# HOW-TO-SETUP

Runbook for Claude Code: building, verifying and changing Lighting without rediscovering the traps.
`README.md` is for humans. This file is about getting it right.

## 1. Layout

```
.claude-plugin/plugin.json, marketplace.json   Claude Code plugin + marketplace (source "./")
.codex-plugin/plugin.json                      Codex plugin (skills only, MCP comes from `lighting install codex`)
.agents/plugins/marketplace.json               Codex marketplace (source local "./")
bin/lighting.exe                               Rust client (client/), on Claude's PATH through the plugin bin/ dir
.mcp.json                                      MCP server: lighting.exe mcp (1 tool)
skills/lighting/                               SKILL.md + references/, loaded on description match
commands/                                      /lighting:setup, :status, :bench
hooks/hooks.json                               Stop hook: `lighting done --quiet` after every Claude reply
tests/test_units.py                            unit tests, plain asserts (python tests/test_units.py or pytest)
.github/workflows/                             ci.yml (every push/PR), release.yml (tag v* -> release + SHA256)
lighting/                                      Python package: daemon, CLI fallback, host, desktop, OCR, MCP, selftest
lighting/routines.py                           task trace, repetition learning (alignment), run, repair, stats
lighting/record.py, deskrec.py                 record start/stop; low-level mouse/keyboard hooks + own UIA worker
extension/                                     MV3 extension (ES modules) + page scripts + vendored Defuddle
extension/page-rec.js                          in-page recorder (injected while recording, REC badge)
tests/fixture.html                             selftest page (traps: hidden injection text, modal, confirm(), cookie banner, iframe, long lists, nameless button, video)
```

Runtime state is in `%USERPROFILE%\.lighting\` (venv, key, blocklist, out, extension copy, host.bat + host.py, host json, client-root, log, config, bin/, routines/, episodes.jsonl, apps.json, bench-real.json). The plugin root changes on every update, so nothing is written there; `client-root` records the active one.

## 2. Ground truth (verified on Windows 11, Brave 154, Claude Code plugin docs 2026-09)

- Plugin `bin/` is on the Bash tool's PATH. A file named `lighting` (no extension) beats `lighting.exe`, so there is **only** the exe. `rtk` can start the exe, it cannot start a bash script.
- Git Bash spawning any process costs ~50-70 ms; a bash script adds another ~70 ms. That is why the client is native.
- Chromium >= 136 ignores `--remote-debugging-port` on the default profile. The extension + native messaging route is the only way to use the real profile.
- Brave reads native messaging hosts from `HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts`; Chrome and Edge have their own keys. `install.py` writes all of them.
- `brave://extensions` cannot be opened from the command line (Brave opens a blank tab instead). `autoload.py` opens a tab and types the URL.
- Brave's extensions page is a Svelte app: toggles have no name and no AutomationId. The developer mode switch is found as the toggle nearest to the text "Developer mode" / "Entwicklermodus". "Load unpacked" has a normal name.
- Brave reports its internal pages as `chrome://`, not `brave://`.
- CDP `Input.dispatchMouseEvent` type `mouseMoved` waits for a rendered frame (650-1000 ms in an occluded window). Clicks send only pressed/released; hover and drag send moves without awaiting them.
- A click that opens `alert/confirm/prompt` never gets its input ack until the dialog closes. `dispatch()` races the input against the dialog event.
- Links with `target=_blank` (noopener) have no `openerTabId`. Use `webNavigation.onCreatedNavigationTarget`.
- UI Automation returns the property default (e.g. ToggleState = indeterminate) for patterns an element does not support. Always check `Is<Pattern>PatternAvailable` first.
- `FindAllBuildCache` with a cache request is one cross-process call: 16-160 ms for typical apps. The Windows 11 Notepad needs ~800 ms regardless.
- Windows OCR: German model misreads English UI text; English model is the default. Upscaling small regions 2x helps.
- The uv venv `python.exe`/`pythonw.exe` are launchers that start the real interpreter as a child. Terminating the launcher leaves the child running: close test windows with WM_CLOSE.
- Git Bash rewrites arguments that look like POSIX paths before a native exe sees them: `/QDHShamiro` arrives as `C:/Program Files/Git/QDHShamiro`, `a=/b` as `a=B:/`. The Rust client undoes the rewrite when `MSYSTEM` is set, using `EXEPATH` to find the Git root. `/c/Users/...` still becomes `C:/Users/...`, which is what file arguments want.
- Ctrl+Pause is reported as Break (VK_CANCEL), so a Ctrl+Alt+Pause hotkey never fires. The stop hotkey is Ctrl+Alt+End.
- A SYSTEM process can hold the foreground (seen: a hidden `GameInputServiceWindow` of GameInputSvc.exe). While it does, Windows drops injected input (UIPI) and refuses `SetForegroundWindow`. `pointer.front()` tries SetForegroundWindow, then UIA `SetFocus`, then minimize/restore, and reports failure instead of typing into the wrong window.
- `Runtime.evaluate` with `replMode: true` returns an unresolved promise object (`{}`) for promise results. `js` wraps code with `return` or `await` in an async IIFE instead.
- `fetch` with `credentials: "include"` fails against APIs that answer `Access-Control-Allow-Origin: *`. Default is `same-origin`; `--cookies` forces include.
- After a daemon restart the extension needs ~0.3-1 s to reconnect. `ensure_host` waits up to 6 s when the browser process is running instead of failing at once, and `any_browser` (routing, `status`, `selftest`) waits until the daemon is `RECONNECT_GRACE` (2.5 s) old, so a first `click` is not routed to the desktop.
- `cmd.exe` re-reads a running batch file from its old byte offset after each command. Rewriting `host.bat` while Brave's host runs made cmd start a second host that waited forever for a hello the extension had already sent, and the extension never saw a disconnect. `host.bat` is therefore one static line (`%~dp0`-relative, `& exit /b`), written only when its bytes change; `host.py` next to it reads the plugin root from `client-root` at start.
- `elementFromPoint` is a full hit test. On a GitHub diff with 62,000 elements it costs ~14 ms per call, which made the old 3-point cover check take 0.8 s per snapshot. Snapshots now hit-test only items that overlap a fixed/sticky/absolute element (collected during the walk), within a 60 ms budget, and skip off-screen `li/tr/article/row` subtrees (their controls are only counted). Elements of 1 px or less (screen-reader-only) are not listed.
- Text search (`click "..."`) first searches the visible area only and stops on an exact visible match; only then it walks the whole page. A cheap `textContent`/attribute prefilter (whitespace removed) avoids `innerText` on non-matching elements.
- CDP `mouseWheel` in a background or occluded window waits for a rendered frame (2.2 s on GitHub). `scroll` moves the nearest scrollable ancestor with JS; the wheel event is only the fallback for pages that cannot scroll at all (maps, canvas apps).
- `chrome.storage.session` is cleared on every extension reload (update, `ext-reload`). The target falls back to the most recently used tab in the "Lighting" group, and tab aliases the extension does not know are rejected instead of being read as raw tab ids.
- Device-metrics overrides in a background tab: a second change waits for a frame ack that never comes. `viewport` polls `innerWidth` and shows the tab for a moment if nothing changed within 250 ms.
- Extensions cannot open `data:` URLs as top-level pages (the tab stays `about:blank`).
- The Rust client treats the venv as stale when `requirements.txt` or `plugin.json` is newer than `venv/.lighting-stamp`; the Python side touches the stamp when nothing had to change. Without that a new dependency never got installed within the same plugin root.
- Chrome rejects an injected content script that contains a Unicode noncharacter (a raw U+FFFF sat in a Defuddle regex) with "It isn't UTF-8 encoded". Defuddle never loaded until the bundle was patched to use `￿` escapes; `text` silently used the `innerText` fallback. The injection error is now reported in the output.
- Defuddle returns only the navigation on listing and app pages (Modrinth, YouTube home, SpigotMC lists). `text` switches to the page text when the reader view is under 30% of it.
- Python's OpenSSL builds certificate chains from the Windows store and can pick an expired intermediate (`read https://en.wikipedia.org` failed, the browser was fine). `read` verifies with `truststore` (native Windows verification).
- Background tabs render no frames, so CSS animations never advance: a GitHub overlay stayed at opacity 0 and was invisible to snapshots. Finite running animations are finished before every snapshot and text search (infinite ones are left alone).
- GitHub hydrates `react-partial` islands on first interaction; the click's effect arrives ~300-400 ms later. When the diff after a click is empty and the clicked element did not change state, `after()` waits up to 500 ms for a mutation and diffs again. A changed element state is reported directly (`ok -> e5 checkbox "x" [x]`).
- SPA navigation (GitHub Turbo/React) changes the URL first and renders later. Clicks on links to another URL wait up to 3 s for the navigation to start, then for the title to change, then for quiet.
- `[tabindex]` alone does not make a control: focusable scroll containers and tooltip triggers (`tabindex=0`, no pointer cursor) are skipped; empty "stretched" links take the card's title as name.
- Rust's `Command` on Windows passes every inheritable handle to the child, including handles the client itself inherited from its parent. `rtk` hands its own pipe down that way, so clearing `HANDLE_FLAG_INHERIT` on the client's three std handles was not enough: the daemon kept rtk's pipe open and the first `rtk lighting ...` of every session hung until the daemon stopped. The client now starts the daemon with `CreateProcessW` and `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (only the log handle). Python's `Popen(close_fds=True)` passes a handle list and is safe.
- `DETACHED_PROCESS` makes Windows ignore `CREATE_NO_WINDOW`. The uv `pythonw.exe` launcher starts a console `python.exe`, which then got a new, visible console window. Spawn with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` only (`HIDDEN`); the child shares the hidden console.
- `~/.lighting/bin/lighting.exe` comes first on PATH for bash, rtk and other AIs. It used to be refreshed only on a version bump, so a rebuilt client was never used from the shell. The daemon copies it on every start.
- Store (UWP) apps: the window belongs to `ApplicationFrameHost.exe`, the focused element to the app (`CalculatorApp.exe`). `type focused` accepts a focused element from another process when the target window is in the foreground.
- `launch` order: exact Start menu name, then Win+R names (`calc`, `notepad`: an .exe in System32/Windows or an App Paths entry), then the fuzzy Start menu match. "calc" is the Start menu name of OpenOffice Calc; the Windows calculator is "Rechner" on German Windows.
- Cleanup: `done` asks every connected extension to close the non-active tabs of the Lighting group (a tab the user made active is being looked at and stays), and closes the windows `launch` created (hwnd not in the window list before the launch; apps that were open stay) with WM_CLOSE, then gives the foreground back to the window that had it before the first launch. `keep` ungroups tabs and forgets windows. The Stop hook runs `done --quiet`; for `done` the client never sets anything up and never starts the daemon.
- Status light: `paint()` in tabs.js sets the Lighting group color, orange before every command that works on a tab, red on an error or `abort`, green on `cleanup` (also with `cleanup off`, then nothing closes). New groups take the current color.
- Images are listed only with `snap --media` (image-heavy pages would cost tokens otherwise); they keep the visibility and aria-hidden checks that video skips, icons under 48 px are left out, and an image without alt takes its `figcaption`. `frames` keeps the tab in front for the whole capture because background tabs stop decoding video; "did not move" comes from equal timestamps or equal thumbnails (JPEG noise alone breaks exact comparisons).
- Video analysis: `frames` seeks by default (sample i at from + (i + 0.5) * span / n), waits for `seeked` and `readyState >= 2`, then two animation frames (or 150 ms), and captures with `Page.captureScreenshot` + `clip` (rect center - size/2 + `cssVisualViewport.pageX/pageY`; rect first, it may scroll). It mutes the tab during capture and restores time, paused state and `controls` afterwards. A video counts as paused when it was paused or never loaded (`readyState < 2`): YouTube reports a deferred background video as playing, and restoring that once started it with sound.
- Chrome defers media in background tabs until they are shown: `frames` waits up to 4 s for metadata while the tab is in front. A video that never played (`played.length === 0`) shows YouTube's cover image over the element, so every seek captured the same picture; it is started muted until `currentTime` moves, then paused.
- YouTube `/embed/` URLs opened directly show "Error 153" (the player needs a referrer). Test with watch pages.
- Captions: text tracks with mode `disabled` are set to `hidden` so their cues load (restored afterwards); YouTube captions are read from `.ytp-caption-segment` when CC is on.
- Header collapse: `<header>`/`banner` outside article/aside/main/nav/section is a landmark; it collapses at 8+ own controls (navs inside it keep their own lines) and lists names up to 150 characters. The first-look cap is 40 lines and `navChars` 1,600 characters: with only a line cap, collapsing the header just pulled more lines in and `open` got bigger.
- Where the time goes (0.7.0, measured): a Bash call costs 51-57 ms of Git Bash process start alone (`/usr/bin/true` 51 ms), Lighting adds ~5 ms. Through MCP a call is 1.2 ms (`ping`) and a browser command 2.9 ms; the daemon -> extension -> daemon trip is 1.7 ms. For an agent the model round per step (1-5 s) dominates, so fewer steps (`do`, `open -f`, routines) beat shaving milliseconds.
- Opening and closing a file per command cost 14.3 ms because the virus scanner checks it on close. `log.jsonl` stays open (line buffered) and is rotated every 500 lines. Same trap for any per-command file write.
- Profiling switch: create `~/.lighting/trace`, then `lighting stop`; `daemon.log` gets `trace <cmd>: route / observe / total / log` and `trace browser <cmd>: reply / inside extension` lines. Delete the file and stop again to turn it off (read once at start, no cost when off).
- Tried and dropped (no measurable effect): `timeBeginPeriod(1)` in daemon and host, CDP `optimizeForSpeed` for JPEG screenshots (272 vs 278 ms). Pillow `optimize=True` only made files smaller (tokens depend on pixels), dropping it saved ~5 ms.
- `showBriefly` waits for two animation frames in the tab (executeScript, max 300 ms; 120 ms if the page cannot be scripted) instead of a fixed sleep: faster on a 144 Hz screen and safe on slow pages.
- After a desktop command, shared verbs (`snap`, `click`, ...) go to the desktop. Test the web side with web-only commands (`open`, `text`) or `--on web`.
- Bionic (LM Studio's agent app) reads global skills from `~/.lmstudio/skills/<name>/SKILL.md` (folder name = skill name) and LM Studio reads MCP servers from `~/.lmstudio/mcp.json`.

- Codex does not put a plugin's `bin/` on PATH. `refresh()` copies the exe to `~/.lighting/bin` (rename-then-write, a running exe can be renamed but not overwritten) and `setup` adds that folder to the user `Path`. The copied exe has no plugin root next to it, so the client falls back to `client-root` when `../lighting/__init__.py` is missing.
- `lighting install <ai>` writes absolute paths to `~/.lighting/bin/lighting.exe`: plugin roots change on every update, that path does not. It is blocked through MCP because it prints to stdout.
- Snapshot links: same-page links print `.`, links under the current path `./rest`, queries over 40 chars `?…`. On a GitHub repo page that cut the snapshot from 2,199 to 1,876 chars.

- `snap -f` first filters the visible area; with no hit there it walks the whole page (unpruned) once. Word-start hits win over mid-word hits (`-f mit` lists "MIT license", not "commits").
- A new page on the same origin whose first 6+ lines match the last page (role + name, href and `*` ignored) prints them as `e1-e19 same header as the last page`. Only on navigation (`open`, clicks that navigate, plain `snap` on a new URL); `--all`, `-s`, `-f`, `--force`, `--diff` always print everything. GitHub issues -> pulls: ~505 -> ~433 tokens.
- A version bump makes the daemon reload the extension on the next call; the first selftest after a bump can fail one check with `browser disconnected`. Run it again.

- Filters rank per term: whole word > word start > substring, and only each term's best tier is shown (`-f sos` drops "Sosa La M" when "SOS" exists). Same code for web (`page.js`), desktop snapshots, `windows -f`, OCR `read -f` (`common.tiers` / `best_only`) and desktop `click "text"`, where equal names prefer button > link > menuitem > ... > dataitem (Spotify rows have a dataitem and a button with the same name; the dataitem only selects).
- Stale guard: the background keeps the last URL (origin + path + query) of every tab after each command. A text-targeted click/type/fill/select/check/hover on a tab whose URL changed in between (user navigated, autoplay, pushState) is refused once with the new URL. Ref targets are not affected (refs die with the page anyway).
- `<video>`/`<audio controls>` get a line (`e40 video 0:02/0:15 playing muted`). TikTok's playing video has `opacity: 0` and sits under an overlay, so media skip the opacity, aria-hidden and cover checks.
- Nameless controls print a hint from `data-e2e`/`data-testid`/`data-test`/`data-qa`/`data-cy`/`id`/`name` (`button #feed-follow`); `click "#feed-follow"` resolves it. Generic or random values (`tux-web-icon-button`, 4+ digits, hashes) are skipped.
- `launch` resolves names with `Get-StartApps` (Store apps included, ~1.5 s, cached in `apps.json` for a day) and starts `shell:AppsFolder\<AppID>`. The window is the new foreground/new window, stable for 0.6 s; single-instance apps that were already open are found by exe/title after 0.5 s. German Windows calls Notepad "Editor".
- Desktop `click "text"`/`type "Field"` re-snapshot when the last snapshot is older than 1.5 s or from another window: UIA elements get reused with new names (Spotify search results), so old refs pointed at off-screen rows.
- Routines: `commands.run_one` calls `routines.observe` after every non-system command. Actions become stable steps (`ctx.last_target` holds the resolved name/role/hint of e-/d-refs, the extension returns `target` + `where` with every tab command). Episodes end after 45 s idle or when an entry command (`open`/`launch`/`focus`) switches site after 2+ actions, or at `stop`. Learning = Smith-Waterman local alignment of the new episode against the last 150 (step similarity = token LCS ratio, gaps allowed), then a second pass after substituting the slot values found in the first. Differing token runs (punctuation glue merged) become params named after the fill/type label, the key before `:`/`=`, else item/page/app/text.
- `run` retries transient errors (nothing matches, is gone, page changed, ...) for up to 6 s per step, adds `--first` on "matches several", skips optional steps, stops before `--yes` steps, verifies the end state (params in title/URL, else same URL prefix / same exe). Unverified = failed. A run that fails and is finished by hand becomes the next version at episode end (`repair`). Selftest and bench set `ctx.no_learn`.
- Recorder: web events come from `page-rec.js` in the isolated world (trusted events only; Lighting's own CDP input counts as trusted, its `selectOpt`/`setValue` synthetic events do not). Desktop: `WH_MOUSE_LL` + `WH_KEYBOARD_LL` on their own thread with a message loop; the callbacks only queue, a second thread with its **own** COM apartment and `IUIAutomation` resolves `ElementFromPoint`/`GetFocusedElement` (UIA objects from the command thread cannot be used there). Injected input is ignored unless `--injected` (selftest). Keys become text via `ToUnicodeEx` with flag 4 (keeps dead-key state), AltGr = ctrl+alt + printable char.
- Writing other programs' config files: keep their line endings (`eol_of`). Python's `write_text` on Windows turned the LF `~/.codex/config.toml` into CRLF.

## 3. Build order when changing things

1. Python: edit `lighting/*.py`, `py_compile`, then `lighting stop` (next call starts the new daemon).
2. Extension: edit `extension/*.js`, check the modules load (copy to `.mjs`, `node --check`, import with a mocked `chrome`), then `lighting ext-reload`. If the service worker is broken it cannot reload itself: run `lighting setup`, which reloads it through the extensions page.
3. Client: `cd client && cargo build --release`, copy `target/release/lighting.exe` to `bin/` (rename the old one to `bin/lighting.old` if it is running). It links the CRT statically (no VC++ runtime needed). `lighting stop`: the next daemon copies it to `~/.lighting/bin`, which is the one on PATH.
4. Versions: bump `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json` together; the runtime reads the Claude one. The extension manifest version is rewritten from it when the extension is copied; a mismatch makes the daemon reload the extension once.

## 4. Verification protocol (do not report done before all pass)

```
claude plugin validate .
claude plugin validate .claude-plugin/plugin.json
claude plugin validate skills
claude plugin validate commands
python tests/test_units.py     # 9/9
cd client && cargo test && cargo clippy --release --all-targets
lighting selftest              # must print 95/95 passed (94/94 with a SKIP line when other Lighting tabs are open or you used the mouse)
lighting bench
lighting bench --real          # compare tokens with the last run
```
Cold start through a pipe, isolated from the browser's own daemon restart: `USERNAME=ltest lighting stop; rtk lighting status | cat` must return at once (`USERNAME` picks the pipe name).
Then one real page by hand (`open`, `snap`, `click`, `text`) and one real app (`windows`, `snap w<N>`, `click d<N>`).
For routines: do one task twice with different values, `lighting routine learn`, then `lighting run <name> param=...`.

## 5. Mistakes already made (do not repeat)

- **Patching JS through Python heredocs** turned `\n` into a real line break and `\u0000` into a NUL byte. The extension died silently. Use the Edit tool for anything with backslashes, then run the module check.
- **`ext-reload` before the extension reconnected** posted the reload into nothing and the old code kept running. Wait for a connected host, post, then wait for a *new* host.
- **Selftest counted clicks from an old test window** because terminating the uv launcher did not close the app.
- **`read <url>` leaked hidden injection text** until `html2md` learned `hidden`, `aria-hidden`, inline hiding styles and classes hidden in `<style>` blocks.
- **Diff after an action overwrote the snapshot state**, so the next `snap` claimed "unchanged". `diff()` only updates the ref set.
- **Filtered desktop snapshots dropped refs**, so `click "Go"` after `snap -f status` failed. Refs are kept for all items; the filter only hides lines.
- **Testing the hotkey with injected keys while a SYSTEM window was in front** showed nothing; the hotkey was fine. Bring a normal window to the front first (`pointer.front`), then inject.
- **`fetch /path` from Git Bash failed** because the path had been rewritten to the Git install folder before Lighting saw it.
- **Switching plugin roots killed the browser connection for good** because `refresh()` rewrote `host.bat` under a running `cmd.exe`, and `rmtree` of the loaded extension copy left a window where the files were missing. Now the host files are static and the extension copy is overwritten in place.
- **Every cold start hung the calling shell** because the daemon inherited the client's stdout pipe. Test cold starts through a pipe: `lighting stop; timeout 25 lighting status | cat`.
- **The selftest reused the working tab** (`open` without `--new`) and closed it at the end. It now opens its own tabs, and a decoy tab proves that `close` returns to the previous tab instead of the first tab of the group (that bug made a test click land on GitHub).
- **Benchmarks on the small test page hid the big-page costs.** Measure on a real heavy page too (a GitHub compare view with a large diff).
- **A Python heredoc turned `\n` into a real line break again** (selftest.py). Anything with backslashes goes through the Edit tool.
- **And again in 0.4.0** (`\r\n` in install.py, a `\\[` regex in page.js). Heredoc replacements with backslashes either break the file or silently match nothing. Edit tool, or `chr(13)`/`chr(10)` in generated code.
- **The first routine learner treated flags as data**: `open x -n` vs `open y` split "Context-Engine" into two parameters. Slots are computed from arguments only; flags only affect similarity.
- **`routine learn` "worked" while nothing was verified**: a run that clicked the wrong "Issues" link reported ok. Verification failures now fail the run and count in the stats.
- **The `text` selftest only checked the page title**, which the plain-text fallback also contains, so the broken Defuddle bundle went unnoticed. It now requires a Markdown table from the fixture.
- **The first `rtk lighting ...` of every session hung for minutes** (0.4.0): cold-start tests used a plain pipe, and after `lighting stop` the browser's host restarted the daemon before the client could, so the client never spawned it. Test with a private pipe (`USERNAME=ltest`) and through `rtk`.
- **Tests of a rebuilt client ran the old one**: `lighting` resolved to the stale `~/.lighting/bin` copy. Compare `md5sum bin/lighting.exe ~/.lighting/bin/lighting.exe`.
- **Only the fixture was tested.** A sweep over real sites (GitHub, YouTube, Wikipedia, SpigotMC, Modrinth, Hugging Face, PaperMC docs) found six output and timing problems in one hour. Repeat that sweep after bigger changes.

## 6. Failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `no browser ... connected` | extension not loaded, or service worker crashed | `lighting setup` |
| Every click ~1 s | `mouseMoved` awaited | send pressed/released only |
| `browser did not answer 'click' within 35s` | JS dialog opened by the click | `dispatch()` + `lighting dialog accept` |
| Buttons show `[-]` | toggle state read without pattern check | check availability properties |
| `lighting` runs slow (~200 ms) | bash script shadowing the exe | keep only `bin/lighting.exe` |
| Host never connects after update | old multi-line `host.bat` rewritten under a running cmd.exe | static one-line `host.bat` + `host.py` reading `client-root`; kill the stuck `host.py` python once |
| Daemon restarts on every call | two open Claude sessions run different plugin versions, each call switches `client-root` | expected until the old session ends; same version = no restart |
| `lighting` hangs until timeout in a pipe | daemon inherited a pipe handle (the caller's, or one the caller inherited, e.g. from rtk) | client spawns the daemon with an explicit handle list |
| A console window pops up when the daemon starts | `DETACHED_PROCESS` voids `CREATE_NO_WINDOW` for the launcher's child | spawn with `HIDDEN` flags only |
| `launch calc` opens OpenOffice Calc | fuzzy Start menu match won | Win+R names come before the fuzzy match |

## 7. Hard rules

1. No code comments; knowledge goes here.
2. Output is plain text, one line per action, no JSON, no emoji.
3. Never print secrets; `--env VAR` values stay out of logs and output.
4. Blocklisted sites stay read-only; irreversible buttons need `--yes`.
5. Every performance number in the README is measured and reproducible (`lighting bench`, `lighting selftest`, transcript timestamps).
