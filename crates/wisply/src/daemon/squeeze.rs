use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use wisply_core::{Call, Fail, out_file};
use wisply_squeeze::{clean, commas, decode, read_limited, squeeze};

use super::ctx::Ctx;
use super::{Daemon, Out, Pending, web};

pub const USAGE: &str = "wisply squeeze <file> | <command> 2>&1 | wisply squeeze | wisply squeeze -- <command>";
pub const MAX_BYTES: usize = 64_000_000;
const INLINE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(250);
const BUDGET: Duration = Duration::from_secs(90);

struct Done {
    text: String,
    cut: usize,
}

fn squeezed(out_dir: &Path, text: &str, name: &str) -> Done {
    let Some(s) = squeeze(text) else { return Done { text: clean(text), cut: 0 } };
    let path = out_file(out_dir, &format!("squeeze-{name}"), "txt");
    let tail = format!(
        "\nsqueezed {} lines (~{} tokens) to {} lines (~{} tokens), all of it: {}",
        commas(s.lines_in),
        commas(s.chars_in / 4),
        s.lines_out,
        commas(s.chars_out / 4),
        path.to_string_lossy().replace('\\', "/")
    );
    let small = s.chars_out + tail.chars().count();
    if small >= s.chars_in || fs::create_dir_all(out_dir).and_then(|()| fs::write(&path, &s.plain)).is_err() {
        return Done { text: s.plain, cut: 0 };
    }
    Done { text: s.body + &tail, cut: s.chars_in - small }
}

fn read_files(ctx: &Ctx, pos: &[String]) -> Result<String, Fail> {
    let mut parts = Vec::new();
    for p in pos {
        let path = web::local_file(ctx, p);
        let file = fs::File::open(&path).map_err(|e| Fail::hint(format!("cannot read {p} ({e})"), USAGE))?;
        parts.push(decode(&read_limited(file, MAX_BYTES)));
    }
    Ok(parts.join("\n"))
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    if c.pos.first().is_some_and(|p| p == "--") {
        return Err(Fail::hint(
            "this wisply client cannot run commands yet",
            "<command> > out.txt 2>&1; wisply squeeze out.txt",
        ));
    }
    if c.pos.is_empty() {
        return Err(Fail::hint("nothing to squeeze", USAGE));
    }
    let text = read_files(ctx, &c.pos)?;
    let first = Path::new(&c.pos[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = c.val("name").map_or(first, String::from);
    let out_dir = d.home.join("out");
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("squeeze".into())
        .spawn(move || {
            let _ = tx.send(squeezed(&out_dir, &text, &name));
        })
        .map_err(|e| Fail::new(format!("squeeze could not start: {e}")))?;
    let finish = |ctx: &mut Ctx, done: Done| {
        ctx.cut += done.cut;
        done.text
    };
    if let Ok(done) = rx.recv_timeout(INLINE) {
        return Ok(Out::Text(finish(ctx, done)));
    }
    Ok(Out::Wait(Pending::new(BUDGET, POLL, move |_, ctx, last| match rx.try_recv() {
        Ok(done) => Ok(Some(finish(ctx, done))),
        Err(TryRecvError::Empty) if last => Err(Fail::hint("squeezing did not end in time", USAGE)),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => Err(Fail::hint("squeeze failed", USAGE)),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squeezed_text_points_at_the_full_file() {
        let dir = std::env::temp_dir().join(format!("wisply-squeeze-{}", std::process::id()));
        let small = squeezed(&dir, "\x1b[31mred\x1b[0m\nok\n", "x");
        assert_eq!((small.text.as_str(), small.cut), ("red\nok", 0));
        let log: String = (0..400).map(|i| format!("step {i} ok\n")).collect();
        let big = squeezed(&dir, &log, "build");
        let path = big.text.rsplit_once("all of it: ").map(|(_, p)| p.to_string()).expect("tail");
        assert!(big.cut > 0 && big.text.lines().count() < 200, "{}", big.text);
        assert_eq!(fs::read_to_string(&path).expect("full"), log.trim_end());
        let _ = fs::remove_dir_all(&dir);
    }
}
