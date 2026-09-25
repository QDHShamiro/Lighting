<h1 align="center">Lighting</h1>

<p align="center">
  <em>Your real browser and every Windows app, as a few lines of text.</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/claude%20code-plugin-111111?style=flat-square" alt="Claude Code plugin">
  <img src="https://img.shields.io/badge/codex-plugin-111111?style=flat-square" alt="Codex plugin">
  <img src="https://img.shields.io/badge/mcp-any%20client-111111?style=flat-square" alt="MCP for any client">
  <img src="https://img.shields.io/badge/selftest-61%2F61-111111?style=flat-square" alt="61/61 selftest">
  <img src="https://img.shields.io/badge/platform-windows-111111?style=flat-square" alt="Windows">
  <img src="https://img.shields.io/badge/license-MIT-111111?style=flat-square" alt="MIT license">
</p>

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
current page are shown as `./path`, tracking parameters are dropped, and a 60-link footer collapses
to one line. You pay for what you can act on.

---

## Measured

Same GitHub page, Brave 154, Windows 11. Tokens are characters / 4.

| Task | Claude in Chrome | Lighting |
|---|---|---|
| Open a page and get something to act on | 2 calls, 2.7 s | 1 call, **~480 tokens** |
| Snapshot again | 0.9 s, many elements without names | **0.12 s**, names + states |
| Whole page | **~13,500 tokens**, cut off at 50,000 of 64,787 chars | **~990 tokens** |
| Find one thing | `find` | `snap -f repositories`, **~50 tokens** |
| Screenshot | ~2,350 image tokens | only when you ask for one, ~700 |

- `lighting selftest`: **61/61 in 5.2 s** (browser + desktop).
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

**Cursor, Gemini CLI, VS Code (Copilot), Windsurf, Claude Desktop, any MCP client**

```bash
git clone https://github.com/QDHShamiro/Lighting.git
Lighting\bin\lighting.exe setup
lighting install cursor        # or: gemini | vscode | windsurf | claude-desktop | codex
```

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

**Windows desktop**

| | |
|---|---|
| Look | `windows` · `snap w2` (UI Automation) · `read w2` (OCR) · `shot w2` (even when covered) |
| Act | `click d5` · `click o3` (OCR text, real click, cursor goes back) · `type d5 text` · `press ctrl+s` · `clip get\|set` |

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

<details>
<summary><b>Files it creates, and how to remove it</b></summary>

Everything lives in `%USERPROFILE%\.lighting\`: the Python venv, the extension copy, the native host,
`bin\lighting.exe`, the pipe key, the blocklist, a 24 h output cache and the action log (typed text
and secrets are never logged). Registry: the native messaging host keys for Brave, Chrome, Edge and
Chromium, and `.lighting\bin` in your user `Path`.

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
