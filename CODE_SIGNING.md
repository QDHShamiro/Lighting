# Code signing policy

Windows releases of Wisply are meant to be signed through the SignPath Foundation. The application is pending; until
it is approved the Windows programs are unsigned and this page says so. Once approved:

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

## What gets signed

- `wisply.exe`, `wisply-ui.exe` and `wisply-task.exe` in `wisply-<version>-windows-x64.zip`, and the single
  `wisply.exe` on the [Releases page](https://github.com/QDHShamiro/Wisply/releases).
- Only files built by the public release workflow (`.github/workflows/release.yml`) from a version tag of this
  repository go to SignPath. Nothing built on a personal computer is signed.
- The signing key stays with SignPath. Every release needs a manual approval before it is signed.
- The macOS and Linux programs are not signed. Every file of a release, on every system, is listed with its SHA-256
  in `SHA256SUMS.txt` and carries a GitHub build attestation (`gh attestation verify <file> -R QDHShamiro/Wisply`).

## Team roles

| Role | Members |
|---|---|
| Committers and reviewers | [QDHShamiro](https://github.com/QDHShamiro) |
| Approvers | [QDHShamiro](https://github.com/QDHShamiro) |

Changes from anyone else come in as pull requests and are reviewed before they are merged. All members use
multi-factor authentication for GitHub and SignPath.

## Privacy policy

Wisply has no telemetry and no account. It works on your computer and sends data only where you send it:

- Pages you open go to their sites through your own browser, as if you opened them yourself.
- With your own API key only: questions about videos, images and sound go to Google Gemini, speech to text goes to
  Groq or OpenAI, and the session notes of the local memory can be written by Gemini. `wisply config gemini false`
  and `wisply config memory_learn false` turn this off; without a key nothing is sent.
- `wisply secure` sends the names and versions of a project's dependencies to [osv.dev](https://osv.dev) to look up
  known vulnerabilities.
- `read` and `fetch` of GitHub links ask `api.github.com`. `wisply doctor --fix` downloads yt-dlp and ffmpeg, and the
  code map downloads language parsers, from their GitHub releases, each checked against its published SHA-256.

Downloads: only from the [Releases page](https://github.com/QDHShamiro/Wisply/releases).
