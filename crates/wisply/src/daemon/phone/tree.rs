use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use roxmltree::{Document, Node, NodeId};
use wisply_core::Fail;

use super::super::words::tiers;

const MIN_PX: i32 = 4;
const LABEL: usize = 60;
const SUB_MAX: usize = 60;
const HINT_MAX: usize = 30;
const ROLES: &[(&str, &str)] = &[
    ("Button", "button"),
    ("ImageButton", "button"),
    ("CheckBox", "checkbox"),
    ("Switch", "checkbox"),
    ("ToggleButton", "checkbox"),
    ("RadioButton", "radio"),
    ("EditText", "edit"),
    ("AutoCompleteTextView", "edit"),
    ("Spinner", "combobox"),
    ("SeekBar", "slider"),
    ("TextView", "text"),
    ("ImageView", "image"),
    ("TabWidget", "tab"),
];
static BOUNDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(\d+),(\d+)\]\[(\d+),(\d+)\]").expect("bounds"));
static SHELL_SPECIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[\\"'`$&|;<>()*?!#~ ]"#).expect("special"));

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub x: i32,
    pub y: i32,
    pub name: String,
    pub role: &'static str,
}

pub struct Snap {
    pub app: String,
    pub lines: Vec<(Vec<u8>, String)>,
    pub refs: Vec<(String, Target)>,
}

pub fn parse_devices(out: &str) -> Vec<(String, String, String)> {
    out.lines()
        .skip(1)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let model = parts.iter().find_map(|p| p.strip_prefix("model:")).unwrap_or("");
            (parts.len() >= 2).then(|| (parts[0].to_string(), parts[1].to_string(), model.to_string()))
        })
        .collect()
}

fn bounds(text: Option<&str>) -> (i32, i32, i32, i32) {
    BOUNDS.captures(text.unwrap_or("")).map_or((0, 0, 0, 0), |m| {
        let n = |i: usize| m[i].parse().unwrap_or(0);
        (n(1), n(2), n(3), n(4))
    })
}

fn yes(node: Node, attr: &str) -> bool {
    node.attribute(attr) == Some("true")
}

fn own_text(node: Node) -> String {
    let t = node.attribute("text").filter(|t| !t.is_empty()).or_else(|| node.attribute("content-desc")).unwrap_or("");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn kind(node: Node) -> &'static str {
    let cls = node.attribute("class").unwrap_or("");
    let short = cls.rsplit('.').next().unwrap_or(cls);
    if let Some((_, r)) = ROLES.iter().find(|(c, _)| *c == short) {
        return r;
    }
    if yes(node, "checkable") {
        return "checkbox";
    }
    if yes(node, "clickable") || yes(node, "long-clickable") { "button" } else { "text" }
}

fn is_control(node: Node) -> bool {
    yes(node, "clickable")
        || yes(node, "checkable")
        || yes(node, "long-clickable")
        || matches!(kind(node), "edit" | "slider")
}

fn kids<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(|k| k.has_tag_name("node"))
}

fn inner<'a, 'i>(node: Node<'a, 'i>, out: &mut Vec<String>, toggles: &mut Vec<Node<'a, 'i>>) {
    for kid in kids(node) {
        if yes(kid, "checkable") {
            toggles.push(kid);
        } else if is_control(kid) && !own_text(kid).is_empty() {
            continue;
        }
        let t = own_text(kid);
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
        inner(kid, out, toggles);
    }
}

