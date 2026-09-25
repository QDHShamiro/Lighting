# Lighting on the Windows desktop

## Find the window
- `lighting windows` lists top-level windows: `w3 Discord - Discord.exe *` (`*` = in front, `(min)` = minimized).
- Target a window by ref (`snap w3`) or by name (`snap app:Discord`, matches title or exe). Later commands without a ref use that window.

## Read it
- `lighting snap w3` gives the controls from UI Automation: `d5 button "Save"`, `d6 edit "Search" ="abc"`, `d7 checkbox "Wrap" [x]`, `d8 tab "Home" *`, `(disabled)`, `(collapsed)`.
- `--text` also lists static text (labels, status lines). `-f word` filters the output.
- Browser windows hide the web page part (use the web commands for the page); `--web` includes it.
- Apps without accessible controls (games, canvas UIs): `lighting read w3` runs Windows OCR and prints `o4 "Play" @960,420`. `--lang de` or `--lang en` picks the OCR language (default English, switch with `lighting config ocr_lang de`).
- `lighting shot w3` saves a small JPEG of just that window. Read it only if you really need pixels.

## Act
- `click d5` uses the control's own action (Invoke, Toggle, Select, Expand) in the background. The user's mouse and focus stay where they are.
- `type d6 hello` sets the value in the background. Without a value pattern it pastes via the clipboard (the window comes to the front for a moment, the clipboard is restored).
- `press ctrl+s` brings the window to the front for the key press, then gives focus back. `--stay` keeps the window in front. `--game` sends scan codes (DirectInput games).
- `click o4` or `click "Play"` (OCR text) does a real mouse click: it waits until the user has not moved the mouse for 0.3 s, clicks and puts the cursor back exactly.
- `scroll d9` / `scroll up` scroll a list or the window. `drag d3 d9` drags with the real mouse (cursor restored).
- An orange Claude pointer shows where Lighting acts. Turn it off with `lighting config pointer off`.

## Limits
- Windows that run as administrator cannot be controlled from a normal terminal (Windows UIPI). Lighting says so.
- Some apps expose few controls (older games, custom-drawn UIs): use `read` + `click o<n>`.
- The Windows 11 Notepad answers UI Automation slowly (~0.8 s per snapshot); most apps take 20-200 ms.
