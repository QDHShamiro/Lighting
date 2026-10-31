use lt_core::stamp_secs;

const DAY_S: i64 = 86_400;

pub fn commas(n: i64) -> String {
    let sign = if n < 0 { "-" } else { "" };
    format!("{sign}{}", lt_squeeze::commas(usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX)))
}

pub fn short(n: f64) -> String {
    for (size, unit) in [(1e9, "B"), (1e6, "M"), (1e3, "k")] {
        if n >= size {
            let v = n / size;
            return if n >= 10.0 * size { format!("{v:.0}{unit}") } else { format!("{v:.1}{unit}") };
        }
    }
    format!("{}", n as i64)
}

pub fn pyround(x: f64) -> i64 {
    x.round_ties_even() as i64
}

pub fn day_no(day: &str) -> Option<i64> {
    stamp_secs(&format!("{} 00:00:00", day.get(..10)?)).map(|s| s.div_euclid(DAY_S))
}

pub fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

pub fn date_of(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_days_print_like_python() {
        assert_eq!([short(999.0), short(1_250.0), short(15_850.0), short(2.5e9)], ["999", "1.2k", "16k", "2.5B"]);
        assert_eq!([commas(0), commas(1_234_567), commas(-1_000)], ["0", "1,234,567", "-1,000"]);
        assert_eq!([pyround(2.5), pyround(3.5), pyround(-0.5), pyround(0.6)], [2, 4, 0, 1]);
        let d = day_no("2026-10-30").expect("day");
        assert_eq!(
            (date_of(d), weekday(d), date_of(d - weekday(d))),
            ("2026-10-30".to_string(), 4, "2026-10-26".to_string())
        );
        assert_eq!(date_of(day_no("2024-02-29").expect("leap")), "2024-02-29");
    }
}
