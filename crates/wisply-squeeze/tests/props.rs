use proptest::collection::{btree_map, vec};
use proptest::prelude::*;
use serde_json::{Map, Value};
use wisply_squeeze::squeeze;

const SAFE: [&str; 10] = ["alpha", "beta", "gamma", "delta", "step", "done", "row", "value", "ok", "item"];

fn filler() -> impl Strategy<Value = String> {
    vec(prop::sample::select(SAFE.to_vec()), 2..8).prop_map(|w| w.join(" "))
}

fn noisy_line() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ -~]{0,90}",
        "\\PC{0,40}",
        "[a-z ]{0,20}\r[a-z %0-9]{0,20}",
        "\x1b\\[3[0-7]m[a-z]{1,9}\x1b\\[0m error [0-9]{1,4}",
        "\tat [a-z.]{1,30}\\([A-Z][a-z]+\\.java:[0-9]{1,4}\\)",
        "\\{\"[a-z]{1,6}\": [0-9]{1,5}\\}",
        Just(String::new()),
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        (-1e12f64..1e12).prop_map(Value::from),
        "\\PC{0,60}".prop_map(Value::from),
        Just(Value::from("fatal error: disk full")),
    ];
    leaf.prop_recursive(4, 96, 10, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..14).prop_map(Value::Array),
            btree_map("[a-z]{1,6}", inner, 0..7).prop_map(|m| Value::Object(m.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn never_larger_and_the_same_every_time(lines in vec(noisy_line(), 0..400), crlf in any::<bool>()) {
        let text = lines.join(if crlf { "\r\n" } else { "\n" });
        let first = squeeze(&text);
        let again = squeeze(&text);
        prop_assert_eq!(first.as_ref().map(|s| &s.body), again.as_ref().map(|s| &s.body));
        if let Some(s) = first {
            prop_assert!(s.chars_out < s.chars_in);
            prop_assert!(s.chars_in <= text.chars().count());
            prop_assert_eq!(s.chars_out, s.body.chars().count());
        }
    }

    #[test]
    fn error_lines_stay(lines in vec(filler(), 150..400), errors in btree_map(0usize..400, "[a-z]{6}", 1..8)) {
        let mut lines = lines;
        let mut want = Vec::new();
        for (at, tag) in errors {
            let line = format!("ERROR {tag} failed");
            let at = at % lines.len();
            lines.insert(at, line.clone());
            want.push(line);
        }
        let s = squeeze(&lines.join("\n")).expect("a long log is squeezed");
        for line in want {
            prop_assert!(s.body.lines().any(|l| l.starts_with(&line)), "lost {line}");
        }
    }

    #[test]
    fn json_error_rows_stay(n in 70usize..300, bad in 0usize..300) {
        let bad = bad % n;
        let rows: Vec<String> = (0..n)
            .map(|i| {
                let state = if i == bad { "crashed with a fatal error" } else { "fine" };
                format!("{{\"id\":{i},\"state\":\"{state}\"}}")
            })
            .collect();
        let s = squeeze(&rows.join("\n")).expect("json lines are squeezed");
        prop_assert!(s.json);
        prop_assert!(s.body.contains("crashed with a fatal error"));
    }

    #[test]
    fn any_json_is_safe(v in json_value()) {
        let text = serde_json::to_string_pretty(&v).expect("json text");
        let first = squeeze(&text);
        let again = squeeze(&text);
        prop_assert_eq!(first.as_ref().map(|s| &s.body), again.as_ref().map(|s| &s.body));
        if let Some(s) = first {
            prop_assert!(s.chars_out < s.chars_in);
        }
    }
}
