use crate::pat::{ANSI, IMPORTANT, NUMS, PREFIX, TRACE};
use crate::text::{clip, head, rstrip, strip, words};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::iter;

const HEAD: usize = 10;
const TAIL: usize = 30;
const AFTER: usize = 2;
const TRACE_LONG: usize = 12;
const TRACE_TIGHT: usize = 3;
const SAME: usize = 2;
const GAP_KEEP: usize = 2;
const LINES: usize = 120;
const LINE_CHARS: usize = 300;
const KIND_CHARS: usize = 60;

pub(crate) fn clean(text: &str) -> Vec<String> {
    let unix = text.replace("\r\n", "\n");
    let plain = ANSI.replace_all(&unix, "");
    let mut out: Vec<String> = plain
        .split('\n')
        .map(|line| {
            let line = if line.contains('\r') {
                line.split('\r').filter(|p| !strip(p).is_empty()).next_back().unwrap_or("")
            } else {
                line
            };
            rstrip(line).to_string()
        })
        .collect();
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

struct Line<'a> {
    raw: &'a str,
    tmpl: String,
    flow: bool,
    hot: bool,
}

impl<'a> Line<'a> {
    fn new(raw: &'a str) -> Self {
        let body = PREFIX.replace(raw, "");
        let flow = TRACE.is_match(&body) || body.starts_with([' ', '\t']);
        let hot = !flow && IMPORTANT.is_match(raw);
        let numbered = NUMS.replace_all(&body, "#");
        let tmpl = words(&numbered).collect::<Vec<_>>().join(" ");
        Line { raw, tmpl, flow, hot }
    }

    fn kind(&self) -> &str {
        let end = self.tmpl.match_indices(' ').nth(2).map_or(self.tmpl.len(), |(i, _)| i);
        head(&self.tmpl[..end], KIND_CHARS)
    }
}

fn marked(lines: &[Line], after: usize, trace: usize) -> Vec<bool> {
    let n = lines.len();
    let mut keep: Vec<bool> = (0..n).map(|i| i < HEAD || i + TAIL >= n).collect();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.hot {
            continue;
        }
        let count = seen.entry(line.tmpl.as_str()).or_insert(0);
        *count += 1;
        if *count > SAME {
            continue;
        }
        keep[i] = true;
        let mut j = i + 1;
        while j < n && j - i <= trace && lines[j].flow {
            keep[j] = true;
            j += 1;
        }
        for k in keep.iter_mut().take(n.min(j + after)).skip(j) {
            *k = true;
        }
    }
    keep
}

fn gap(run: &[&Line]) -> String {
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for line in run.iter().filter(|l| !strip(l.raw).is_empty()) {
        let kind = line.kind();
        match at.get(kind) {
            Some(&i) => kinds[i].1 += 1,
            None => {
                at.insert(kind, kinds.len());
                kinds.push((kind, 1));
            }
        }
    }
    kinds.sort_by_key(|k| Reverse(k.1));
    let what: Vec<String> =
        kinds.iter().take(2).filter(|(k, c)| !k.is_empty() && *c > 1).map(|(k, c)| format!("{c} x {k}")).collect();
    if what.is_empty() {
        format!("... {} lines", run.len())
    } else {
        format!("... {} lines ({})", run.len(), what.join(", "))
    }
}

fn assemble(lines: &[Line], keep: &[bool]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<&Line> = Vec::new();
    let mut prev: Option<&str> = None;
    let mut same = 1;
    let items = lines.iter().zip(keep).map(|(l, &k)| (Some(l), k)).chain(iter::once((None, true)));
    for (line, kept) in items {
        if let Some(l) = line
            && !kept
        {
            run.push(l);
            continue;
        }
        let tmpl = line.map(|l| l.tmpl.as_str());
        if let Some(t) = tmpl
            && run.is_empty()
            && !t.is_empty()
            && prev == Some(t)
        {
            same += 1;
            continue;
        }
        if same > 1
            && let Some(last) = out.last_mut()
        {
            last.push_str(&format!(" (x{same})"));
        }
        same = 1;
        if run.len() > GAP_KEEP {
            out.push(gap(&run));
        } else {
            out.extend(run.iter().map(|l| clip(l.raw, LINE_CHARS)));
        }
        run.clear();
        if let Some(l) = line {
            out.push(clip(l.raw, LINE_CHARS));
        }
        prev = tmpl;
    }
    out
}

pub(crate) fn squeeze(raw: &[String]) -> Vec<String> {
    let lines: Vec<Line> = raw.iter().map(|r| Line::new(r)).collect();
    let mut out = Vec::new();
    for (after, trace) in [(AFTER, TRACE_LONG), (0, TRACE_TIGHT), (0, 0)] {
        out = assemble(&lines, &marked(&lines, after, trace));
        if out.len() <= LINES {
            return out;
        }
    }
    let top = LINES - TAIL;
    let mut cut = out[..top].to_vec();
    cut.push(format!("... {} more squeezed lines (see the full file)", out.len() - LINES));
    cut.extend_from_slice(&out[out.len() - TAIL..]);
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_drops_colors_carriage_returns_and_trailing_blanks() {
        let got = clean("\x1b[32mok\x1b[0m\r\n10%\r50%\r100%\r\n  \r \r\nend  \n\n \n");
        assert_eq!(got, ["ok", "100%", "", "end"]);
        assert!(clean("").is_empty());
    }

    #[test]
    fn kind_is_the_first_three_template_words() {
        let line = Line::new("[12:00:01 INFO]: Player 42 joined from 10.0.0.1 port 5");
        assert_eq!(line.tmpl, "Player # joined from #.#.#.# port #");
        assert_eq!(line.kind(), "Player # joined");
        assert!(Line::new("\tat a.B(B.java:1)").flow);
        assert!(!Line::new("\tat a.B(B.java:1) error").hot);
    }

    #[test]
    fn repeated_kept_lines_collapse() {
        let raw: Vec<String> = (0..100).map(|i| format!("tick {i}")).collect();
        let out = squeeze(&raw);
        assert_eq!(out, ["tick 0 (x10)", "... 60 lines (60 x tick #)", "tick 70 (x30)"]);
    }
}
