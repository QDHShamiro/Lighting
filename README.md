<h1 align="center">Lighting</h1>

<p align="center">
  <em>Your real browser and every Windows app, as a few lines of text.</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/claude%20code-plugin-111111?style=flat-square" alt="Claude Code plugin">
  <img src="https://img.shields.io/badge/codex-plugin-111111?style=flat-square" alt="Codex plugin">
  <img src="https://img.shields.io/badge/mcp-any%20client-111111?style=flat-square" alt="MCP for any client">
  <img src="https://img.shields.io/badge/routines-self--learning-111111?style=flat-square" alt="Self-learning routines">
  <img src="https://img.shields.io/badge/selftest-87%2F87-111111?style=flat-square" alt="87/87 selftest">
  <img src="https://img.shields.io/badge/platform-windows-111111?style=flat-square" alt="Windows">
  <img src="https://img.shields.io/badge/license-MIT-111111?style=flat-square" alt="MIT license">
  <br>
  <img src="https://img.shields.io/badge/first%20look%20at%20a%20page-%E2%88%9297%25%20tokens-FF8A00?style=flat-square" alt="-97% tokens per page">
  <img src="https://img.shields.io/badge/click%20result-%E2%88%9298%25%20tokens-FF8A00?style=flat-square" alt="-98% tokens per click">
  <img src="https://img.shields.io/badge/known%20task-%E2%88%9296%25%20tokens-FF8A00?style=flat-square" alt="-96% tokens per known task">
</p>

<h3 align="center">An AI that looks at a page with screenshots pays ~15,850 tokens. Lighting pays ~470.</h3>

<table>
<tr>
<th width="50%">📸 Screenshot agent · ~15,850 tokens</th>
<th width="50%">🔦 Lighting · ~470 tokens</th>
</tr>
<tr>
<td valign="top">

One screenshot (**~2,350** image tokens), then the page as text (**~13,500** tokens, cut off at 50,000 of 64,787 characters). After every click: another screenshot, another **~2,350**. And it is logged out, because it drives a fresh browser.

</td>
<td valign="top">

```
[t17] QDHShamiro/Lighting: Claude Code plugin ... - github.com/QDHShamiro/Lighting
e16 link "Issues" ->./issues
e17 link "Pull requests" ->./pulls
e27 button "Star QDHShamiro/Lighting"
...
```
One call. Everything clickable has a ref. A click answers in one line (**13-53** tokens). Your real browser, your logins.

</td>
</tr>
</table>

<p align="center"><b>A 10-step task: ~37,000 tokens with screenshots, ~920 with Lighting. That is ~36,000 tokens saved, every time.</b><br><sub>Computed from the measured rows below: 1 page look + 9 clicks with a screenshot each, against 1 <code>open</code> + 9 one-line clicks.</sub></p>

---

## The problem

You ask your AI to check something on a website. It takes a screenshot: ~2,350 tokens. It reads the
page: ~13,500 tokens, cut off halfway. It clicks, and takes another screenshot to see what happened.

Ten steps later the context is full of pixels and accessibility dumps, and the AI is still logged
out, because it drives a fresh browser that knows none of your accounts.

None of that is what the AI needs. It needs to know what it can click, and whether the click worked.

## What it does

Lighting drives **your** browser (Brave, Chrome, Edge, with all your logins) through a small
extension, and any Windows app through UI Automation and OCR. Every answer is short text with refs:

```
$ lighting open github.com/QDHShamiro/Context-Engine
[t1] QDHShamiro/Context-Engine - github.com/QDHShamiro/Context-Engine (scroll 0%)
e15 link "Code" * ->.
e16 link "Issues" ->./issues
e17 link "Pull requests" ->./pulls
e27 button "Star QDHShamiro/Context-Engine"
...
$ lighting click "Issues"
ok e16 -> github.com/QDHShamiro/Context-Engine/issues
```

```
  your AI ──Bash──→ lighting.exe (Rust, ~50 ms) ──┐
  your AI ──MCP───→ lighting mcp ─────────────────┴─→ daemon ──┬─→ extension ──→ your open browser
                                                               ├─→ UI Automation ──→ Windows apps
                                                               └─→ Windows OCR ──→ games, canvas UIs
```

