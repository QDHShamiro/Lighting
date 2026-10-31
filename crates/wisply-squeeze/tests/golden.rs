use std::fs;
use std::path::Path;

#[test]
fn every_fixture_matches_the_python_squeeze() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("golden dir") {
        let input = entry.expect("golden entry").path();
        if input.extension().is_none_or(|e| e != "in") {
            continue;
        }
        let text = fs::read_to_string(&input).expect("golden input");
        let want = fs::read_to_string(input.with_extension("out")).expect("golden output");
        let got = wisply_squeeze::squeeze(&text).map_or_else(|| "<unchanged>".to_string(), |s| s.body);
        if got != want {
            let at = got
                .lines()
                .zip(want.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| got.lines().count().min(want.lines().count()));
            panic!(
                "{} differs at line {}\n got: {:?}\nwant: {:?}",
                input.display(),
                at + 1,
                got.lines().nth(at),
                want.lines().nth(at)
            );
        }
        checked += 1;
    }
    assert!(checked >= 16, "only {checked} golden fixtures");
}
