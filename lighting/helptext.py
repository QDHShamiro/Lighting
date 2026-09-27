HELP = """lighting <command> [args]   browser + Windows desktop for AI agents
refs: e12 web element | d5 app control | o3 screen text | w2 window | t77 tab

web     open <url> [-n] [-f "a|b"] [--text]   snap [-f "a|b"] [-s css|ref] [--all] [--diff]   text [-f txt] [--links]
        click <ref|"text"|#hint> [--yes] [--force]   fill "Label=value" ... [--submit]   search [site] "words"
        type <ref> <text> [--env VAR] [--append]   press <keys>   select <ref> <option>
        check <ref> [off]   hover <ref>   drag <ref> <ref>   scroll [ref|up|down] [--until "text"]
        wait <"text"|url:x|ms|ref> [--gone] [--reload 15s --timeout 10m]   expect <"text"|url:x> [--gone]
        fetch <url> [--pick a.b]   read <url|file.pdf> [url ...] [--links]   js <code|--file f>   dismiss
        upload <ref> <file>   tabs   tab <id>   close [id]   back   forward   reload   table <ref|css>
        shot [ref] [--marks] [--if-changed]   viewport <WxH|reset>   snap --media (images too)
        frames <video ref> [--count 6] [--scenes] [--from m:ss --to m:ss] [--live --every ms]
        read <image url|file> (saved as JPEG to look at)
        dialog accept|dismiss [text]   downloads   console   net [n] [-f x] (JSON the page loaded)
        tools   call <tool> '{json}' [--yes] (WebMCP)   open <app link> (obsidian://, spotify:) starts the app
sound   listen [ref|url|file] [--from m:ss --to m:ss] [--lang de] [--live] [--local]   frames <ref> --audio
desktop windows [-f x]   launch <app|uri|path> [-f x]   focus <w>   close <w>   snap <w> [--web]
        click <d|o|"text">   type <d|"Field"|focused> <text> [--tab] [--enter]   press <keys>   read [w|screen]
        shot [w|screen] [--seconds 10] (video)   clip [get|set <text>]   press playpause|volumeup (no window)
        claude ["prompt" --yes] [--dir path] [--name x] [--no-remote]  new Claude tab with remote control
chat    inbox <w|"title"> [--from name] [--since id] [--timeout 5m]   reply <w|"title"> "text" [--wait] [--yes]
        unread [w|"title"] (unread DMs and mentions)
keys    keys [app|site] [-f x]   keys add <app> "ctrl+k=what"   keys rm <app> <key>
routine routines [-f x]   run <name|words> [param=value ...] [--yes]   routine show|save|rm|rename|forget <name>
        record start [name] [--web|--desktop]   record stop [name] [param=value ...]   record cancel|status
cleanup done [--quiet]   keep [t|w]   config cleanup on|off   config window on|off (own window per session)
system  do "cmd; cmd; ..."   status   stop   config [key value]   log [stats]   version
        setup [--manual]   install <ai>   selftest [web|app|chat]   bench [--real]   ext-reload   uninstall [--purge]

Any action takes -f "a|b" to show the matching lines of the result in the same call.
Output is plain text, one line per action. Big results go to a file (path printed).
Tasks that repeat become routines by themselves (config learn off to stop learning).
Tabs and apps Lighting opened close on done (Claude Code runs it after every reply); keep hands one to the user.
Safety: banking/payment sites are read-only, risky clicks, links in chats and long URL data to new sites need --yes,
Ctrl+Alt+End stops everything."""