Only visible, usable elements are listed. Hidden text never reaches the model, links under the
current page are shown as `./path`, tracking parameters are dropped, a header that repeats from the
last page collapses to one line, and so does a 60-link footer. You pay for what you can act on.

**It cleans up after itself.** Lighting works in its own orange "Lighting" tab group in the
background. When the AI finishes its reply, the tabs and apps it opened close again and the window
you were in comes back to the front (`lighting done`, run by a Claude Code hook). A tab you are
looking at stays, an app that was open before stays, an app with unsaved work is only asked to close.
`lighting keep` hands a tab or app over to you; `lighting config cleanup off` keeps everything open.

## The second time is one call

An AI that does a task twice pays twice: every snapshot, every click, every thought. Lighting keeps a
trace of each task with stable targets (`click "Sign in"`, not `e12`). When a task ends, it aligns it
with earlier ones like DNA sequences; the parts that differ become parameters.

```
  run 1   launch spotify:search:SOS    → snap → click "SOS wiedergeben"
  run 2   launch spotify:search:Numb   → snap → click "Numb wiedergeben"
          ! learned routine spotify-wiedergeben from 2 runs
  run 3   lighting run spotify-wiedergeben search=SOS
          ok spotify-wiedergeben: 2 steps in 1.4 s, verified -> yukan archive - SOS - Spotify.exe
```

- **Learns by itself.** Seen twice, a routine exists. No prompt, no naming, no setup.
- **Speaks up.** Start a known task and the first output says `! routine X can finish this in one call`.
- **Checks its work.** A run passes only when the result shows up (the song in the window title, the repo in the URL).
- **Repairs itself.** A step breaks, the AI finishes by hand, Lighting rebuilds the routine from what worked (`v2`).
- **Keeps score.** Runs, success rate, time and tokens saved per routine; three failures in five runs mark it `FLAKY`.
- **Or just show it.** `lighting record start`, do it yourself in the browser or any app, `lighting record stop song=SOS`.

```
$ lighting routines
github-issues page= | 2/3 ok, 3.4 s, saved ~1.7k tokens | open github.com/QDHShamiro/{page} > click Issues
spotify-wiedergeben search= | 2/2 ok, 1.4 s, saved ~181 tokens | launch spotify:search:{search} > click "{search} wiedergeben"
```

(The failed `github-issues` run was a repo that does not exist: it clicked the global Issues link, the
check caught it.)

---

## 📊 The numbers

Every number here comes from a run you can repeat: `lighting bench`, `lighting bench --real`, `lighting selftest`.
Brave 154, Windows 11, Lighting 0.5.0. Tokens are characters / 4. Small numbers stay small, and a row where the comparison does not work says so.

### Per action

