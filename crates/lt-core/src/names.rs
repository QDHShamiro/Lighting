use crate::{Fail, close_match};

const SYSTEM: &[&str] = &[
    "do",
    "channels",
    "ping",
    "stop",
    "status",
    "config",
    "log",
    "version",
    "help",
    "selftest",
    "bench",
    "demo",
    "board",
    "ui",
    "autoload",
    "ext-reload",
    "run",
    "routine",
    "routines",
    "record",
    "done",
    "keep",
    "keys",
    "suggest",
    "brain",
    "improve",
    "doctor-info",
    "access-prompt",
    "tokens",
    "grep",
    "squeeze",
    "secure",
];
const MEMORY: &[&str] = &["remember", "recall", "forget", "todo", "memory", "hook", "wiki", "code"];
const MEMORY_LANE: &[&str] = &["suggest", "tokens", "grep", "squeeze", "board"];
const HOOK_LANE: &[&str] = &["suggest", "hook"];
pub const SHARED: &[&str] = &["snap", "click", "type", "press", "shot", "scroll", "hover", "drag"];
pub const APP_ONLY: &[&str] = &["windows", "focus", "clip", "launch"];
pub const CHAT: &[&str] = &["inbox", "reply", "unread"];
pub const WEB_ONLY: &[&str] = &[
    "open",
    "text",
    "fill",
    "select",
    "check",
    "wait",
    "expect",
    "table",
    "fetch",
    "js",
    "dismiss",
    "search",
    "net",
    "tools",
    "call",
    "upload",
    "tabs",
    "tab",
    "close",
    "back",
    "forward",
    "reload",
    "dialog",
    "downloads",
    "console",
    "viewport",
    "frames",
];
const MORE: &[&str] = &["read", "listen", "watch", "voice", "phone", "claude", "desktop"];
const HIDDEN: &[&str] = &["hook", "suggest", "board", "doctor-info", "access-prompt", "autoload", "ping"];
const GUESS: &[(&str, &str)] = &[
    ("find", "snap -f \"word\" (page), code <name> (code) or recall \"words\" (memory)"),
    ("url", "tabs (every tab with its URL)"),
    ("goto", "open <url>"),
    ("navigate", "open <url>"),
    ("screenshot", "shot"),
    ("eval", "js <code>"),
    ("sleep", "wait <ms|\"text\">"),
];

pub const QUIET: &[&str] = &[
    "done",
    "suggest",
    "hook",
    "ping",
    "status",
    "log",
    "version",
    "help",
    "config",
    "brain",
    "improve",
    "doctor-info",
    "access-prompt",
    "tokens",
    "board",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Main,
    Memory,
    Hook,
}

pub fn lane(name: &str) -> Lane {
    let name = name.to_lowercase();
    if HOOK_LANE.contains(&name.as_str()) {
        Lane::Hook
    } else if MEMORY_LANE.contains(&name.as_str()) || MEMORY.contains(&name.as_str()) {
        Lane::Memory
    } else {
        Lane::Main
    }
}

fn all() -> impl Iterator<Item = &'static str> {
    [SYSTEM, MEMORY, SHARED, APP_ONLY, CHAT, WEB_ONLY, MORE].into_iter().flatten().copied()
}

fn known() -> Vec<&'static str> {
    let mut names: Vec<&str> = all().filter(|n| !HIDDEN.contains(n)).collect();
    names.sort_unstable();
    names.dedup();
    names
}

pub fn built_later(name: &str) -> bool {
    all().any(|n| n == name)
}

pub fn unknown(name: &str) -> Fail {
    let guess = GUESS.iter().find(|(word, _)| *word == name).map(|(_, hint)| *hint);
    let hint = guess.or_else(|| close_match(name, &known())).unwrap_or("help");
    Fail::hint(format!("unknown command '{name}'"), format!("lighting {hint}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_match_the_2x_daemon() {
        assert_eq!(lane("suggest"), Lane::Hook);
        assert_eq!(lane("HOOK"), Lane::Hook);
        assert_eq!(lane("recall"), Lane::Memory);
        assert_eq!(lane("squeeze"), Lane::Memory);
        assert_eq!(lane("snap"), Lane::Main);
        assert_eq!(lane(""), Lane::Main);
    }

    #[test]
    fn unknown_commands_get_the_2x_hints() {
        let cases = [
            ("snapp", "snap"),
            ("goto", "open <url>"),
            ("zzzz", "help"),
            ("pinng", "help"),
            ("stauts", "status"),
            ("recal", "recall"),
            ("clik", "click"),
            ("verison", "version"),
            ("lgo", "help"),
            ("snaps", "snap"),
            ("tab", "tab"),
            ("t", "help"),
        ];
        for (word, hint) in cases {
            assert_eq!(unknown(word).text(), format!("err: unknown command '{word}' -> try: lighting {hint}"));
        }
        assert!(built_later("snap") && built_later("recall") && built_later("ping") && !built_later("snapp"));
    }

    #[test]
    fn the_known_names_are_the_2x_list_secure_and_the_window() {
        let want = "back bench brain call channels check claude click clip close code config console demo desktop dialog \
                    dismiss do done downloads drag expect ext-reload fetch fill focus forget forward frames grep help \
                    hover improve inbox js keep keys launch listen log memory net open phone press read recall record \
                    reload remember reply routine routines run scroll search secure select selftest shot snap squeeze \
                    status stop tab table tabs text todo tokens tools type ui unread upload version viewport voice wait watch \
                    wiki windows";
        assert_eq!(known(), want.split(' ').collect::<Vec<_>>());
    }
}
