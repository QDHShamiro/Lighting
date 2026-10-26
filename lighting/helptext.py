HELP = """lighting <command> [args]   browser + Windows desktop for AI agents
refs: e12 web element | d5 app control | o3 screen text | w2 window | t77 tab

web     open <url> [-n] [-f "a|b"] [--text]   snap [-f "a|b"] [-s css|ref] [--all] [--full] [--diff] [--links]   text [-f txt] [--links]
        click <ref|"text"|#hint> [--yes] [--force]   fill "Label=value" ... [--submit]   search [site] "words"
        type <ref> <text> [--env VAR] [--append]   press <keys>   select <ref> <option>
        check <ref> [off]   hover <ref>   drag <ref> <ref>   scroll [ref|up|down] [--until "text"]
        wait <"text"|url:x|ms|ref> [--gone] [--reload 15s --timeout 10m]   expect <"text"|url:x> [--gone]
        fetch <url> [--pick a.b]   read <url|file.pdf> [url ...] [--links]   js <code|--file f>   dismiss
        upload <ref> <file>   tabs [-f x]   tab <id>   close [id]   back   forward   reload   table <ref|css>
        shot [ref] [--marks] [--if-changed] [--threshold 2 (% moved)]   viewport <WxH|reset>   snap --media (images too)
        frames <video ref> [--count 6] [--scenes] [--small] [--from m:ss --to m:ss] [--live --every ms]
        read <image url|file> (saved as JPEG to look at)
        dialog accept|dismiss [text]   downloads   console   net [n] [-f x] (JSON the page loaded)
        tools   call <tool> '{json}' [--yes] (WebMCP)   open <app link> (obsidian://, spotify:) starts the app
video   watch <url|file|ref> [--from m:ss --to m:ss] [--at 1:23,2:45] [--text] [--full] [--count 12] [--lang de]
sound   listen [ref|url|file] [--from m:ss --to m:ss] [--lang de] [--live] [--local]   frames <ref> --audio
voice   voice join "Call" | here | leave | say "text" | status | watch on|off | set key=value | persona "Call"  (Windows)
desktop windows [-f x] (one match = chosen)   launch <app|uri|path> [-f x]   focus <w>   close <w>   snap <w> [--web]
        click <d|o|"text">   type <d|"Field"|focused> <text> [--tab] [--enter]   press <keys>   read [w|screen]
        wait "text" [w] [--gone] [--timeout 60s] (in an app)
        shot [w|screen] [--seconds 10] (video)   clip [get|set <text>]   press playpause|volumeup (no window)
        claude ["prompt" --yes] [--dir path] [--name x] [--model m] [--effort e] [--no-remote] [--bg]  new Claude tab with remote control (--bg: no window)
        claude list [--all] | claude logs <id>   background Claude sessions and their last lines
phone   phone devices|setup   phone snap [-f x]   phone tap <p3|"text"> [--long]   phone type ["text"] [--enter]
        phone key back|home|recent|enter   phone swipe up|down|left|right   phone shot   phone launch <app>   phone open <url>
chat    inbox <w|"title"> [--from name] [--since id] [--timeout 5m]   reply <w|"title"> "text" [--wait] [--yes]
        unread [w|"title"] (unread DMs and mentions)
keys    keys [app|site] [-f x]   keys add <app> "ctrl+k=what"   keys rm <app> <key>
brain   brain (what Lighting learned)   brain forget <site|app:x.exe>   brain reset --yes   improve [days] (self-check)
        known pages show the controls you use there; snap --full shows all   config brain off
memory  remember "fact" [--kind decision] [--global]   recall "words" [--all] | recall <id>   forget <id>
        todo [--all] [--list x]   todo add "task" [--list x]   todo done|doing|drop <id>   todo note <id> "text"
        memory (state)   memory learn   memory model (local AI, downloads itself)   memory import claude-mem   memory off
        memory import context-engine   wiki (drafts) | wiki accept <n> [folder] | wiki drop <n>   config vault <folder>
code    code (overview)   code <Name> (definition + callers)   code callers <Name>   code "what it does" (by meaning)
        code outline <file> (signatures)   code body <Name|Class.name|file.name>   code impact [--base main] [--tests]
        grep <pattern> [path ...] [-i] [-w] [-F] [-l] [-C 2] [--glob "*.py"] [--all] (grouped by file, long lines cut at the hit)
        squeeze <file> | <cmd> 2>&1 | lighting squeeze | squeeze -- <cmd> (long output: errors + the end, repeats folded, all of it in a file)
routine routines [-f x]   run <name|words> [param=value ...] [--yes]   routine show|save|rm|rename|forget <name>
        record start [name] [--web|--desktop]   record stop [name] [param=value ...]   record cancel|status
        routine schedule <name> daily 08:00|weekly mon 09:30|hourly|every 30m [param=value]   routine schedules|unschedule
cleanup done [--quiet]   keep [t|w]   config cleanup on|off   config window on|off (own window per session)   config focus never|away|idle (take the front: never = default)
system  do|batch "cmd; cmd; ..." [--keep-going] [--quiet] (?cmd = optional, one per line too)   status   stop   config [key value]   log [stats]   version
        tokens [days|today|all] (used, without Lighting, saved: overview, also /lighting:tokens)   tokens claude [days] (where your Claude Code tokens go, biggest fixes first)
        setup [--manual]   install <ai>   selftest [web|app|chat]   bench [--real [--runs 3]]   demo [--fast]   ext-reload   uninstall [--purge]
        doctor [--fix] [--offline]   checks every part of Lighting, --fix repairs what it safely can

Any action takes -f "a|b" to show the matching lines of the result in the same call.
Output is plain text, one line per action. Big results go to a file (path printed).
Tasks that repeat become routines by themselves (config learn off to stop learning).
Tabs and apps Lighting opened close on done (Claude Code runs it after every reply); keep hands one to the user.
Safety: banking/payment sites are read-only, risky clicks, links in chats and long URL data to new sites need --yes,
Ctrl+Alt+End stops everything."""
