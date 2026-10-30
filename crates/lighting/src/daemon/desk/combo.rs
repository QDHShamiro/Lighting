#![cfg_attr(not(windows), allow(dead_code))]

pub const SHIFT: u16 = 0x10;
pub const CTRL: u16 = 0x11;
pub const ALT: u16 = 0x12;

const VK: &[(&str, u16)] = &[
    ("enter", 0x0D),
    ("return", 0x0D),
    ("tab", 0x09),
    ("esc", 0x1B),
    ("escape", 0x1B),
    ("backspace", 0x08),
    ("space", 0x20),
    ("delete", 0x2E),
    ("del", 0x2E),
    ("insert", 0x2D),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("left", 0x25),
    ("up", 0x26),
    ("right", 0x27),
    ("down", 0x28),
    ("ctrl", 0x11),
    ("control", 0x11),
    ("shift", 0x10),
    ("alt", 0x12),
    ("win", 0x5B),
    ("meta", 0x5B),
    ("capslock", 0x14),
    ("printscreen", 0x2C),
    ("menu", 0x5D),
    ("apps", 0x5D),
    ("pause", 0x13),
    ("numlock", 0x90),
    ("scrolllock", 0x91),
    ("arrowleft", 0x25),
    ("arrowup", 0x26),
    ("arrowright", 0x27),
    ("arrowdown", 0x28),
    ("pgup", 0x21),
    ("pgdn", 0x22),
    ("ins", 0x2D),
    ("prtsc", 0x2C),
    ("bksp", 0x08),
    ("plus", 0xBB),
    ("minus", 0xBD),
    ("comma", 0xBC),
    ("period", 0xBE),
    ("playpause", 0xB3),
    ("play", 0xB3),
    ("nexttrack", 0xB0),
    ("prevtrack", 0xB1),
    ("previoustrack", 0xB1),
    ("stop", 0xB2),
    ("mediastop", 0xB2),
    ("volumeup", 0xAF),
    ("volup", 0xAF),
    ("volumedown", 0xAE),
    ("voldown", 0xAE),
    ("mute", 0xAD),
    ("volumemute", 0xAD),
    ("browserback", 0xA6),
    ("browserforward", 0xA7),
    ("browserrefresh", 0xA8),
    ("browsersearch", 0xAA),
    ("browserhome", 0xAC),
    ("numpadmultiply", 0x6A),
    ("numpadadd", 0x6B),
    ("numpadsubtract", 0x6D),
    ("numpaddecimal", 0x6E),
    ("numpaddivide", 0x6F),
    ("lctrl", 0xA2),
    ("rctrl", 0xA3),
    ("lshift", 0xA0),
    ("rshift", 0xA1),
    ("lalt", 0xA4),
    ("ralt", 0xA5),
    ("altgr", 0xA5),
    ("lwin", 0x5B),
    ("rwin", 0x5C),
    ("super", 0x5B),
    ("cmd", 0x5B),
    ("windows", 0x5B),
    ("option", 0x12),
    ("strg", 0x11),
    ("umschalt", 0x10),
    ("entf", 0x2E),
    ("einfg", 0x2D),
    ("pos1", 0x24),
    ("bildauf", 0x21),
    ("bildab", 0x22),
];
const MODS: &[&str] = &[
    "ctrl", "control", "shift", "alt", "win", "meta", "strg", "umschalt", "super", "cmd", "windows", "option", "altgr",
    "lctrl", "rctrl", "lshift", "rshift", "lalt", "ralt", "lwin", "rwin",
];
pub const EXTENDED: &[u16] =
    &[0x2E, 0x2D, 0x24, 0x23, 0x21, 0x22, 0x25, 0x26, 0x27, 0x28, 0x5B, 0x5C, 0x5D, 0x90, 0x2C, 0xA3, 0xA5, 0x6F];
pub const MEDIA: &[&str] = &[
    "playpause",
    "play",
    "nexttrack",
    "prevtrack",
    "previoustrack",
    "stop",
    "mediastop",
    "volumeup",
    "volup",
    "volumedown",
    "voldown",
    "mute",
    "volumemute",
];
const QUICK: &[&str] = &[
    "tab",
    "esc",
    "escape",
    "backspace",
    "space",
    "delete",
    "del",
    "home",
    "end",
    "pageup",
    "pagedown",
    "pgup",
    "pgdn",
    "up",
    "down",
    "left",
    "right",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
];

