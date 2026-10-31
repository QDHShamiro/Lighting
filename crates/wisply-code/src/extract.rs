use std::sync::LazyLock;

use regex::Regex;
use tree_sitter::{Node, Parser};

const CALLS: &[&str] = &[
    "call",
    "call_expression",
    "method_invocation",
    "invocation_expression",
    "macro_invocation",
    "object_creation_expression",
    "new_expression",
    "function_call",
    "method_call",
    "method_call_expression",
];
const CALLEE_FIELDS: &[&str] = &["name", "function", "type", "constructor", "method", "macro"];
const NAME_KINDS: &[&str] = &["identifier", "type_identifier", "simple_identifier", "constant", "property_identifier"];
const SIG_CHARS: usize = 160;

static DEF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:function|method|class|interface|enum|struct|trait|record|module|constructor|type_alias|object|protocol|annotation_type|union|namespace|mod)_(?:definition|declaration|item|signature)$|^(?:function_item|struct_item|enum_item|trait_item|mod_item|type_item|macro_definition)$",
    )
    .expect("def")
});
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_]\w*").expect("word"));

#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: String,
    pub kind: String,
    pub line: i64,
    pub end: i64,
    pub parent: String,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    pub name: String,
    pub line: i64,
    pub caller: String,
}

fn text<'a>(node: Option<Node>, data: &'a [u8]) -> &'a str {
    node.and_then(|n| n.utf8_text(data).ok()).unwrap_or("")
}

fn name_of(node: Node, data: &[u8]) -> String {
    let named = node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|c| NAME_KINDS.contains(&c.kind()))
    });
    text(named, data).to_string()
}

fn callee(node: Node, data: &[u8]) -> String {
    for field in CALLEE_FIELDS {
        if let Some(n) = node.child_by_field_name(field) {
            return WORD.find_iter(text(Some(n), data)).last().map(|m| m.as_str().to_string()).unwrap_or_default();
        }
    }
    String::new()
}

pub fn extract(parser: &mut Parser, data: &[u8]) -> (Vec<Def>, Vec<Ref>) {
    let Some(tree) = parser.parse(data, None) else { return (Vec::new(), Vec::new()) };
    let (mut defs, mut refs) = (Vec::new(), Vec::new());
    let mut stack = vec![(tree.root_node(), String::new())];
    while let Some((node, owner)) = stack.pop() {
        let mut here = owner.clone();
        let kind = node.kind();
        if DEF.is_match(kind) {
            let name = name_of(node, data);
            if !name.is_empty() {
                let first = text(Some(node), data).lines().next().unwrap_or("").trim();
                defs.push(Def {
                    name: name.clone(),
                    kind: kind.split('_').next().unwrap_or(kind).to_string(),
                    line: node.start_position().row as i64 + 1,
                    end: node.end_position().row as i64 + 1,
                    parent: owner.clone(),
                    sig: first.chars().take(SIG_CHARS).collect(),
                });
                here = name;
            }
        } else if CALLS.contains(&kind) {
            let name = callee(node, data);
            if !name.is_empty() {
                refs.push(Ref { name, line: node.start_position().row as i64 + 1, caller: owner.clone() });
            }
        }
        let mut cursor = node.walk();
        let kids: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(kids.into_iter().rev().map(|k| (k, here.clone())));
    }
    (defs, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_match_2x_node_names() {
        for kind in ["function_definition", "class_declaration", "function_item", "method_declaration", "mod_item"] {
            assert!(DEF.is_match(kind), "{kind}");
        }
        for kind in ["call_expression", "identifier", "function_type", "class_body"] {
            assert!(!DEF.is_match(kind), "{kind}");
        }
    }
}
