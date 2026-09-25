# Lighting

Browser and Windows desktop control for [Claude Code](https://claude.com/claude-code), built for **few tokens and fast steps**.

Lighting drives your real, logged-in browser (Brave, Chrome, Edge) through a small extension and any Windows app through UI Automation and OCR. It answers in short text lines with refs instead of accessibility dumps and screenshots.

```
$ lighting open github.com/QDHShamiro
[t1] QDHShamiro (QDHShamiro) - github.com/QDHShamiro (scroll 0%)
e13 link "Overview" * ->/QDHShamiro
e14 link "Repositories 44 (44)" ->/QDHShamiro?tab=repositories
...
$ lighting click "Repositories"
ok e14 -> github.com/QDHShamiro?tab=repositories
```

## Measured: same page, same machine

GitHub profile page, Brave 154 on Windows 11. Lighting numbers are the CLI call in Bash. Claude in Chrome numbers come from the tool-call timestamps in the Claude Code transcript. Tokens are estimated as characters / 4.

| Task | Claude in Chrome | Lighting |
|---|---|---|
| Open the page and get something to act on | 2 calls: `navigate` 1.8 s + `read_page` 0.9 s | 1 call: `open` 2.2 s (includes GitHub's load time), ~480 tokens |
| Snapshot again | `read_page interactive` 0.9 s, many elements without names | `snap` 0.12 s, ~490 tokens, names + states |
| Whole page | `read_page` 0.37 s, **~13,500 tokens**, cut off at 50,000 of 64,787 chars | `snap --all` 0.12 s, **~990 tokens** |
| Find one thing | `find` (not measured) | `snap -f repositories` 0.12 s, ~50 tokens |
| Screenshot | 1920x951 viewport = ~2,350 image tokens (computed, 28 px patches) | `shot` writes a 1024 px JPEG (~700 tokens, computed), only if you Read it |

Built-in test page (`lighting selftest`, 61 checks, browser + desktop): **61/61 in 5.2 s**. A click including navigation check and change report: ~140 ms. Filling and submitting a 4-field form: ~160 ms. A CLI round trip: ~50 ms.

Big pages stay fast. GitHub compare view with a 7,000-line diff (62,000 elements), same machine:

| Step | 0.1.0 | now |
|---|---|---|
| `snap` | 1.2 s | 0.11 s |
| `click "Text"` / `hover "Text"` on a visible element | 1.2 s | 0.4 s |
| `scroll down` | 2.2 s | 0.17 s |

Real sites after load, same machine: `snap` takes 17-49 ms on SpigotMC, Modrinth, Hugging Face, GitHub, YouTube and Wikipedia. `text` returns the article as Markdown via Defuddle (Wikipedia's Minecraft article: 0.8 s) and plain page text on listing pages; links are reduced to their text unless `--links` is given.

## What it does

**Browser** (your normal profile, all logins)
- `open`, `snap` (visible usable elements, covered and hidden ones left out, `-f "login|email"` filters), `text` (page as markdown via [Defuddle](https://github.com/kepano/defuddle)), `read <url> [url2 ...]` (pages and PDFs as text without a browser, in parallel)
- `click`, `type`, `fill "Label=value" --submit`, `press`, `select`, `check`, `hover`, `drag`, `scroll --until`
- `wait`, `expect`, `table`, `fetch --pick` (JSON with your cookies), `js` (or `js --file`), `upload`, `downloads`, `console`, `dialog accept|dismiss`, `dismiss` (cookie banners)
- `shot --marks` puts the e-refs on the screenshot, `viewport 390x844` shows the mobile layout
- Works in its own orange tab group, can take over any tab on request, follows links that open new tabs
- JavaScript dialogs do not freeze it; it tells you and waits for `dialog accept|dismiss`

**Windows desktop**
- `windows`, `snap w2` (UI Automation controls), `click d5`, `type d5 text`, `press ctrl+s`, `scroll`, `drag`
- Background first: clicks and typing use UI Automation patterns, your mouse stays where it is
- `read w2` reads screen text with Windows OCR (games, canvas UIs), `click o3` clicks that text with a quick real click and puts your cursor back
- `shot w2` and `read w2` capture the window itself, even when other windows cover it
- An orange pointer shows where Lighting acts

**Built for agents**
- One line per action: `ok`, `ok -> new-url` with a mini snapshot, or `ok (+3 new: ...)`
- `do "step; step; step"` runs several steps in one call
- Big outputs are cut and saved to a file you can grep
- Errors say what to do next: `err: e12 is covered by div.cookie -> try: lighting dismiss`

## Install (Claude Code plugin)

Requirements: Windows 10/11, [uv](https://docs.astral.sh/uv/getting-started/installation/), and Brave, Chrome or Edge.

```
/plugin marketplace add QDHShamiro/lighting
/plugin install lighting@lighting
/lighting:setup
```

`/lighting:setup` installs the Python dependencies into `~/.lighting/venv`, registers the native messaging host and **loads the extension into your browser by itself** (it opens the extensions page, switches on Developer mode and picks the folder through Windows UI Automation). If that is not possible it prints the three manual steps.

The plugin adds the `lighting` command to Claude Code's shell, a skill that teaches Claude to use it cheaply, the commands `/lighting:setup`, `/lighting:status`, `/lighting:bench`, and an MCP tool `lighting` for clients without a shell.

## How it works

```
Claude Code --Bash--> bin/lighting.exe (Rust, ~50 ms) --+
Claude Code --MCP---> lighting mcp -----------------------+--> named pipe --> lighting daemon (Python)
                                                                                |-- UI Automation, OCR, input, pointer
                                                                                +-- native host --> browser extension (MV3)
                                                                                                     chrome.debugger + page scripts
```

- The daemon starts on the first call and stays. The pipe is protected by a random token in `~/.lighting/key`.
- The extension only talks to the local native host, never to web pages.
- Snapshots are built in the page by a small script (no full accessibility tree), with stable refs kept in the extension's isolated world.

## Safety

- Banking and payment sites are read-only (`~/.lighting/blocklist.txt`).
- Buttons that look irreversible (buy, pay, delete, ...) need `--yes`.
- Hidden text is never shown to the model, which blocks a common prompt-injection trick.
- Secrets come from environment variables (`--env VAR`) and are never logged.
- A URL that carries a long query to a site not opened yet (`open`, `fetch`, `read`) needs `--yes`, so injected instructions cannot quietly send data out.
- **Ctrl+Alt+End** stops everything immediately.
- The browser shows its "is being debugged" bar while Lighting controls a tab.

## Commands

Run `lighting help` for the full list. The skill in `skills/lighting/` documents every command for Claude.

## Development

```
cd client && cargo build --release && copy target\release\lighting.exe ..\bin\
lighting stop            # restart the daemon with new Python code
lighting ext-reload      # copy extension files and reload the extension
lighting selftest        # 61 checks, browser + desktop
lighting bench           # speed and size table on the test page
claude plugin validate .
```

`HOW-TO-SETUP.md` is the runbook with everything learned while building it.

## Limits

- Windows only for now.
- Firefox is not supported (no `debugger` extension API).
- Cross-origin iframes: snapshot with `--frame host`, clicks there are DOM clicks.
- Windows blocks input to apps running as administrator unless the terminal runs as administrator too.

## License

MIT. Bundles [Defuddle](https://github.com/kepano/defuddle) (MIT, see `extension/defuddle.LICENSE.txt`).
