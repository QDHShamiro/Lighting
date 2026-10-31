# Wisply: advanced use

## Snapshots
- Only visible, usable elements are listed. Hidden, covered (behind a modal) and aria-hidden elements are left out, which also blocks hidden prompt-injection text.
- A big nav/footer collapses to one line: `e1 nav "Main": 34 items (wisply snap -s e1)`. Long lists show 5 items plus `... +45 similar items (wisply snap -s e40 --all)`.
- When a modal is open, the snapshot shows only the modal.
- `more: 23 below` means scroll or use `--all`.
- `unchanged since last snap` means the page did not change and your refs still work.
- A repeated plain `snap` of the same page answers only what changed since your last `snap`: `changed since your last snap: +2, -1 gone (all 40 lines: wisply snap --full)` and the new lines. After a big change, a new page or with any flag you get the full view; `snap --full` always does.
- Refs stay valid until the page navigates. A stale ref answers `e12 is gone -> try: wisply snap`; when the same element (role and name) is still on the page once, Wisply uses it and says `! e12 was gone (the page changed); used the same button "Save" instead`.

## The brain (learns on this PC)
- Wisply remembers which controls you use on each page and app. On a page it knows, `open`, plain `snap`, `back`, `forward`, `reload` and app snapshots show only those (up to 15) plus notes, headings and alerts: `brain: 3 controls you use here, 30 others hidden (wisply snap --full shows all)`. Need something else: `snap --full`, `snap -f word` or `click "Name"` (searches the whole page).
- It learns from misses: when `click "Checkout"` finds nothing and `click "Go to checkout"` works right after, the next miss says so, and from the second time on it uses the name that worked (`! nothing called "Checkout" here; used "Go to checkout" ...`).
- `wisply brain` shows what it learned, `brain forget github.com` or `brain forget app:spotify.exe` drops a site, `brain reset --yes` everything (only when the user asks), `config brain off` turns it off. Nothing leaves this PC; blocklisted sites are never learned.
- `wisply improve [days]` is a self-check: the most frequent errors of the last days with what to do, loops that kept going after an error, and what the brain saved. Once a week a 3-line summary comes with the first prompt.

## Frames
- Same-origin iframes are part of the normal snapshot.
- Cross-origin iframes appear as `iframe host.com (cross-origin, wisply snap --frame host.com)`. Refs there look like `f5.e3` and work like any ref (`click f5.e3`, `type f5.e3 text`): payment and login frames included.

## Data without the UI
- `wisply fetch /api/me --pick login` runs fetch() inside the page with the user's cookies and returns compact JSON. `--pick data.items[].name` selects fields. `--method POST --body '{"a":1}'` for writes (not on blocked sites).
- `wisply table e8` or `table #prices` returns a table as tab-separated rows.
- `wisply read <url>` asks the server for markdown (Accept: text/markdown) and falls back to cleaned HTML. It sees no JavaScript content and no login: use `open` + `text` for those.
- The data behind a page: Wisply keeps the JSON answers (GET) of its tabs. `open` says `data: 3 JSON calls (wisply net)`; `wisply net [-f word]` lists them, `net 2` shows their shape (`items[] (40) {id, name, price}, total: 40`), then `fetch <url> --pick items[].name` gets exactly that, often instead of reading the page. Nothing is sent again by itself.
- `search youtube "lofi"` (or `search "words"` on the current site) opens the result page directly when the search URL is known (seeds for YouTube, GitHub, Google, Wikipedia, Amazon, Modrinth, SpigotMC, TikTok, Reddit, DuckDuckGo). Otherwise it uses the search field once and learns the URL (`! learned the search URL of ...`), kept in `~/.wisply/sites.json`.
- WebMCP (pages that offer tools to AI, Chrome origin trial): `open` says `webmcp: 2 tools (wisply tools)`; `wisply tools` lists them, `wisply call <tool> '{"q": "x"}'` runs one. Tools that can change something need `--yes`.

