//! Minimal date-time helpers replacing `glib::DateTime`: strict-ish
//! ISO-8601 parsing to unix seconds and UTC/local civil formatting.
//! No network, no locale — the only tz input is the OS local offset.

/// Days-from-civil (Howard Hinnant's algorithm): days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`: returns (year, month 1-12, day 1-31).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Decompose unix seconds into (year, month, day, hour, min, sec) UTC.
fn utc_civil(secs: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    Some((y, m, d, rem / 3600, rem % 3600 / 60, rem % 60))
}

fn parse_ndigits(bytes: &[u8], pos: usize, n: usize) -> Option<(i64, usize)> {
    let s = bytes.get(pos..pos + n)?;
    if s.iter().all(u8::is_ascii_digit) {
        Some((
            s.iter().fold(0i64, |v, b| v * 10 + (b - b'0') as i64),
            pos + n,
        ))
    } else {
        None
    }
}

/// Parse `YYYY-MM-DD[T ]HH:MM[:SS[.frac]][Z|±HH[:MM]]` into unix seconds.
/// A missing or unparseable timezone yields None — callers must not guess
/// the machine's local tz. A bare date (`YYYY-MM-DD`) is midnight UTC only
/// when the trailing offset is explicitly present, which never happens for
/// a bare date, so it also yields None.
pub fn parse_iso8601_unix(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    let (y, mut p) = parse_ndigits(b, 0, 4)?;
    if *b.get(p)? != b'-' {
        return None;
    }
    let (m, np) = parse_ndigits(b, p + 1, 2)?;
    p = np;
    if *b.get(p)? != b'-' {
        return None;
    }
    let (d, np) = parse_ndigits(b, p + 1, 2)?;
    p = np;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    match b.get(p) {
        None => return None, // date-only: no explicit zone
        Some(&c) if c == b'T' || c == b't' || c == b' ' => p += 1,
        Some(_) => return None,
    }
    let (hh, np) = parse_ndigits(b, p, 2)?;
    p = np;
    if *b.get(p)? != b':' {
        return None;
    }
    let (mm, np) = parse_ndigits(b, p + 1, 2)?;
    p = np;
    let mut ss = 0i64;
    if b.get(p) == Some(&b':') {
        let (v, np) = parse_ndigits(b, p + 1, 2)?;
        ss = v;
        p = np;
        // Fractional seconds are accepted and truncated.
        if b.get(p) == Some(&b'.') {
            p += 1;
            while b.get(p).is_some_and(u8::is_ascii_digit) {
                p += 1;
            }
        }
    }
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Timezone: Z/z or ±HH[:MM].
    let off = match b.get(p) {
        Some(&c) if c == b'Z' || c == b'z' => {
            p += 1;
            0i64
        }
        Some(&sign) if sign == b'+' || sign == b'-' => {
            let (oh, np) = parse_ndigits(b, p + 1, 2)?;
            p = np;
            let mut om = 0i64;
            if b.get(p) == Some(&b':') {
                let (v, np) = parse_ndigits(b, p + 1, 2)?;
                om = v;
                p = np;
            } else if let Some(&c1) = b.get(p) {
                // ±HHMM without a colon.
                if c1.is_ascii_digit() {
                    let (v, np) = parse_ndigits(b, p, 2)?;
                    om = v;
                    p = np;
                }
            }
            if oh > 23 || om > 59 {
                return None;
            }
            let v = oh * 3600 + om * 60;
            if sign == b'-' {
                -v
            } else {
                v
            }
        }
        _ => return None,
    };
    if p != b.len() {
        return None; // trailing garbage
    }
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - off)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for unix seconds (UTC).
pub fn format_utc(secs: i64) -> Option<String> {
    let (y, m, d, hh, mm, ss) = utc_civil(secs)?;
    if !(0..=9999).contains(&y) {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"))
}

/// Civil (y, m, d, hh, mm, ss) of unix seconds in the OS local timezone.
#[cfg(unix)]
fn local_civil(secs: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = secs;
    // localtime_r never fails for in-range values; a null means out of range.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some((
        tm.tm_year as i64 + 1900,
        tm.tm_mon as i64 + 1,
        tm.tm_mday as i64,
        tm.tm_hour as i64,
        tm.tm_min as i64,
        tm.tm_sec as i64,
    ))
}

