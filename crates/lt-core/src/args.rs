use std::collections::BTreeMap;

const VALUED: &[&str] = &[
    "f",
    "s",
    "d",
    "max",
    "browser",
    "lang",
    "timeout",
    "width",
    "until",
    "pick",
    "frame",
    "body",
    "method",
    "button",
    "region",
    "delta",
    "file",
    "role",
    "last",
    "on",
    "count",
    "every",
    "from",
    "to",
    "reload",
    "since",
    "seconds",
    "dir",
    "name",
    "model",
    "effort",
    "threshold",
    "runs",
    "kind",
    "list",
    "project",
    "base",
    "glob",
    "context",
    "include",
    "exclude",
    "exclude-dir",
    "max-count",
    "out",
    "ask",
    "at",
    "fps",
    "title",
    "mtime",
    "task",
    "words",
];
const MULTI: &[&str] = &["glob", "include", "exclude", "exclude-dir"];
const ALIAS: &[(&str, &str)] = &[
    ("-f", "f"),
    ("-s", "s"),
    ("-d", "d"),
    ("-n", "new"),
    ("-a", "all"),
    ("-y", "yes"),
    ("-e", "errors"),
    ("-g", "gone"),
    ("-m", "media"),
    ("-o", "out"),
];
const SAME: &[(&str, &str)] = &[("batch", "do"), ("video", "watch"), ("sound", "listen")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flag {
    On,
    Val(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    pub pos: Vec<String>,
    pub flags: BTreeMap<String, Flag>,
}

impl Call {
    pub fn on(&self, key: &str) -> bool {
        match self.flags.get(key) {
            Some(Flag::On) => true,
            Some(Flag::Val(v)) => !v.is_empty(),
            None => false,
        }
    }

    pub fn val(&self, key: &str) -> Option<&str> {
        match self.flags.get(key) {
            Some(Flag::Val(v)) => Some(v),
            _ => None,
        }
    }

    fn put(&mut self, key: &str, val: &str) {
        match self.flags.get_mut(key) {
            Some(Flag::Val(old)) if MULTI.contains(&key) => {
                old.push(',');
                old.push_str(val);
            }
            _ => {
                self.flags.insert(key.to_string(), Flag::Val(val.to_string()));
            }
        }
    }
}

pub fn parse(argv: &[String]) -> Call {
    let raw = argv.first().map(|a| a.to_lowercase()).unwrap_or_default();
    let name = SAME.iter().find(|(from, _)| *from == raw).map_or(raw, |(_, to)| (*to).to_string());
    let mut call = Call { name, ..Call::default() };
    let mut i = 1;
    while i < argv.len() {
        let a = argv[i].as_str();
        let mut key = None;
        if let Some(rest) = a.strip_prefix("--").filter(|r| !r.is_empty()) {
            if let Some((k, v)) = rest.split_once('=') {
                call.put(k, v);
                i += 1;
                continue;
            }
            key = Some(rest);
        } else if call.name != "grep" {
            key = ALIAS.iter().find(|(short, _)| *short == a).map(|(_, long)| *long);
        }
        match key {
            None => call.pos.push(a.to_string()),
            Some(k) if VALUED.contains(&k) && i + 1 < argv.len() && !argv[i + 1].starts_with("--") => {
                let val = argv[i + 1].clone();
                call.put(k, &val);
                i += 1;
            }
            Some(k) => {
                call.flags.insert(k.to_string(), Flag::On);
            }
        }
        i += 1;
    }
    call
}

pub fn wants_help(argv: &[String]) -> Option<String> {
    let first = argv.first().map(String::as_str)?;
    if ["help", "-h", "--help"].contains(&first) {
        return Some(argv.get(1).cloned().unwrap_or_default());
    }
    let rest = &argv[1..];
    let own = rest.iter().position(|a| a == "--").map_or(rest, |end| &rest[..end]);
    (own.iter().any(|a| a == "--help") || own == ["-h"]).then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    fn val(v: &str) -> Flag {
        Flag::Val(v.to_string())
    }

    #[test]
    fn parses_like_the_2x_daemon() {
        let call = parse(&argv(&["Snap", "-f", "save|draft", "--max", "5", "e3", "--new", "-n"]));
        assert_eq!(call.name, "snap");
        assert_eq!(call.pos, ["e3"]);
        assert_eq!(call.flags.get("f"), Some(&val("save|draft")));
        assert_eq!(call.flags.get("max"), Some(&val("5")));
        assert_eq!(call.flags.get("new"), Some(&Flag::On));
        let call = parse(&argv(&["grep", "-f", "x", "--glob", "*.rs", "--glob=*.py", "--max", "--all"]));
        assert_eq!(call.pos, ["-f", "x"]);
        assert_eq!(call.flags.get("glob"), Some(&val("*.rs,*.py")));
        assert_eq!(call.flags.get("max"), Some(&Flag::On));
        assert!(call.on("all") && !call.on("none"));
        assert_eq!(parse(&argv(&["video", "--", "--x=1"])).name, "watch");
        let call = parse(&argv(&["log", "--", "--=v", "-f"]));
        assert_eq!(call.pos, ["--"]);
        assert_eq!(call.flags.get(""), Some(&val("v")));
        assert_eq!(call.flags.get("f"), Some(&Flag::On));
        let call = parse(&argv(&["board", "save", "a.md", "--mtime", "17", "--title", "x y", "--task", "z"]));
        assert_eq!(call.pos, ["save", "a.md"]);
        assert_eq!([call.val("mtime"), call.val("title"), call.val("task")], [Some("17"), Some("x y"), Some("z")]);
        assert_eq!(parse(&[]), Call::default());
    }

    #[test]
    fn help_is_wanted_like_in_2x() {
        assert_eq!(wants_help(&argv(&["help", "snap"])).as_deref(), Some("snap"));
        assert_eq!(wants_help(&argv(&["--help"])).as_deref(), Some(""));
        assert_eq!(wants_help(&argv(&["snap", "--help"])).as_deref(), Some("snap"));
        assert_eq!(wants_help(&argv(&["snap", "-h"])).as_deref(), Some("snap"));
        assert_eq!(wants_help(&argv(&["snap", "-h", "x"])), None);
        assert_eq!(wants_help(&argv(&["js", "--", "--help"])), None);
        assert_eq!(wants_help(&[]), None);
    }
}
