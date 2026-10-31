use regex::Regex;
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("squeeze pattern")
}

pub(crate) static ANSI: LazyLock<Regex> =
    LazyLock::new(|| re(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Z0-9]"));

pub(crate) static IMPORTANT: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?i)\w*(?:error|exception)s?\b|\b(?:err|fail|failed|failing|failure|failures|fatal|traceback|panic|",
        r"warn|warning|warnings|denied|refused|cannot|can't|unable|not found|no such|missing|undefined|",
        r"segfault|abort|aborted|timeout|timed out|killed|assert|assertion|severe|critical)\b",
    ))
});

pub(crate) static TRACE: LazyLock<Regex> = LazyLock::new(|| re(r#"^\s+at \S|^\s*File "|^\s+\.\.\. \d+ more|^\s+\^"#));

pub(crate) static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?Z? (?:[A-Z]{3,8} )?)?(?:\[[^\]]{0,60}\]:? ?){0,3}")
});

pub(crate) static NUMS: LazyLock<Regex> = LazyLock::new(|| re(r"0x[0-9a-fA-F]+|\b[0-9a-fA-F]{8,}\b|\d+"));

pub(crate) static QUIET: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)"[\w-]*(?:error|fail|exception|warn)[\w-]*"\s*:\s*(?:null|false|0|\[\]|\{\}|"")"#));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_like_the_python_ones() {
        assert_eq!(ANSI.replace_all("\x1b[1;31mred\x1b[0m \x1b]0;t\x07x\x1b(B", ""), "red x");
        assert!(IMPORTANT.is_match("java.lang.NullPointerException: x"));
        assert!(IMPORTANT.is_match("Can't keep up"));
        assert!(IMPORTANT.is_match("upstream TIMED OUT"));
        assert!(!IMPORTANT.is_match("terrific errand"));
        assert!(TRACE.is_match("\tat a.b.C(C.java:1)"));
        assert!(TRACE.is_match("  File \"x.py\", line 1"));
        assert_eq!(PREFIX.replace("2026-10-30T04:00:00.819Z INFO [main] [db]: query", ""), "query");
        assert_eq!(PREFIX.replace("[12:00:01 INFO]: joined", ""), "joined");
        assert_eq!(NUMS.replace_all("id 0x1F deadbeef12 v1.2", "#"), "id # # v#.#");
        assert_eq!(QUIET.replace_all(r#"{"error": null, "lastWarning": "", "x": 1}"#, ""), r#"{, , "x": 1}"#);
    }
}