#[cfg(windows)]
fn local_civil(secs: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // unix seconds -> FILETIME (100ns ticks since 1601-01-01).
    let ticks = secs
        .checked_mul(10_000_000)?
        .checked_add(116_444_736_000_000_000)?;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&ft, &mut utc) } == 0 {
        return None;
    }
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) } == 0 {
        return None;
    }
    Some((
        local.wYear as i64,
        local.wMonth as i64,
        local.wDay as i64,
        local.wHour as i64,
        local.wMinute as i64,
        local.wSecond as i64,
    ))
}

/// `%d.%m %H:%M` in local time for unix seconds.
pub fn format_local(secs: i64) -> Option<String> {
    let (y, m, d, hh, mm, _) = local_civil(secs)?;
    if !(0..=9999).contains(&y) {
        return None;
    }
    Some(format!("{d:02}.{m:02} {hh:02}:{mm:02}"))
}

/// `%d.%m %H:%M` local time for an ISO-8601 timestamp; None when the text
/// is not ISO-8601 with an explicit zone (raw strings never reach the UI).
pub fn format_local_iso(text: &str) -> Option<String> {
    format_local(parse_iso8601_unix(text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_civil_roundtrip() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2026, 11, 15),
            (2000, 2, 29),
            (1900, 3, 1),
            (1, 1, 1),
        ] {
            let z = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(z), (y, m, d), "{y}-{m}-{d}");
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn parse_iso8601_offsets() {
        assert_eq!(parse_iso8601_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso8601_unix("2026-11-15T08:00:00Z"),
            Some(1_794_729_600)
        );
        assert_eq!(
            parse_iso8601_unix("2026-11-15T11:00:00+03:00"),
            parse_iso8601_unix("2026-11-15T08:00:00Z")
        );
        assert_eq!(
            parse_iso8601_unix("2026-11-15T07:00:00-01:00"),
            parse_iso8601_unix("2026-11-15T08:00:00Z")
        );
        assert_eq!(
            parse_iso8601_unix("2026-11-15T11:00:00+0300"),
            parse_iso8601_unix("2026-11-15T08:00:00Z")
        );
        // Fractional seconds and missing seconds are tolerated.
        assert_eq!(
            parse_iso8601_unix("2026-11-15T08:00:00.500Z"),
            Some(1_794_729_600)
        );
        assert_eq!(parse_iso8601_unix("2026-11-15T08:00Z"), Some(1_794_729_600));
    }

    #[test]
    fn parse_iso8601_rejects_no_zone_and_garbage() {
        for bad in [
            "2026-11-15T08:00:00", // no zone: must not guess local
            "2026-11-15",          // bare date
            "2026-11-15 08:00:00Zx",
            "2026-13-15T08:00:00Z",
            "2026-11-32T08:00:00Z",
            "2026-11-15T25:00:00Z",
            "garbage",
            "",
            "2026/11/15T08:00:00Z",
            "2026-11-15T08:00:00+25:00",
        ] {
            assert_eq!(parse_iso8601_unix(bad), None, "{bad}");
        }
    }

    #[test]
    fn format_utc_iso() {
        assert_eq!(
            format_utc(1_794_729_600).as_deref(),
            Some("2026-11-15T08:00:00Z")
        );
        assert_eq!(format_utc(0).as_deref(), Some("1970-01-01T00:00:00Z"));
    }

    #[test]
    fn format_local_smoke() {
        // Local rendering must match UTC text shifted by the local offset;
        // exact zone is machine-dependent, so assert the shape instead.
        let s = format_local(1_794_729_600).unwrap();
        let b = s.as_bytes();
        assert_eq!(s.len(), 11);
        assert!(b[2] == b'.' && b[5] == b' ' && b[8] == b':');
        assert_eq!(format_local_iso("not-a-date"), None);
    }
}
