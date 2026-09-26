HELP = """lighting <command> [args]   browser + Windows desktop for AI agents
refs: e12 web element | d5 app control | o3 screen text | w2 window | t77 tab

web     open <url> [-n]   snap [-f "a|b"] [-s css|ref] [--all] [--diff]   text [-f txt] [--links]
        click <ref|"text"|#hint> [--yes] [--force]   fill "Label=value" ... [--submit]
        type <ref> <text> [--env VAR] [--append]   press <keys>   select <ref> <option>
        check <ref> [off]   hover <ref>   drag <ref> <ref>   scroll [ref|up|down] [--until "text"]
        wait <"text"|url:x|ms|ref> [--gone]   expect <"text"|url:x> [--gone]   table <ref|css>
        fetch <url> [--pick a.b]   read <url|file.pdf> [url ...] [--links]   js <code|--file f>   dismiss
        upload <ref> <file>   tabs   tab <id>   close [id]   back   forward   reload
        shot [ref] [--marks] [--if-changed]   viewport <WxH|reset>   snap --media (images too)
        frames <video ref> [--count 6] [--every 1500]   read <image url|file> (saved as JPEG to look at)
        dialog accept|dismiss [text]   downloads   console
desktop windows [-f x]   launch <app|uri|path>   focus <w>   close <w>   snap <w> [--web]
        click <d|o|"text">   type <d|"Field"|focused> <text>   press <keys>   read [w|screen]
        shot [w|screen]   clip [get|set <text>]
routine routines [-f x]   run <name> [param=value ...] [--yes]   routine show|save|rm|rename|forget <name>
        record start [name] [--web|--desktop]   record stop [name] [param=value ...]   record cancel|status
cleanup done [--quiet]   keep [t|w]   config cleanup on|off
system  do "cmd; cmd; ..."   status   stop   config [key value]   log   version
        setup [--manual]   install <ai>   selftest [web|app]   bench [--real]   ext-reload   uninstall [--purge]

Output is plain text, one line per action. Big results go to a file (path printed).
Tasks that repeat become routines by themselves (config learn off to stop learning).
Tabs and apps Lighting opened close on done (Claude Code runs it after every reply); keep hands one to the user.
Safety: banking/payment sites are read-only, risky clicks and long URL data to new sites need --yes,
Ctrl+Alt+End stops everything."""
