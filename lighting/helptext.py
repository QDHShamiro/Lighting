HELP = """lighting <command> [args]   browser + Windows desktop for Claude Code
refs: e12 web element | d5 app control | o3 screen text | w2 window | t77 tab

web     open <url> [-n]   snap [-f txt] [-s css|ref] [--all] [--diff]   text [-f txt]
        click <ref|"text"> [--yes] [--force]   fill "Label=value" ... [--submit]
        type <ref> <text> [--env VAR] [--append]   press <keys>   select <ref> <option>
        check <ref> [off]   hover <ref>   drag <ref> <ref>   scroll [ref|up|down] [--until "text"]
        wait <"text"|url:x|ms|ref> [--gone]   expect <"text"|url:x> [--gone]   table <ref|css>
        fetch <url> [--pick a.b]   read <url>   js <code>   dismiss   upload <ref> <file>
        tabs   tab <id>   close [id]   back   forward   reload   shot [ref] [--if-changed]
        dialog accept|dismiss [text]   downloads   console
desktop windows   focus <w>   snap <w> [--web]   click <d|o|"text">   type <d> <text>
        press <keys>   read [w|screen]   shot [w|screen]   clip [get|set <text>]
system  do "cmd; cmd; ..."   status   stop   config [key value]   log   version
        setup [--manual]   selftest [web|app]   bench   ext-reload   uninstall [--purge]

Output is plain text, one line per action. Big results go to a file (path printed).
Safety: banking/payment sites are read-only, risky clicks need --yes, Ctrl+Alt+End stops everything."""