pub struct Combo {
    pub mods: Vec<u16>,
    pub main: String,
    pub vk: u16,
    pub shift_state: u8,
}

pub fn parts(combo: &str) -> Vec<String> {
    let c: String = combo.chars().filter(|ch| *ch != ' ').collect();
    if c == "+" || c.ends_with("++") {
        let mut out: Vec<String> =
            c[..c.len().saturating_sub(2)].split('+').filter(|p| !p.is_empty()).map(String::from).collect();
        out.push("+".into());
        return out;
    }
    let out: Vec<String> = c.split('+').filter(|p| !p.is_empty()).map(String::from).collect();
    if out.is_empty() { vec![combo.to_string()] } else { out }
}

pub fn named(name: &str) -> Option<u16> {
    let low = name.to_lowercase();
    if let Some((_, vk)) = VK.iter().find(|(n, _)| *n == low) {
        return Some(*vk);
    }
    let n: u16 = low.strip_prefix('f').filter(|r| !r.is_empty()).and_then(|r| r.parse().ok())?;
    (1..=24).contains(&n).then_some(0x6F + n)
}

pub fn modifier(name: &str) -> bool {
    MODS.contains(&name.to_lowercase().as_str())
}

pub fn parse(combo: &str, scan: impl Fn(char) -> Option<(u16, u8)>) -> Result<Combo, String> {
    let mut list = parts(combo);
    let main = list.pop().unwrap_or_default();
    let mut mods = Vec::new();
    for p in &list {
        if !modifier(p) {
            return Err(format!("unknown modifier '{p}'"));
        }
        mods.extend(named(p));
    }
    let mut chars = main.chars();
    let (vk, shift_state) = match (named(&main), chars.next(), chars.next()) {
        (Some(vk), _, _) => (vk, 0),
        (None, Some(ch), None) if ch.is_ascii_alphanumeric() => (ch.to_ascii_uppercase() as u16, 0),
        (None, Some(ch), None) => scan(ch).ok_or_else(|| format!("unknown key '{main}'"))?,
        _ => return Err(format!("unknown key '{main}'")),
    };
    Ok(Combo { mods, main, vk, shift_state })
}

pub fn media(combo: &str) -> bool {
    MEDIA.contains(&combo.to_lowercase().as_str())
}

pub fn quick(combo: &str) -> bool {
    let list: Vec<String> = parts(combo).iter().map(|p| p.to_lowercase()).collect();
    let Some((last, mods)) = list.split_last() else { return false };
    if mods.iter().any(|m| m != "shift") {
        return false;
    }
    let mut chars = last.chars();
    QUICK.contains(&last.as_str()) || matches!((chars.next(), chars.next()), (Some(ch), None) if !ch.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_split_like_2x() {
        assert_eq!(parts("ctrl + shift + s"), ["ctrl", "shift", "s"]);
        assert_eq!(parts("ctrl++"), ["ctrl", "+"]);
        assert_eq!(parts("+"), ["+"]);
        let c = parse("Strg+Umschalt+F5", |_| None).unwrap();
        assert_eq!((c.mods, c.vk, c.main.as_str()), (vec![CTRL, SHIFT], 0x74, "F5"));
        assert_eq!(parse("a", |_| None).unwrap().vk, u16::from(b'A'));
        assert_eq!(parse("7", |_| None).unwrap().vk, u16::from(b'7'));
        assert_eq!(parse("!", |_| Some((0x31, 1))).unwrap().shift_state, 1);
        assert_eq!(parse("foo+a", |_| None).err().unwrap(), "unknown modifier 'foo'");
        assert_eq!(parse("ctrl+nothing", |_| None).err().unwrap(), "unknown key 'nothing'");
        assert_eq!(named("f25"), None);
        assert!(media("VolUp") && !media("enter"));
        assert!(quick("shift+tab") && quick("x") && !quick("ctrl+x") && !quick("enter"));
    }
}