| Action | Without Lighting | With Lighting | Saved |
|---|---:|---:|---:|
| Look at a page (GitHub repo) | ~15,850 (screenshot + page text) | **~470** (`open`) | **−97%** |
| See what a click did | ~2,350 (new screenshot) | **13-53** (one-line change report) | **−98%** |
| Find one thing on a page | ~2,350 (screenshot) | **~36-50** (`snap -f`) | **−98%** |
| Do a known task again (a repo's issues) | ~916 (every step by hand) | **~40** (`run github-issues`, 1 call) | **−96%** |
| Screenshot, when you really need one | ~2,350 | ~700 (1024 px JPEG) | **−70%** |

### Whole page text, 6 real sites

`lighting text` against the page's own rendered text (`document.body.innerText`), which is already much smaller than what screenshot agents read. `text` keeps the first 6,000 characters in the answer and writes the rest to a file, so long pages save the most.

| Site | Page text | `lighting text` | Saved |
|---|---:|---:|---:|
| Wikipedia, Minecraft | 38,789 | 1,509 | **−96%** |
| GitHub repo | 2,713 | 1,503 | −45% |
| YouTube home | 908 | 682 | −25% |
| Hugging Face models | 993 | 866 | −13% |
| Modrinth plugins | 1,303 | 1,203 | −8% |
| TikTok For You | (no DOM text, video only) | 24 | n/a |
| **Total** | **44,706** | **5,763** | **−87%** |

### Speed

| | |
|---|---|
| `snap` on GitHub, YouTube, Modrinth, Hugging Face, TikTok | 4-24 ms |
| Click with navigation check and change report | ~135 ms |
| `open` a real site (includes the page load) | 1.3-4.7 s |
| Cold start through `rtk` or any pipe | no hang (fixed in 0.5.0) |

- `lighting selftest`: **87/87** (browser + desktop, cold start included). Unit tests: `python tests/test_units.py` (8/8), `cargo test` in `client/`.
- `lighting bench --real` times GitHub, Wikipedia, YouTube, Modrinth, Hugging Face and TikTok, prints the page-text comparison above and compares with the last run.
- Click with navigation check and change report: ~140 ms. Filling and submitting a form: ~160 ms.
- GitHub diff with 62,000 elements: `snap` 0.11 s, `scroll` 0.17 s.
- `snap` on SpigotMC, Modrinth, Hugging Face, GitHub, YouTube, Wikipedia: 17-49 ms.

---

## Install

Requirements: Windows 10/11, [uv](https://docs.astral.sh/uv/getting-started/installation/), and Brave, Chrome or Edge.

**Claude Code**

```
/plugin marketplace add QDHShamiro/Lighting
/plugin install lighting@lighting
/lighting:setup
```

**Codex**

```
codex plugin marketplace add QDHShamiro/Lighting
```

Then install `lighting` from `/plugins`, and run once in a terminal:

```
<plugin folder>\bin\lighting.exe setup
lighting install codex
```

**Cursor, Gemini CLI, VS Code (Copilot), Windsurf, Claude Desktop, LM Studio / Bionic, Cline, Roo Code, Zed, any MCP client**

```bash
git clone https://github.com/QDHShamiro/Lighting.git
Lighting\bin\lighting.exe setup
lighting install cursor        # or: gemini | vscode | windsurf | claude-desktop | codex
                               #     bionic | lmstudio | cline | roo | zed
```

`install bionic` (same as `lmstudio`) also copies the Lighting skill into `~/.lmstudio/skills/lighting`,
where Bionic picks up global skills; later updates refresh that copy.

`setup` installs the Python side into `~/.lighting`, registers the native messaging host,
**loads the extension into your browser by itself** (it opens the extensions page, switches on
Developer mode and picks the folder through UI Automation) and puts `lighting` on your PATH.
`install <ai>` adds the MCP server to that AI's config and leaves everything else in it alone.

<details>
<summary><b>MCP config by hand</b></summary>

```json
{
  "mcpServers": {
    "lighting": {
      "command": "C:\\Users\\<you>\\.lighting\\bin\\lighting.exe",
      "args": ["mcp"]
    }
  }
}
```

One tool, `lighting`, that takes one command line (`snap -f price`). Codex uses TOML:

```toml
[mcp_servers.lighting]
command = "C:\\Users\\<you>\\.lighting\\bin\\lighting.exe"
args = ["mcp"]
```

</details>

---

## Commands

**Browser** (your normal profile, all logins)

| | |
|---|---|
| Look | `open <url>` · `snap [-f "a\|b"] [-s e40] [--diff] [--all]` · `text` (Markdown via [Defuddle](https://github.com/kepano/defuddle)) · `read <url> [url2 ...]` (pages and PDFs, no browser) · `shot --marks` |
| Act | `click` · `type` · `fill "Email=a@b.c" --submit` · `press` · `select` · `check` · `hover` · `drag` · `scroll --until "text"` · `upload` |
| Wait | `wait "text" \| url:/x \| 1500 [--gone]` · `expect "text"` |
| Data | `table e8` · `fetch /api/me --pick login` (with your cookies) · `js` |
| Tabs | `tabs` · `tab t3` · `close` · `back` · `reload` · `dialog accept\|dismiss` · `dismiss` (cookie banners) · `viewport 390x844` |
| Clean up | `done` (close what Lighting opened) · `keep [t3\|w2]` (hand it over to you) · `config cleanup off` (keep everything open) |

**Windows desktop**

| | |
|---|---|
| Start | `launch spotify` · `launch "spotify:search:SOS"` · `launch calc` (Win+R names and Store apps too) · `close w4` · `close "app:Rechner"` |
| Look | `windows` · `snap w2` (UI Automation) · `read w2` (OCR) · `shot w2` (even when covered) |
| Act | `click d5` · `click "Save"` · `click o3` (OCR text, real click, cursor goes back) · `type "Search" text` · `type focused text` · `press ctrl+s` · `clip get\|set` |

**Routines**

| | |
|---|---|
| Use | `routines [-f word]` · `run <name> param=value` · `routine show <name>` |
| Teach | learned by itself on repeat · `routine save <name> [param=value]` · `record start <name>` … `record stop [param=value]` |
| Tidy | `routine rm\|rename <name>` · `routine forget` (task history) · `config learn off` |

Clicks and typing go through UI Automation in the background, your mouse stays where it is.
`do "fill Email=a@b.c; click Continue; expect Welcome"` runs several steps in one call.
`lighting help` lists everything.

---

## Safety

- Banking and payment sites are read-only (`~/.lighting/blocklist.txt`).
- Buttons that look irreversible (buy, pay, delete, ...) need `--yes`.
- Hidden text is never shown to the model, which blocks a common prompt-injection trick.
- A URL with a long query to a site not opened yet needs `--yes`, so injected instructions cannot quietly send data out.
- Secrets come from environment variables (`--env VAR`) and are never logged.
- **Ctrl+Alt+End** stops everything immediately.
- Every push runs CI (unit tests, all modules compile, extension parses, client builds with clippy clean). Every release carries `lighting.exe` built by CI from the tag and `SHA256SUMS.txt`, which also lists the hash of `bin/lighting.exe` in the repo: check yours with `certutil -hashfile bin\lighting.exe SHA256`, or build it yourself with `cd client && cargo build --release`.

<details>
<summary><b>Files it creates, and how to remove it</b></summary>

Everything lives in `%USERPROFILE%\.lighting\`: the Python venv, the extension copy, the native host,
`bin\lighting.exe`, the pipe key, the blocklist, a 24 h output cache and the action log (typed text
and secrets are never logged). Routines live in `routines\`, the last 300 tasks they are learned from
in `episodes.jsonl` (typed text included so routines can replay it; `--env` secrets are stored as
`@secret`; `lighting routine forget` clears it, `lighting config learn off` stops it). Registry: the
native messaging host keys for Brave, Chrome, Edge and Chromium, and `.lighting\bin` in your user `Path`.

```
lighting uninstall          # registry keys off, daemon stopped
lighting uninstall --purge  # also removes ~/.lighting except the venv
```

</details>

<details>
<summary><b>How it works</b></summary>

- The daemon starts on the first call and stays. The pipe is protected by a random token in `~/.lighting/key`.
- The extension only talks to the local native host, never to web pages.
- Snapshots are built in the page by a small script (no full accessibility tree), refs stay stable in the extension's isolated world.
- JavaScript dialogs do not freeze it: it reports them and waits for `dialog accept|dismiss`.
- Links that open a new tab switch the target automatically.

`HOW-TO-SETUP.md` has every trap this works around.

</details>

## Troubleshooting

| Symptom | Fix |
|---|---|
| `no browser ... connected` | `lighting setup` |
| `lighting: command not found` | open a new terminal after `setup`, or run `bin\lighting.exe setup` again |
| `cannot script this page` (brave://, Web Store) | `lighting snap w1 --web` controls the browser window instead |
| Something hangs | `lighting stop`, the next call starts a fresh daemon |

## Limits

- Windows only. Firefox is not supported (no `debugger` extension API).
- Cross-origin iframes: snapshot with `--frame host`, clicks there are DOM clicks.
- Apps running as administrator only take input when the terminal runs as administrator too.

## Development

```
cd client && cargo build --release && copy target\release\lighting.exe ..\bin\
lighting stop && lighting ext-reload && lighting selftest && lighting bench
claude plugin validate .
```

## License

MIT. Bundles [Defuddle](https://github.com/kepano/defuddle) (MIT, see `extension/defuddle.LICENSE.txt`).