fn describe<'a, 'i>(node: Node<'a, 'i>) -> (&'static str, String, String, Vec<Node<'a, 'i>>) {
    let mut role = kind(node);
    let mut name = own_text(node);
    let (mut texts, mut toggles) = (Vec::new(), Vec::new());
    if role != "edit" {
        inner(node, &mut texts, &mut toggles);
    }
    if name.is_empty()
        && let Some(first) = texts.first()
    {
        name.clone_from(first);
    }
    let sub = if texts.len() > 1 && texts[0] == name { texts[1].clone() } else { String::new() };
    let check = if yes(node, "checkable") { Some(node) } else { toggles.first().copied() };
    if check.is_some() && matches!(role, "button" | "text" | "checkbox") {
        role = "checkbox";
    }
    if name.is_empty()
        && role != "text"
        && let Some(id) = node.attribute("resource-id").filter(|id| !id.is_empty())
    {
        name = format!("#{}", id.rsplit('/').next().unwrap_or(id));
    }
    let mut extra = String::new();
    if let Some(check) = check {
        extra += if yes(check, "checked") { " [x]" } else { " [ ]" };
    }
    if role == "edit" {
        let hint = node.attribute("hint").unwrap_or("");
        if yes(node, "password") {
            extra += " =***";
        } else if !hint.is_empty() && hint != name {
            extra += &format!(" ({})", hint.chars().take(HINT_MAX).collect::<String>());
        }
        if yes(node, "focused") {
            extra += " (focused)";
        }
    }
    if yes(node, "selected") {
        extra += " *";
    }
    if node.attribute("enabled") == Some("false") {
        extra += " (disabled)";
    }
    if !sub.is_empty() && sub.chars().count() <= SUB_MAX {
        extra += &format!(" - {sub}");
    }
    (role, name, extra, toggles)
}

struct Walk<'f> {
    filt: &'f [String],
    seen: HashSet<NodeId>,
    snap: Snap,
}

impl Walk<'_> {
    fn add(&mut self, role: &'static str, name: String, extra: &str, x: i32, y: i32) {
        let r = format!("p{}", self.snap.refs.len() + 1);
        let label = if name.chars().count() <= LABEL {
            name.clone()
        } else {
            name.chars().take(LABEL - 3).collect::<String>() + "..."
        };
        let quoted = if label.is_empty() { String::new() } else { format!(" \"{}\"", label.replace('"', "'")) };
        let line = format!("{r} {role}{quoted}{extra}");
        let tier = if self.filt.is_empty() { Vec::new() } else { tiers(self.filt, &[&name, role, extra]) };
        if self.filt.is_empty() || tier.iter().any(|t| *t > 0) {
            self.snap.lines.push((tier, line));
        }
        self.snap.refs.push((r, Target { x, y, name, role }));
    }

    fn visit(&mut self, node: Node, inside: bool) {
        if self.snap.app.is_empty() {
            self.snap.app = node.attribute("package").unwrap_or("").to_string();
        }
        let (l, t, r, b) = bounds(node.attribute("bounds"));
        let big = r - l >= MIN_PX && b - t >= MIN_PX;
        let control = is_control(node) && !self.seen.contains(&node.id());
        let mut mine = inside;
        if big && control {
            let (role, name, extra, toggles) = describe(node);
            self.seen.extend(toggles.iter().map(Node::id));
            if !name.is_empty() || matches!(role, "edit" | "slider") {
                let (mut x, mut y) = ((l + r) / 2, (t + b) / 2);
                if let Some(first) = toggles.first().filter(|_| !yes(node, "checkable")) {
                    let (tl, tt, tr, tb) = bounds(first.attribute("bounds"));
                    if tr - tl >= MIN_PX && tb - tt >= MIN_PX {
                        (x, y) = ((tl + tr) / 2, (tt + tb) / 2);
                    }
                }
                self.add(role, name, &extra, x, y);
                mine = true;
            }
        } else if big && !inside && !self.seen.contains(&node.id()) && !yes(node, "scrollable") {
            let name = own_text(node);
            if !name.is_empty() {
                self.add("text", name, "", (l + r) / 2, (t + b) / 2);
            }
        }
        for kid in kids(node) {
            self.visit(kid, mine);
        }
    }
}

pub fn snap_of(xml: &str, filt: &[String]) -> Result<Snap, Fail> {
    let doc = Document::parse(xml).map_err(|e| Fail::new(format!("the phone gave a broken screen tree: {e}")))?;
    let mut walk =
        Walk { filt, seen: HashSet::new(), snap: Snap { app: String::new(), lines: Vec::new(), refs: Vec::new() } };
    for top in kids(doc.root_element()) {
        walk.visit(top, false);
    }
    Ok(walk.snap)
}

