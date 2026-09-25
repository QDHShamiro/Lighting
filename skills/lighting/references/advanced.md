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
- `wait "Order placed" --timeout 20000`, `wait url:/dashboard`, `wait "Loading" --gone`, `wait 1500`.

## Status, config, logs
- `lighting status` shows the daemon, connected browsers and the current target.
- `lighting config` lists settings: `browser`, `pointer`, `ocr_lang`, `shot_width` (default 1024 px, screenshot tokens are width/28 x height/28).
- `lighting log` shows the last actions (typed text and secrets are never logged).
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
- `lighting selftest` runs 41 checks (browser + desktop) in about 4 s.
