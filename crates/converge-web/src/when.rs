//! Relative date rendering (D6). Decisions carry real ISO-8601 UTC timestamps;
//! screens render them as the familiar "1h ago" / "yesterday" / "Mar 8"
//! strings. Pure functions, no chrono — the only platform-dependent bit is
//! reading "now", isolated in [`when`].

/// Strict `YYYY-MM-DDTHH:MM:SSZ` → epoch seconds. Anything else is `None`.
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &s[r];
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    Some(days_from_civil(y, m as u32, d as u32) * 86400 + hh * 3600 + mm * 60 + ss)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64; // [0, 11], Mar = 0
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Civil date from days since 1970-01-01 (inverse of `days_from_civil`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Render a captured-at epoch relative to `now`:
/// `just now` → `Nm ago` → `Nh ago` → `yesterday` → `Nd ago` → `Mar 8`
/// (same calendar year) → `Jan 2025`.
pub fn format_when(captured_epoch: i64, now_epoch: i64) -> String {
    let delta = now_epoch - captured_epoch;
    if delta < 60 {
        return "just now".into(); // includes clock skew (captured "in the future")
    }
    if delta < 3600 {
        return format!("{}m ago", delta / 60);
    }
    if delta < 86400 {
        return format!("{}h ago", delta / 3600);
    }
    if delta < 2 * 86400 {
        return "yesterday".into();
    }
    if delta < 7 * 86400 {
        return format!("{}d ago", delta / 86400);
    }
    let (cy, cm, cd) = civil_from_days(captured_epoch.div_euclid(86400));
    let (ny, _, _) = civil_from_days(now_epoch.div_euclid(86400));
    let month = MONTHS[(cm - 1) as usize];
    if cy == ny {
        format!("{month} {cd}")
    } else {
        format!("{month} {cy}")
    }
}

/// Render an ISO timestamp relative to the current time. Unparseable input is
/// returned unchanged (defensive; seed validation makes it unreachable).
pub fn when(iso: &str) -> String {
    match parse_iso_utc(iso) {
        Some(t) => format_when(t, now_epoch()),
        None => iso.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
fn now_epoch() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn now_epoch() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        // One civil day apart == 86400 s.
        let a = parse_iso_utc("2026-06-30T00:00:00Z").unwrap();
        let b = parse_iso_utc("2026-07-01T00:00:00Z").unwrap();
        assert_eq!(b - a, 86400);
        // Rejects: wrong length, non-digit, bad ranges, missing Z.
        assert_eq!(parse_iso_utc("2026-1-01T00:00:00Z"), None);
        assert_eq!(parse_iso_utc("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso_utc("2026-01-01T24:00:00Z"), None);
        assert_eq!(parse_iso_utc("2026-01-01 00:00:00Z"), None);
        assert_eq!(parse_iso_utc("2026-01-01T00:00:00"), None);
        assert_eq!(parse_iso_utc("not a date"), None);
    }

    #[test]
    fn format_ladder() {
        let now = parse_iso_utc("2026-07-01T09:00:00Z").unwrap();
        let t = |iso: &str| format_when(parse_iso_utc(iso).unwrap(), now);
        assert_eq!(format_when(now - 30, now), "just now");
        assert_eq!(format_when(now + 100, now), "just now"); // future → clamp
        assert_eq!(format_when(now - 90, now), "1m ago");
        assert_eq!(t("2026-07-01T08:30:00Z"), "30m ago");
        assert_eq!(t("2026-07-01T04:00:00Z"), "5h ago");
        assert_eq!(t("2026-06-30T08:00:00Z"), "yesterday"); // 25h
        assert_eq!(t("2026-06-28T09:00:00Z"), "3d ago");
        assert_eq!(t("2026-03-08T10:00:00Z"), "Mar 8"); // same calendar year
        assert_eq!(t("2025-11-20T10:00:00Z"), "Nov 2025"); // previous year
    }

    #[test]
    fn when_passthrough_on_garbage() {
        assert_eq!(when("garbage"), "garbage");
    }
}