pub fn tree_xml(text: &str) -> Option<&str> {
    let end_tag = "</hierarchy>";
    let (start, end) = (text.find("<?xml")?, text.rfind(end_tag)?);
    (start < end).then(|| &text[start..end + end_tag.len()])
}

pub fn escape_text(text: &str) -> String {
    SHELL_SPECIAL
        .replace_all(text, |m: &regex::Captures| if &m[0] == " " { "%s".into() } else { format!("\\{}", &m[0]) })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0"><node index="0" text="" class="android.widget.FrameLayout" package="com.example.app" checkable="false" checked="false" clickable="false" enabled="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][1080,2400]"><node text="Settings &amp; more" class="android.widget.TextView" bounds="[0,100][1080,200]" /><node text="" class="android.widget.LinearLayout" clickable="true" bounds="[0,200][1080,350]"><node text="Wi-Fi" class="android.widget.TextView" bounds="[40,210][600,260]" /><node text="Connected to &quot;Home&quot;" class="android.widget.TextView" bounds="[40,270][600,320]" /><node text="" class="android.widget.Switch" checkable="true" checked="true" clickable="false" bounds="[900,240][1040,310]" /></node><node text="" class="android.widget.EditText" clickable="true" focused="true" hint="Search settings" bounds="[0,400][1080,500]" /><node text="" class="android.widget.EditText" clickable="true" password="true" bounds="[0,520][1080,600]" /><node text="" resource-id="com.example.app:id/fab_add" class="android.widget.ImageButton" clickable="true" bounds="[900,2200][1040,2340]" /><node text="Tab A" class="android.widget.TextView" clickable="true" selected="true" bounds="[0,620][500,700]" /><node text="Off" class="android.widget.Button" clickable="true" enabled="false" bounds="[0,720][500,800]" /><node text="" content-desc="A very long description that goes on and on past the sixty char limit" class="android.widget.ImageView" clickable="true" bounds="[0,820][500,900]" /><node text="" class="androidx.recyclerview.widget.RecyclerView" scrollable="true" bounds="[0,900][1080,2000]"><node text="Row one" class="android.widget.TextView" bounds="[0,910][1080,980]" /></node><node text="tiny" class="android.widget.TextView" bounds="[0,0][2,2]" /></node></hierarchy>"#;

    #[test]
    fn the_screen_tree_reads_like_2x() {
        let xml = tree_xml(UI).unwrap();
        let snap = snap_of(xml, &[]).unwrap();
        let lines: Vec<&str> = snap.lines.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(snap.app, "com.example.app");
        assert_eq!(
            lines.join("\n"),
            "p1 text \"Settings & more\"\np2 checkbox \"Wi-Fi\" [x] - Connected to \"Home\"\np3 edit (Search settings) (focused)\np4 edit =***\np5 button \"#fab_add\"\np6 text \"Tab A\" *\np7 button \"Off\" (disabled)\np8 image \"A very long description that goes on and on past the sixt...\"\np9 text \"Row one\""
        );
        let at = |i: usize| (snap.refs[i].1.x, snap.refs[i].1.y, snap.refs[i].1.role);
        assert_eq!((at(1), at(4), at(8)), ((970, 275, "checkbox"), (970, 2270, "button"), (540, 945, "text")));
        let filt = vec!["wi-fi".to_string(), "search".to_string()];
        let hits: Vec<String> = snap_of(xml, &filt).unwrap().lines.into_iter().map(|(_, l)| l).collect();
        assert!(hits.iter().any(|l| l.starts_with("p2 ")) && hits.iter().any(|l| l.starts_with("p3 ")));
    }

    #[test]
    fn devices_and_typed_text_match_adb() {
        let out = "List of devices attached\nR58M123 device usb:1-1 product:x model:Pixel_7 device:y\nemulator-5554 unauthorized\n\n";
        assert_eq!(
            parse_devices(out),
            vec![
                ("R58M123".into(), "device".into(), "Pixel_7".into()),
                ("emulator-5554".into(), "unauthorized".into(), String::new())
            ]
        );
        assert_eq!(escape_text("hi there (you)! $5 & 'x'"), r"hi%sthere%s\(you\)\!%s\$5%s\&%s\'x\'");
        assert!(tree_xml("junk").is_none());
    }
}
