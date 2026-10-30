use std::fs;

use lt_core::{Call, Fail};
use lt_memory::{accept_draft, drafts, drop_draft, rel_path};

use super::super::{Daemon, config};
use super::index;

const WIKI_HELP: &str = "lighting wiki | wiki accept <n> [folder] | wiki drop <n> | wiki show <n>";
const INBOX: &str = "Inbox";

pub fn wiki(d: &Daemon, c: &Call) -> Result<String, Fail> {
    let root = index::vault(&d.home);
    let name = config::text(&config::load(&d.home), "vault_inbox");
    let inbox = root.join(if name.trim().is_empty() { INBOX } else { name.trim() });
    let shown = inbox.display().to_string().replace('\\', "/");
    let found = drafts(&inbox);
    let sub = c.pos.first().map(|s| s.to_lowercase()).unwrap_or_default();
    if sub.is_empty() {
        if found.is_empty() {
            return Ok(format!("no drafts wait in {shown}"));
        }
        let mut lines =
            vec![format!("{} drafts wait in {shown} (lighting wiki accept <n> | drop <n> | show <n>):", found.len())];
        for (i, draft) in found.iter().enumerate() {
            let stem = draft.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
            lines.push(format!("  {} {} [{}]: {stem}", i + 1, draft.kind, draft.project));
        }
        return Ok(lines.join("\n"));
    }
    if !["accept", "drop", "show"].contains(&sub.as_str()) {
        return Err(Fail::hint(format!("unknown wiki command '{sub}'"), WIKI_HELP));
    }
    let pick = c.pos.get(1).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=found.len()).contains(n));
    let Some(n) = pick else {
        let which: String =
            if found.is_empty() { "no drafts wait".into() } else { format!("which draft? 1 to {}", found.len()) };
        return Err(Fail::hint(which, WIKI_HELP));
    };
    let path = &found[n - 1].path;
    let name = path.file_name().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let done = match sub.as_str() {
        "show" => {
            return fs::read(path)
                .map(|raw| String::from_utf8_lossy(&raw).into_owned())
                .map_err(|e| Fail::new(e.to_string()));
        }
        "drop" => drop_draft(&root, path).map(|_| format!("dropped {name} (in the vault's .trash)")),
        _ => {
            let folder = c.pos[2..].join(" ");
            let folder = Some(folder.trim()).filter(|f| !f.is_empty());
            accept_draft(&root, path, folder)
                .map(|dest| format!("accepted -> {} (registered in its INDEX)", rel_path(&root, &dest)))
        }
    };
    let text = done.map_err(|e| Fail::hint(e, WIKI_HELP))?;
    index::nudge();
    Ok(text)
}
