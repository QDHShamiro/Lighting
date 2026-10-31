const ROLES: &[(&str, &str)] = &[
    ("button", "button"),
    ("check_box", "checkbox"),
    ("switch", "checkbox"),
    ("radio_button", "radio"),
    ("text_field", "edit"),
    ("text_area", "edit"),
    ("combo_box", "combobox"),
    ("link", "link"),
    ("list_item", "listitem"),
    ("menu_item", "menuitem"),
    ("slider", "slider"),
    ("spin_button", "spinner"),
    ("tab", "tab"),
    ("tree_item", "treeitem"),
    ("table_row", "dataitem"),
    ("table_cell", "dataitem"),
    ("web_area", "document"),
    ("static_text", "text"),
    ("heading", "text"),
    ("window", "window"),
    ("dialog", "window"),
    ("alert", "window"),
];
const VALUED: &[&str] = &["edit", "combobox", "document", "spinner", "slider"];
const QUIET: &[&str] = &["listitem", "dataitem", "treeitem", "tab", "button", "link", "menuitem"];
const MAX_TEXT: usize = 120;
const VALUE_SHOWN: usize = 40;
const VALUE_MAX: usize = 43;

pub struct Node {
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub checked: Option<Option<bool>>,
    pub selected: bool,
    pub expanded: Option<bool>,
    pub enabled: bool,
    pub secret: bool,
}

pub fn role(platform: &str) -> Option<&'static str> {
    ROLES.iter().find(|(p, _)| *p == platform).map(|(_, r)| *r)
}

pub fn extra(n: &Node, role: &str) -> String {
    let mut s = String::new();
    if let Some(state) = n.checked {
        s += match state {
            Some(true) => " [x]",
            None => " [-]",
            Some(false) => " [ ]",
        };
    }
    if VALUED.contains(&role) {
        if n.secret {
            s += " =***";
        } else if let Some(v) = n.value.as_deref().filter(|v| !v.trim().is_empty()) {
            let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
            let shown =
                if v.chars().count() > VALUE_MAX { v.chars().take(VALUE_SHOWN).collect::<String>() + "..." } else { v };
            s += &format!(" =\"{}\"", shown.replace('"', "'"));
        }
    }
    if n.selected {
        s += " *";
    }
    match n.expanded {
        Some(false) => s += " (collapsed)",
        Some(true) => s += " (expanded)",
        None => {}
    }
    if !n.enabled {
        s += " (disabled)";
    }
    s
}

#[derive(Debug, PartialEq, Eq)]
pub enum Row {
    Text(String),
    Control(String),
}

impl Row {
    pub fn with_ref(self, r: &str) -> String {
        match self {
            Row::Text(t) => t,
            Row::Control(c) => format!("{r} {c}"),
        }
    }
}