## Tabs and browsers
- Every Claude session works in its own browser window (the same logged-in profile) with its own tab group `Wisply #1`, `Wisply #2`, ...; calls without a session use the group "Wisply". The window opens behind the user's window and closes when its last tab closes. `wisply config window off` puts the tabs into the user's window instead. `open -n` opens another tab.
- Wisply never brings a window to the front (Windows): app clicks, typing, keys and the wheel go to the window in the background. Shortcuts with Ctrl/Alt other than Ctrl+A/C/V/X/Z fail with a hint instead; `wisply config focus away` lets them take the front when the user has been away for a minute.
- Every Claude session has its own tabs (it sends `CLAUDE_CODE_SESSION_ID`): its commands go to its own last tab, a session without a tab gets `this session has no tab yet` instead of touching another session's tab, and `done` at the end of a reply closes only that session's tabs and apps. `wisply tabs` + `tab t3` borrows any tab. Calls without a session (a plain terminal) share one context as before. Sessions survive `wisply stop`, updates and extension reloads (kept in `~/.wisply/sessions.json`); only a browser restart starts the tab ids over.
- `wisply tabs` lists Wisply's tabs plus the active tab of each window, `--all` (or `-f x`) every tab (`L` = Wisply group, `L2` = `Wisply #2`, `*` = current target, `a` = active). `wisply tab t3` works in any tab, including the user's own.
- Links that open a new tab switch the target automatically (`! new tab t9 opened from t4`).
- `close` only closes Wisply tabs and tabs this session opened, unless you add `--force`. `keep t3` takes a tab out of the cleanup; you can go on working in it.
- Several browsers connected: the last focused one is used. `wisply config browser brave|chrome|edge|auto`.

## Waiting
- Actions already wait for navigation and for the page to go quiet (about 0.1 s).
- `wait "Order placed" --timeout 20s`, `wait url:/dashboard`, `wait "Loading" --gone`, `wait 1500` (times: `90s`, `10m` or milliseconds).
- `wait "All checks have passed" --reload 15s --timeout 10m` reloads the tab every 15 s until the text is there (CI checks, deploys, queues). It waits inside the Wisply daemon: no sleep loops, and other Wisply commands keep working meanwhile. Longer than 2 minutes through Bash? Give the Bash call a longer timeout, or use the MCP tool.
- A page with no controls shows its text instead: `(no controls in view) text: ...`. `(nothing drawn yet ...)` means the page only draws in a visible window (minimized browser, some feeds): `wisply shot`.

## Status, config, logs
- `wisply status` shows the daemon, connected browsers and the current target.
- `wisply config` lists settings: `browser`, `pointer`, `ocr_lang`, `shot_width` (default 1024 px, screenshot tokens are width/28 x height/28).
- `wisply log` shows the last actions (typed text and secrets are never logged). `wisply log stats` sums them up per command: runs, error rate, time, output tokens and how often a `snap` followed an action (the next thing to save).
- Big outputs are cut and the full text is saved to `~/.wisply/out/` (path printed); `grep` that file instead of re-running.
- `--stats` after any command prints its time and token estimate.

## Safety
- Blocklist: `~/.wisply/blocklist.txt` (banking and payment domains). Those sites are read-only for Wisply.
- Buttons whose name looks irreversible (buy, pay, delete, ...) need `--yes`.
- Ctrl+Alt+End stops the running and queued commands immediately.

