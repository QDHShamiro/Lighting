use std::fs;
use std::path::Path;
use std::process::ExitCode;

use lt_core::Fail;

const MAX_LINES: usize = 2000;
const MAX_CHARS: usize = 25_000;

pub fn run(home: &Path, args: &[String]) -> ExitCode {
    match read(home, args) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(fail) => {
            println!("{}", fail.text());
            ExitCode::from(1)
        }
    }
}

fn read(home: &Path, args: &[String]) -> Result<String, Fail> {
    let (mut name, mut lines, mut grep) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lines" => lines = it.next().cloned(),
            "--grep" => grep = it.next().cloned(),
            _ if a.starts_with("--lines=") => lines = Some(a["--lines=".len()..].to_string()),
            _ if a.starts_with("--grep=") => grep = Some(a["--grep=".len()..].to_string()),
            _ if name.is_none() => name = Some(a.clone()),
            _ => {}
        }
    }
    let usage = || {
        Fail::hint("more needs the ref from a squeezed output, like hr-0123456789", "lighting more <ref> --lines 1-80")
    };
    let id = name.as_deref().and_then(find_ref).ok_or_else(usage)?;
    let file = home.join("headroom").join("out").join(format!("{id}.txt"));
    let text = fs::read_to_string(&file)
        .map_err(|_| Fail::hint(format!("{id} is not kept any more (kept for 7 days)"), "run the command again"))?;
    let all: Vec<&str> = text.lines().collect();
    let (from, to) = match lines.as_deref() {
        Some(range) => span(range, all.len())?,
        None => (0, all.len()),
    };
    let terms: Vec<String> = grep.as_deref().map(terms).unwrap_or_default();
    let picked: Vec<String> = if terms.is_empty() {
        all[from..to].iter().map(|l| (*l).to_string()).collect()
    } else {
        (from..to)
            .filter(|&i| {
                let line = all[i].to_lowercase();
                terms.iter().any(|t| line.contains(t.as_str()))
            })
            .map(|i| format!("{}: {}", i + 1, all[i]))
            .collect()
    };
    if picked.is_empty() {
        let what = grep.unwrap_or_default();
        return Ok(format!("no line in {id} has \"{what}\" ({} lines searched)", to - from));
    }
    let (mut out, mut shown, mut chars) = (String::new(), 0, 0);
    for line in &picked {
        chars += line.chars().count() + 1;
        if shown == MAX_LINES || (shown > 0 && chars > MAX_CHARS) {
            break;
        }
        if shown > 0 {
            out.push('\n');
        }
        out.push_str(line);
        shown += 1;
    }
    if shown < picked.len() {
        let rest = picked.len() - shown;
        let next = if terms.is_empty() {
            let at = from + shown + 1;
            format!("lighting more {id} --lines {at}-{}", (at + MAX_LINES - 1).min(all.len()))
        } else {
            "a narrower --grep or --lines a-b".to_string()
        };
        out.push_str(&format!("\n... +{rest} more lines, next: {next}"));
    }
    Ok(out)
}

fn find_ref(text: &str) -> Option<String> {
    let hex = |s: &str| s.len() == 10 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if hex(text) {
        return Some(format!("hr-{text}"));
    }
    text.match_indices("hr-").map(|(at, _)| &text[at + 3..]).find_map(|rest| {
        let id = rest.get(..10)?;
        hex(id).then(|| format!("hr-{id}"))
    })
}

fn span(range: &str, len: usize) -> Result<(usize, usize), Fail> {
    let bad = || Fail::hint(format!("cannot read the lines '{range}'"), "--lines 20-80, --lines 300- or --lines 42");
    let number = |s: &str| s.trim().parse::<usize>().ok().filter(|n| *n > 0);
    let (a, b) = match range.split_once('-') {
        Some((a, "")) => (number(a).ok_or_else(bad)?, len),
        Some((a, b)) => (number(a).ok_or_else(bad)?, number(b).ok_or_else(bad)?),
        None => {
            let n = number(range).ok_or_else(bad)?;
            (n, n)
        }
    };
    if a > len {
        return Err(Fail::new(format!("the output has only {len} lines")));
    }
    if b < a {
        return Err(bad());
    }
    Ok((a - 1, b.min(len)))
}

fn terms(words: &str) -> Vec<String> {
    words.split('|').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str, lines: usize) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("lt-more-{tag}-{}", std::process::id()));
        let out = home.join("headroom").join("out");
        fs::create_dir_all(&out).unwrap();
        let text: Vec<String> = (1..=lines).map(|i| format!("line {i}")).collect();
        fs::write(out.join("hr-0123456789.txt"), text.join("\n")).unwrap();
        home
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn reads_parts_of_a_kept_output() {
        let h = home("parts", 30);
        assert_eq!(read(&h, &args(&["hr-0123456789", "--lines", "3-4"])).unwrap(), "line 3\nline 4");
        assert_eq!(read(&h, &args(&["0123456789", "--lines=29-"])).unwrap(), "line 29\nline 30");
        assert_eq!(read(&h, &args(&["~/.lighting/headroom/out/hr-0123456789.txt", "--lines", "7"])).unwrap(), "line 7");
        assert_eq!(read(&h, &args(&["hr-0123456789", "--grep", "LINE 3|nothing"])).unwrap(), "3: line 3\n30: line 30");
        assert_eq!(
            read(&h, &args(&["hr-0123456789", "--grep", "line 1", "--lines", "9-11"])).unwrap(),
            "10: line 10\n11: line 11"
        );
        assert_eq!(read(&h, &args(&["hr-0123456789"])).unwrap().lines().count(), 30);
        assert!(read(&h, &args(&["hr-0123456789", "--grep", "zzz"])).unwrap().starts_with("no line in hr-0123456789"));
        let fail = |a: &[&str]| read(&h, &args(a)).unwrap_err().text();
        assert!(fail(&["hr-9999999999"]).starts_with("err: hr-9999999999 is not kept any more"));
        assert!(fail(&[]).starts_with("err: more needs the ref"));
        assert!(fail(&["hr-01234"]).starts_with("err: more needs the ref"));
        assert_eq!(fail(&["hr-0123456789", "--lines", "40-50"]), "err: the output has only 30 lines");
        assert!(fail(&["hr-0123456789", "--lines", "9-3"]).starts_with("err: cannot read the lines '9-3'"));
        assert!(fail(&["hr-0123456789", "--lines", "x"]).starts_with("err: cannot read the lines 'x'"));
        fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn long_outputs_point_to_the_next_part() {
        let h = home("long", 2500);
        let out = read(&h, &args(&["hr-0123456789"])).unwrap();
        let last = out.lines().last().unwrap();
        assert_eq!(out.lines().count(), 2001);
        assert_eq!(last, "... +500 more lines, next: lighting more hr-0123456789 --lines 2001-2500");
        fs::remove_dir_all(&h).unwrap();
    }
}
