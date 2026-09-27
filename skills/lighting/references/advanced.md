# Lighting: advanced use

## Snapshots
- Only visible, usable elements are listed. Hidden, covered (behind a modal) and aria-hidden elements are left out, which also blocks hidden prompt-injection text.
- A big nav/footer collapses to one line: `e1 nav "Main": 34 items (lighting snap -s e1)`. Long lists show 5 items plus `... +45 similar items (lighting snap -s e40 --all)`.
- When a modal is open, the snapshot shows only the modal.
- `more: 23 below` means scroll or use `--all`.
- `unchanged since last snap` means the page did not change and your refs still work.
- Refs stay valid until the page navigates. A stale ref answers `e12 is gone -> try: lighting snap`.

## Frames
- Same-origin iframes are part of the normal snapshot.
- Cross-origin iframes appear as `iframe host.com (cross-origin, lighting snap --frame host.com)`. Refs there look like `f5.e3`.

## Data without the UI
- `lighting fetch /api/me --pick login` runs fetch() inside the page with the user's cookies and returns compact JSON. `--pick data.items[].name` selects fields. `--method POST --body '{"a":1}'` for writes (not on blocked sites).
- `lighting table e8` or `table #prices` returns a table as tab-separated rows.
- `lighting read <url>` asks the server for markdown (Accept: text/markdown) and falls back to cleaned HTML. It sees no JavaScript content and no login: use `open` + `text` for those.

## Tabs and browsers
- Lighting works in its own orange tab group "Lighting". `open -n` opens another tab.
- `lighting tabs` lists all tabs (`L` = Lighting group, `*` = current target, `a` = active). `lighting tab t3` works in any tab, including the user's own.
- Links that open a new tab switch the target automatically (`! new tab t9 opened from t4`).
- `close` only closes Lighting tabs unless you add `--force`.
- Several browsers connected: the last focused one is used. `lighting config browser brave|chrome|edge|auto`.

## Waiting
- Actions already wait for navigation and for the page to go quiet (about 0.1 s).
- `wait "Order placed" --timeout 20s`, `wait url:/dashboard`, `wait "Loading" --gone`, `wait 1500` (times: `90s`, `10m` or milliseconds).
- `wait "All checks have passed" --reload 15s --timeout 10m` reloads the tab every 15 s until the text is there (CI checks, deploys, queues). It waits inside the Lighting daemon: no sleep loops, and other Lighting commands keep working meanwhile. Longer than 2 minutes through Bash? Give the Bash call a longer timeout, or use the MCP tool.
- A page with no controls shows its text instead: `(no controls in view) text: ...`. `(nothing drawn yet ...)` means the page only draws in a visible window (minimized browser, some feeds): `lighting shot`.

## Status, config, logs
- `lighting status` shows the daemon, connected browsers and the current target.
- `lighting config` lists settings: `browser`, `pointer`, `ocr_lang`, `shot_width` (default 1024 px, screenshot tokens are width/28 x height/28).
- `lighting log` shows the last actions (typed text and secrets are never logged). `lighting log stats` sums them up per command: runs, error rate, time, output tokens and how often a `snap` followed an action (the next thing to save).
- Big outputs are cut and the full text is saved to `~/.lighting/out/` (path printed); `grep` that file instead of re-running.
- `--stats` after any command prints its time and token estimate.

## Safety
- Blocklist: `~/.lighting/blocklist.txt` (banking and payment domains). Those sites are read-only for Lighting.
- Buttons whose name looks irreversible (buy, pay, delete, ...) need `--yes`.
- Ctrl+Alt+End stops the running and queued commands immediately.

## Troubleshooting
- `err: no browser with the Lighting extension is connected` -> `lighting setup` (it opens the extensions page and loads the extension by itself).
- `cannot script this page` (brave://, chrome://, Web Store, PDF viewer) -> use desktop control on the browser window: `lighting snap w1 --web`.
- `dialog open: confirm "..."` -> `lighting dialog accept` or `dismiss`.
- Something hangs: `lighting stop` restarts the background process on the next call.
- `lighting selftest` runs 113 checks (browser, desktop, chat). `lighting bench --real` times six real sites and compares with the last run.

## Routines (learned tasks)
- Every successful action is kept with a stable target (`click e12` is stored as `click "Sign in"`, desktop `d5` by its name). Reads (snap, text, shot) only count as cost.
- A task ends after 45 s without commands or when a new site/app starts. Lighting then aligns it with earlier tasks (sequence alignment, like DNA matching); parts that differ between runs become parameters (`spotify:search:SOS` and `spotify:search:Numb` give `search`). Seen twice: a routine is saved and announced once (`! learned routine spotify-wiedergeben ...`).
- Starting a known task prints `! routine X can finish this in one call: lighting run X search=...` after the first matching step.
- Even earlier: a hook reads the user's message and, when a routine fits (its app plus a word, or two words of its name and tags), adds one line to the prompt: `Lighting routine fits this request (one call, swap the values): lighting run discord-markieren user=Luis text='hi'`. Nothing fits: nothing is added (0 tokens). The same routine is named at most every 10 minutes.
- Routines keep the words of the request they were learned from as tags (`markiere`, `discord`, `@`; parameter values left out), so the next request in other words still finds them. Name saved routines by intent in the user's words: `routine save discord-markieren user=Tom text=hi`.
- `lighting run discord markieren user=Tom text=hi` also works without the exact name: words before the `k=v` pairs pick the routine (name, then tags); several fit equally: the error names them.
- Clean-up: tasks that only navigate (open, launch, scroll, wait) are not learned, the same steps on the same site update the existing routine instead of adding `-2`, and learned routines never run within 14 days are deleted (saved and recorded ones stay).
- `lighting run X param=value` replays with fresh lookups: elements that are not there yet are retried for up to 6 s, `matches several` is retried with the first match, steps seen in only one run are optional. Steps that needed `--yes` stop the run unless the run itself gets `--yes`.
- The end state is checked (the parameter must show up in the title or URL, or the same page/app must be open). A run that does not verify counts as failed.
- Stats per routine: runs, ok, average time, tokens saved (manual cost minus the one-line answer). Three failures in the last five runs mark it `FLAKY`; flaky routines are not suggested.
- A failed run says which step broke. Finish the task by hand: when that task ends, Lighting rebuilds the routine from what worked (`! repaired routine X (v2)`).
- `lighting routines [-f word]` lists them in one short line each (`spotify-wiedergeben search= | 2x ok | 1.4 s`), `routine show X` shows steps and stats, `routine save X [param=value]` saves the task just done, `routine rm|rename`, `routine forget` clears the task history (routines stay). `lighting config learn off` stops learning.
- Files: `~/.lighting/routines/<name>.json` (steps, params, check, stats) and `~/.lighting/episodes.jsonl` (last 300 tasks, typed text included, secrets never: `--env` values are stored as `@secret`).

## Recording (the user shows it once)
- `lighting record start <name>` records the Lighting tab (clicks, form fields, keys, scrolling, typed-in URLs) and Windows apps (clicks by control name, typing, shortcuts). A red `REC` badge shows in the page and on the screen.
- Browsers and terminals are skipped on the desktop side; password fields become `@secret`. Recording stops by itself after 15 minutes (`--max` minutes).
- `lighting record stop [name] [param=value ...]` saves a routine; every `value` in the steps becomes `{param}`. `record cancel` throws it away, `record status` shows the progress.
- `--web` or `--desktop` records only one side. Steps carry `--on web|app` so mixed routines go to the right side on replay.

