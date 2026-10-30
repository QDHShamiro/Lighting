pub fn local_now() -> String {
    let [year, month, day, hour, minute, second] = sys::now();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

pub fn local_parts(epoch: f64) -> [u32; 6] {
    let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let offset = stamp_secs(&local_now()).map_or(0, |local| (local - utc + 450).div_euclid(900) * 900);
    civil(epoch as i64 + offset).map(|v| u32::try_from(v).unwrap_or(0))
}

pub fn local_to_utc(local: i64) -> i64 {
    sys::to_utc(local).unwrap_or(local)
}

fn civil(secs: i64) -> [i64; 6] {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [year, month, day, rest / 3600, rest / 60 % 60, rest % 60]
}

pub fn ago(seconds: u64) -> String {
    match seconds {
        0..90 => format!("{seconds}s"),
        90..5400 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

pub fn stamp_secs(stamp: &str) -> Option<i64> {
    let num = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let shape = stamp.len() == 19 && stamp.as_bytes()[4] == b'-' && stamp.as_bytes()[13] == b':';
    if !shape {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(windows)]
mod sys {
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
        fn TzSpecificLocalTimeToSystemTime(zone: *const u8, local: *const SystemTime, utc: *mut SystemTime) -> i32;
    }

    pub fn to_utc(local: i64) -> Option<i64> {
        let [year, month, day, hour, minute, second] = super::civil(local).map(|v| u16::try_from(v).unwrap_or(0));
        let local = SystemTime { year, month, weekday: 0, day, hour, minute, second, millis: 0 };
        let mut utc = SystemTime { year: 0, month: 0, weekday: 0, day: 0, hour: 0, minute: 0, second: 0, millis: 0 };
        if unsafe { TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) } == 0 {
            return None;
        }
        let stamp = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            utc.year, utc.month, utc.day, utc.hour, utc.minute, utc.second
        );
        super::stamp_secs(&stamp)
    }

    pub fn now() -> [u32; 6] {
        let mut t = SystemTime { year: 0, month: 0, weekday: 0, day: 0, hour: 0, minute: 0, second: 0, millis: 0 };
        unsafe { GetLocalTime(&mut t) };
        [t.year, t.month, t.day, t.hour, t.minute, t.second].map(u32::from)
    }
}

#[cfg(not(windows))]
mod sys {
    use std::ffi::{c_char, c_int, c_long};

    #[repr(C)]
    struct Tm {
        sec: c_int,
        min: c_int,
        hour: c_int,
        mday: c_int,
        mon: c_int,
        year: c_int,
        wday: c_int,
        yday: c_int,
        isdst: c_int,
        gmtoff: c_long,
        zone: *const c_char,
    }

    unsafe extern "C" {
        fn time(out: *mut c_long) -> c_long;
        fn localtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm;
        fn mktime(tm: *mut Tm) -> c_long;
    }

    pub fn to_utc(local: i64) -> Option<i64> {
        let [year, mon, mday, hour, min, sec] = super::civil(local).map(|v| c_int::try_from(v).unwrap_or(0));
        let mut tm = Tm {
            sec,
            min,
            hour,
            mday,
            mon: mon - 1,
            year: year - 1900,
            wday: 0,
            yday: 0,
            isdst: -1,
            gmtoff: 0,
            zone: std::ptr::null(),
        };
        let t = unsafe { mktime(&mut tm) };
        (t != -1).then_some(i64::from(t))
    }

    pub fn now() -> [u32; 6] {
        let mut tm = Tm {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 0,
            mon: 0,
            year: 0,
            wday: 0,
            yday: 0,
            isdst: 0,
            gmtoff: 0,
            zone: std::ptr::null(),
        };
        unsafe {
            let t = time(std::ptr::null_mut());
            localtime_r(&t, &mut tm);
        }
        [tm.year + 1900, tm.mon + 1, tm.mday, tm.hour, tm.min, tm.sec].map(|v| u32::try_from(v).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_has_the_2x_log_shape() {
        let now = local_now();
        assert_eq!(now.len(), 19, "{now}");
        assert!(stamp_secs(&now).is_some_and(|s| s > 1_750_000_000), "{now}");
    }

    #[test]
    fn stamps_count_seconds_like_datetime() {
        assert_eq!(stamp_secs("1970-01-01 00:00:00"), Some(0));
        assert_eq!(stamp_secs("2026-10-30 12:34:56"), Some(1_793_363_696));
        assert_eq!(stamp_secs("2024-02-29 23:59:59"), Some(1_709_251_199));
        assert_eq!(stamp_secs("2026-10-30T12:34:56"), Some(1_793_363_696));
        assert_eq!(stamp_secs("2026-10-30"), None);
        assert_eq!(stamp_secs("2026/10/30 12:34:56"), None);
    }

    #[test]
    fn epochs_turn_into_local_dates() {
        let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let [y, mo, d, h, mi, s] = local_parts(utc);
        let shown = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
        let gap = stamp_secs(&shown).unwrap() - stamp_secs(&local_now()).unwrap();
        assert!(gap.abs() <= 2, "{shown} vs {}", local_now());
        let later = local_parts(utc + 86_400.0 * 400.0);
        assert!(later[0] > y || later[1] != mo, "{later:?}");
    }

    #[test]
    fn local_midnight_turns_into_utc() {
        let local = stamp_secs(&local_now()).unwrap();
        let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        assert!((local_to_utc(local) - utc).abs() <= 2, "{} vs {utc}", local_to_utc(local));
    }

    #[test]
    fn ages_round_like_2x() {
        assert_eq!(ago(5), "5s");
        assert_eq!(ago(89), "89s");
        assert_eq!(ago(90), "1m");
        assert_eq!(ago(5399), "89m");
        assert_eq!(ago(5400), "1h");
    }
}