## Troubleshooting
- `err: no browser with the Wisply extension is connected` -> `wisply setup` (it opens the extensions page and loads the extension by itself).
- `cannot script this page` (brave://, chrome://, Web Store, PDF viewer) -> use desktop control on the browser window: `wisply snap w1 --web`.
- `dialog open: confirm "..."` -> `wisply dialog accept` or `dismiss`.
- Something hangs: `wisply stop` restarts the background process on the next call.
- `wisply selftest` runs 130 checks (browser, desktop, chat, sessions). `wisply bench --real` times six real sites and compares with the last run.

## Routines (learned tasks)
- Every successful action is kept with a stable target (`click e12` is stored as `click "Sign in"`, desktop `d5` by its name). Reads (snap, text, shot) only count as cost.
- A task ends after 45 s without commands or when a new site/app starts. Wisply then aligns it with earlier tasks (sequence alignment, like DNA matching); parts that differ between runs become parameters (`spotify:search:SOS` and `spotify:search:Numb` give `search`). Seen twice: a routine is saved and announced once (`! learned routine spotify-wiedergeben ...`).
- Starting a known task prints `! routine X can finish this in one call: wisply run X search=...` after the first matching step, when two or more steps are left.
- Even earlier: a hook reads the user's message and, when a routine fits (a word of the message is its app or site or starts with it, like `Discord-Server`, plus one more word of its name and tags; file paths and Claude Code's summary after a compaction do not count), adds one line to the prompt: `Wisply routine fits this request (one call, swap the values): wisply run discord-markieren user=Luis text='hi'`. Nothing fits: nothing is added (0 tokens). The same routine is named at most every 10 minutes, and after 5 offers in a row without a run it stays quiet until it runs again.
- Routines keep the words of the request they were learned from as tags (`markiere`, `discord`, `@`; parameter values left out), so the next request in other words still finds them. Name saved routines by intent in the user's words: `routine save discord-markieren user=Tom text=hi`.
- Routines can use routines: a step `run gh-open page=Tools` runs another one (at most 3 deep, no loops). A `run` in the middle of a task is kept as one step, and a newly learned routine that contains a whole existing routine uses `run` for that part.
- `wisply run discord markieren user=Tom text=hi` also works without the exact name: words before the `k=v` pairs pick the routine (name, then tags); several fit equally: the error names them.
- Clean-up: tasks that only navigate (open, launch, scroll, wait) are not learned, the same steps on the same site update the existing routine instead of adding `-2`, and learned routines never run within 14 days are deleted (saved and recorded ones stay).
- `wisply run X param=value` replays with fresh lookups: elements that are not there yet are retried for up to 6 s, `matches several` is retried with the first match, steps seen in only one run are optional. Steps that needed `--yes` stop the run unless the run itself gets `--yes`.
- The end state is checked (the parameter must show up in the title or URL, or the same page/app must be open). A run that does not verify counts as failed.
- Stats per routine: runs, ok, average time, tokens saved (manual cost minus the one-line answer). Three failures in the last five runs mark it `FLAKY`; flaky routines are not suggested.
- A failed run says which step broke. Finish the task by hand: when that task ends, Wisply rebuilds the routine from what worked (`! repaired routine X (v2)`).
- `wisply routines [-f word]` lists them in one short line each (`spotify-wiedergeben search= | 2x ok | 1.4 s`), `routine show X` shows steps and stats, `routine save X [param=value]` saves the task just done, `routine rm|rename`, `routine forget` clears the task history (routines stay). `wisply config learn off` stops learning.
- Files: `~/.wisply/routines/<name>.json` (steps, params, check, stats) and `~/.wisply/episodes.jsonl` (last 300 tasks, typed text included, secrets never: `--env` values are stored as `@secret`).

## Recording (the user shows it once)
- `wisply record start <name>` records the Wisply tab (clicks, form fields, keys, scrolling, typed-in URLs) and Windows apps (clicks by control name, typing, shortcuts). A red `REC` badge shows in the page and on the screen.
- Browsers and terminals are skipped on the desktop side; password fields become `@secret`. Recording stops by itself after 15 minutes (`--max` minutes).
- `wisply record stop [name] [param=value ...]` saves a routine; every `value` in the steps becomes `{param}`. `record cancel` throws it away, `record status` shows the progress.
- `--web` or `--desktop` records only one side. Steps carry `--on web|app` so mixed routines go to the right side on replay.


## Memory (kept on this PC)
- Store: `~/.wisply/memory/memory.db` (SQLite). Search mixes words (FTS5) and meaning (local embeddings); notes of the current project rank first, `recall --all` shows others with `[project]`.
- Hooks: session start prints the summary; every prompt can show up to 2 notes of the same project that clearly fit and share a word of their title with the prompt (`Wisply memory may help`); after each reply the session is queued and learned once it has been quiet 10 minutes or ended.
- Learning uses a local model: Wisply downloads llama.cpp and Qwen3.5 (4B with 12 GB RAM or more, else 2B) plus Qwen3-Embedding by itself, checks the checksums, and never runs or downloads while a game runs. `wisply memory model` shows it or starts the download; `config models_dir <folder>` puts the files on another drive.
- Own model instead: `config memory_api http://127.0.0.1:9930/v1` + `config memory_api_model <name>` + `config memory_api_embed <name>` (any OpenAI-compatible server), or files you have: `config memory_server <llama-server>`, `config memory_model <gguf>`, `config memory_embed <gguf>`.
- `memory learn` learns the waiting sessions now, `memory import claude-mem` moves notes, summaries and open to-dos over (safe to repeat), `memory off` stops the hooks, `config memory_learn off` keeps the notes but learns nothing new. `forget <id>` deletes one note.