pub fn line(n: &Node) -> Option<Row> {
    let role = role(&n.role)?;
    if role == "window" {
        return None;
    }
    let name = n.name.trim();
    let extra = extra(n, role);
    if role == "text" {
        return (!name.is_empty() && name.chars().count() <= MAX_TEXT).then(|| Row::Text(format!("  {name}")));
    }
    if name.is_empty() && extra.is_empty() && QUIET.contains(&role) {
        return None;
    }
    let label = super::view::short(name, 60);
    let quoted = if label.is_empty() { String::new() } else { format!(" \"{}\"", label.replace('"', "'")) };
    Some(Row::Control(format!("{role}{quoted}{extra}")))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Named(&'static str),
    F(u8),
}

const NAMED: &[(&str, &str)] = &[
    ("enter", "enter"),
    ("return", "enter"),
    ("esc", "escape"),
    ("escape", "escape"),
    ("tab", "tab"),
    ("space", "space"),
    ("backspace", "backspace"),
    ("delete", "delete"),
    ("del", "delete"),
    ("insert", "insert"),
    ("up", "up"),
    ("down", "down"),
    ("left", "left"),
    ("right", "right"),
    ("home", "home"),
    ("end", "end"),
    ("pageup", "pageup"),
    ("pgup", "pageup"),
    ("pagedown", "pagedown"),
    ("pgdn", "pagedown"),
];
const HELD: &[(&str, &str)] = &[
    ("ctrl", "ctrl"),
    ("control", "ctrl"),
    ("strg", "ctrl"),
    ("shift", "shift"),
    ("alt", "alt"),
    ("option", "alt"),
    ("cmd", "meta"),
    ("command", "meta"),
    ("win", "meta"),
    ("meta", "meta"),
    ("super", "meta"),
];

pub fn combo(text: &str) -> Result<(Vec<&'static str>, Key), String> {
    let parts: Vec<String> = text.split('+').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
    let Some((last, held)) = parts.split_last() else { return Err("which key?".into()) };
    let mut mods = Vec::new();
    for h in held {
        let m = HELD.iter().find(|(n, _)| n == h).map(|(_, m)| *m).ok_or_else(|| format!("unknown modifier {h}"))?;
        mods.push(m);
    }
    if let Some(m) = HELD.iter().find(|(n, _)| n == last).map(|(_, m)| *m) {
        return Err(format!("{m} alone is not a key here"));
    }
    let key = if let Some((_, k)) = NAMED.iter().find(|(n, _)| n == last) {
        Key::Named(k)
    } else if let Some(n) = last.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=24).contains(n)) {
        Key::F(n)
    } else {
        let mut chars = last.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Key::Char(c),
            _ => return Err(format!("unknown key {last}")),
        }
    };
    Ok((mods, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(role: &str, name: &str) -> Node {
        Node {
            role: role.into(),
            name: name.into(),
            value: None,
            checked: None,
            selected: false,
            expanded: None,
            enabled: true,
            secret: false,
        }
    }

    #[test]
    fn mac_and_linux_controls_read_like_windows() {
        assert_eq!(line(&node("button", "Save")).map(|r| r.with_ref("d1")).as_deref(), Some("d1 button \"Save\""));
        assert_eq!(line(&node("button", "")), None);
        assert_eq!(line(&node("static_text", "Hello")).map(|r| r.with_ref("d3")).as_deref(), Some("  Hello"));
        assert_eq!(line(&node("group", "x")), None);
        let mut field = node("text_field", "Name");
        field.value = Some("Ada   \"L\"".into());
        assert_eq!(line(&field).map(|r| r.with_ref("d5")).as_deref(), Some("d5 edit \"Name\" =\"Ada 'L'\""));
        field.secret = true;
        assert_eq!(line(&field).map(|r| r.with_ref("d5")).as_deref(), Some("d5 edit \"Name\" =***"));
        let mut boxed = node("check_box", "Bold");
        boxed.checked = Some(Some(true));
        boxed.enabled = false;
        assert_eq!(line(&boxed).map(|r| r.with_ref("d6")).as_deref(), Some("d6 checkbox \"Bold\" [x] (disabled)"));
        let mut item = node("tree_item", "src");
        item.expanded = Some(false);
        item.selected = true;
        assert_eq!(line(&item).map(|r| r.with_ref("d7")).as_deref(), Some("d7 treeitem \"src\" * (collapsed)"));
    }

    #[test]
    fn key_combos_parse_for_xa11y() {
        assert_eq!(combo("ctrl+s"), Ok((vec!["ctrl"], Key::Char('s'))));
        assert_eq!(combo("Cmd+Shift+Tab"), Ok((vec!["meta", "shift"], Key::Named("tab"))));
        assert_eq!(combo("f5"), Ok((vec![], Key::F(5))));
        assert_eq!(combo("enter"), Ok((vec![], Key::Named("enter"))));
        assert!(combo("ctrl").is_err() && combo("hyper+x").is_err() && combo("ctrl+xyz").is_err());
    }
}
