use lt_core::{Call, Fail};
use serde_json::{Value, json};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::{SNAP_LINES, call, cmds, keys, obj, out};

const STEPS: &[(&str, i64)] = &[("ctrl+tab", 1), ("ctrl+pagedown", 1), ("ctrl+shift+tab", -1), ("ctrl+pageup", -1)];
const CLOSE: &[&str] = &["ctrl+w", "ctrl+f4"];
const NEW_TAB: &str = "lighting open <url> --new (a new tab in the background)";
const OWN_TAB: &str = "lighting close (Lighting closes its own tab, browser windows stay)";
const BROWSER_ONLY: &[(&str, &str)] = &[
    ("ctrl+t", NEW_TAB),
    ("ctrl+n", NEW_TAB),
    ("ctrl+shift+n", NEW_TAB),
    ("ctrl+shift+t", NEW_TAB),
    ("ctrl+shift+w", OWN_TAB),
    ("alt+f4", OWN_TAB),
];
const NO_START: &str = "lighting tabs, then lighting tab t3";
const TABS_WAIT: f64 = 15.0;

fn step(combo: &str) -> Option<i64> {
    STEPS.iter().find(|(k, _)| *k == combo).map(|(_, n)| *n)
}

fn field<'a>(row: &'a Value, key: &str) -> &'a Value {
    row.get(key).unwrap_or(&Value::Null)
}

fn moved(rows: &[Value], own: Option<&str>, by: i64) -> Option<(String, String)> {
    let id = |r: &Value| field(r, "id").as_str().unwrap_or("").to_string();
    let base = own
        .and_then(|o| rows.iter().find(|r| id(r) == o))
        .or_else(|| rows.iter().find(|r| field(r, "target").as_bool() == Some(true)))
        .or_else(|| rows.iter().find(|r| field(r, "active").as_bool() == Some(true)))?;
    let mut row: Vec<&Value> = rows.iter().filter(|r| field(r, "win") == field(base, "win")).collect();
    row.sort_by_key(|r| field(r, "index").as_i64().unwrap_or(0));
    let at = row.iter().position(|r| id(r) == id(base))?;
    let len = i64::try_from(row.len()).ok()?;
    let next = usize::try_from((i64::try_from(at).ok()? + by).rem_euclid(len)).ok()?;
    Some((id(base), id(row[next])))
}

pub fn press(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Option<String>, Fail> {
    let combos: Vec<String> = c.pos.iter().map(|p| keys::canon(p)).collect();
    for (p, k) in c.pos.iter().zip(&combos) {
        if let Some((_, hint)) = BROWSER_ONLY.iter().find(|(b, _)| b == k) {
            return Err(Fail::hint(format!("{p} is a browser shortcut that Lighting cannot press"), *hint));
        }
    }
    if let [only] = combos.as_slice()
        && CLOSE.contains(&only.as_str())
    {
        let force = c.flags.iter().filter(|(k, _)| *k == "force").map(|(k, v)| (k.clone(), v.clone())).collect();
        let closed = cmds::close(d, ctx, &Call { name: "close".into(), pos: Vec::new(), flags: force })?;
        return Ok(Some(format!("{}: {closed}", c.pos[0])));
    }
    let Some(by) = combos.iter().map(|k| step(k)).sum::<Option<i64>>() else { return Ok(None) };
    let msg = call(d, ctx, "tabs", obj(json!({"rows": true})), TABS_WAIT)?;
    let Some(rows) = msg.get("rows").and_then(Value::as_array) else {
        return Err(Fail::hint(
            "the Lighting extension in the browser is older than this Lighting",
            "lighting ext-reload",
        ));
    };
    let Some((base, next)) = moved(rows, ctx.tabs.first().map(String::as_str), by) else {
        return Err(Fail::hint("no tab to start from", NO_START));
    };
    if base == next {
        return Ok(Some(format!("{}: {base} is the only tab in its window, nothing to switch to", c.pos.join(" "))));
    }
    let res = out(d, ctx, "tab", obj(json!({"id": next})), TABS_WAIT, SNAP_LINES)?;
    Ok(Some(format!("{res} ({}: Lighting's target tab moved, the visible tab stays)", c.pos.join(" "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, win: i64, index: i64, active: bool, target: bool) -> Value {
        json!({"id": id, "win": win, "index": index, "active": active, "target": target})
    }

    #[test]
    fn tab_keys_step_through_the_window_of_lightings_tab() {
        let rows = [
            row("t1", 1, 0, false, false),
            row("t2", 1, 1, true, false),
            row("t3", 1, 2, false, true),
            row("t9", 2, 0, true, false),
        ];
        assert_eq!(moved(&rows, Some("t3"), 1).map(|(_, n)| n).as_deref(), Some("t1"));
        assert_eq!(moved(&rows, Some("t3"), -1).map(|(_, n)| n).as_deref(), Some("t2"));
        assert_eq!(moved(&rows, Some("t3"), 2).map(|(_, n)| n).as_deref(), Some("t2"));
        assert_eq!(moved(&rows, Some("gone"), -1).map(|(_, n)| n).as_deref(), Some("t2"));
        assert_eq!(moved(&rows, Some("t9"), 1).map(|(_, n)| n).as_deref(), Some("t9"));
        assert_eq!(moved(&rows[..2], None, 1).map(|(_, n)| n).as_deref(), Some("t1"));
        assert_eq!(moved(&[], None, 1), None);
    }

    #[test]
    fn only_chromium_reserved_keys_stay_out_of_the_page() {
        let kind = |k: &str| {
            let k = keys::canon(k);
            (step(&k), CLOSE.contains(&k.as_str()), BROWSER_ONLY.iter().any(|(b, _)| *b == k))
        };
        assert_eq!(kind("Ctrl+Tab"), (Some(1), false, false));
        assert_eq!(kind("shift+ctrl+tab"), (Some(-1), false, false));
        assert_eq!(kind("strg+pgdn"), (Some(1), false, false));
        assert_eq!(kind("control+prior"), (Some(-1), false, false));
        assert_eq!(kind("ctrl+W"), (None, true, false));
        assert_eq!(kind("Ctrl+T"), (None, false, true));
        assert_eq!(kind("shift+ctrl+n"), (None, false, true));
        for page in ["Enter", "ctrl+1", "ctrl+9", "ctrl+l", "ctrl+f", "ctrl+a", "ctrl+shift+p"] {
            assert_eq!(kind(page), (None, false, false), "{page}");
        }
    }
}
