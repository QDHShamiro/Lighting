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
lighting/                                      Python package: daemon, CLI fallback, host, desktop, OCR, MCP, selftest
extension/                                     MV3 extension (ES modules) + page scripts + vendored Defuddle
tests/fixture.html                             selftest page (traps: hidden injection text, modal, confirm(), cookie banner, iframe, long lists)
```

Runtime state is in `%USERPROFILE%\.lighting\` (venv, key, blocklist, out, extension copy, host.bat + host.py, host json, client-root, log, config). The plugin root changes on every update, so nothing is written there; `client-root` records the active one.

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
- Rust's `Command` on Windows passes every inheritable handle to the child. A daemon started by the client inherited the caller's stdout pipe and kept it open, so `lighting ... | cat` (and the Bash tool) hung until timeout on every cold start. The client clears `HANDLE_FLAG_INHERIT` on its std handles before spawning. Python's `Popen(close_fds=True)` passes a handle list and is safe.

- Codex does not put a plugin's `bin/` on PATH. `refresh()` copies the exe to `~/.lighting/bin` (rename-then-write, a running exe can be renamed but not overwritten) and `setup` adds that folder to the user `Path`. The copied exe has no plugin root next to it, so the client falls back to `client-root` when `../lighting/__init__.py` is missing.
- `lighting install <ai>` writes absolute paths to `~/.lighting/bin/lighting.exe`: plugin roots change on every update, that path does not. It is blocked through MCP because it prints to stdout.
- Snapshot links: same-page links print `.`, links under the current path `./rest`, queries over 40 chars `?…`. On a GitHub repo page that cut the snapshot from 2,199 to 1,876 chars.

- `snap -f` first filters the visible area; with no hit there it walks the whole page (unpruned) once. Word-start hits win over mid-word hits (`-f mit` lists "MIT license", not "commits").
- A new page on the same origin whose first 6+ lines match the last page (role + name, href and `*` ignored) prints them as `e1-e19 same header as the last page`. Only on navigation (`open`, clicks that navigate, plain `snap` on a new URL); `--all`, `-s`, `-f`, `--force`, `--diff` always print everything. GitHub issues -> pulls: ~505 -> ~433 tokens.
- A version bump makes the daemon reload the extension on the next call; the first selftest after a bump can fail one check with `browser disconnected`. Run it again.

## 3. Build order when changing things

1. Python: edit `lighting/*.py`, `py_compile`, then `lighting stop` (next call starts the new daemon).
2. Extension: edit `extension/*.js`, check the modules load (copy to `.mjs`, `node --check`, import with a mocked `chrome`), then `lighting ext-reload`. If the service worker is broken it cannot reload itself: run `lighting setup`, which reloads it through the extensions page.
3. Client: `cd client && cargo build --release`, copy `target/release/lighting.exe` to `bin/`. It links the CRT statically (no VC++ runtime needed).
4. Versions: bump `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json` together; the runtime reads the Claude one. The extension manifest version is rewritten from it when the extension is copied; a mismatch makes the daemon reload the extension once.

## 4. Verification protocol (do not report done before all pass)

```
claude plugin validate .
claude plugin validate .claude-plugin/plugin.json
claude plugin validate skills
claude plugin validate commands
lighting selftest              # must print 61/61 passed
lighting bench
```
Then one real page by hand (`open`, `snap`, `click`, `text`) and one real app (`windows`, `snap w<N>`, `click d<N>`).

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
- **The `text` selftest only checked the page title**, which the plain-text fallback also contains, so the broken Defuddle bundle went unnoticed. It now requires a Markdown table from the fixture.
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
| `lighting` hangs until timeout in a pipe | daemon inherited the caller's stdout pipe | client clears `HANDLE_FLAG_INHERIT` before spawning |

## 7. Hard rules

1. No code comments; knowledge goes here.
2. Output is plain text, one line per action, no JSON, no emoji.
3. Never print secrets; `--env VAR` values stay out of logs and output.
4. Blocklisted sites stay read-only; irreversible buttons need `--yes`.
5. Every performance number in the README is measured and reproducible (`lighting bench`, `lighting selftest`, transcript timestamps).
