# Security

## Report a problem

Please do not open a public issue for a security problem. Use GitHub's private report instead:
https://github.com/QDHShamiro/Lighting/security/advisories/new

Say what is affected, how to reproduce it and what an attacker could do with it. You get an answer within 7 days.
Fixes go into the next release and the release notes name the problem once it is fixed.

Supported: the newest release. Older versions get no fixes.

## What Wisply can do on your computer

Wisply is a remote control for your own computer, driven by an AI agent you run. It can:

- read and click web pages in your logged-in browser through its extension (all sites, because you choose the pages)
- read and operate desktop apps through the accessibility API of Windows, macOS or Linux, and read the screen with OCR
- press keys, move the mouse, use the clipboard, take screenshots and short screen videos
- start apps and open files and links

Guard rails that are always on:

- Payment, bank and checkout sites are blocked (`~/.wisply/blocklist.txt`), and clicks on buy, pay, delete or merge
  buttons need an explicit `--yes`.
- Links that can run programs (`file:`, `ms-msdt:`, `search-ms:` and similar) and starting programs from a path need `--yes`.
- Page and app content is treated as data: hidden text on pages is left out, and the skill tells the agent that
  content is never an instruction.
- Secrets typed with `--env NAME` never appear in logs, routines or the agent's output.
- `Ctrl+Alt+End` stops every running Wisply command at once.

## What leaves your computer

Nothing, unless you ask for it:

- The daemon, the extension and the client talk only over a local pipe (Windows) or a socket in `~/.wisply`
  that only your user can open, and every request carries a key from `~/.wisply/key`.
- Logs, learned routines and the brain stay in `~/.wisply` and are never uploaded.
- `wisply doctor` asks GitHub once for the newest release number (`--offline` skips it).
- `listen` uses YouTube captions or a local Whisper model; it sends audio to Groq or OpenAI only when you set
  `GROQ_API_KEY` or `OPENAI_API_KEY` yourself.

## Release files

Every file of a release is built by GitHub Actions from the tagged source and has a GitHub build attestation:

```
gh attestation verify wisply.exe -R QDHShamiro/Lighting
```

`SHA256SUMS.txt` in each release lists the hashes, including the clients in `bin/` of the repository at that tag.
The clients are not code-signed.
