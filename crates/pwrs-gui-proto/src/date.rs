//! Dates as they travel between the session and the renderers: ISO 8601
//! text with no zone, `YYYY-MM-DDTHH:MM` to the minute, or with seconds and
//! a fraction of a second, `YYYY-MM-DDTHH:MM:SS.f`, read as .NET reads a
//! date and time: in ticks of 100 nanoseconds since the start of year 1, on
//! the proleptic Gregorian calendar, from year 1 to 9999.

/// Ticks in a second.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Ticks in a day.
const TICKS_PER_DAY: i64 = 86_400 * TICKS_PER_SECOND;

/// Days from the Unix epoch back to the start of year 1, negative.
const DAYS_TO_YEAR_ONE: i64 = -719_162;

/// The ticks of `text`, ISO 8601 with no zone: `YYYY-MM-DDTHH:MM`,
/// optionally `:SS` and a fraction of a second of up to seven digits after
/// a dot.
pub fn parse(text: &str) -> Result<i64, String> {
    let bad = || format!("{text} is no date and time written as ISO 8601, YYYY-MM-DDTHH:MM with optional seconds");
    let (date, time) = text.split_once(['T', 't', ' ']).ok_or_else(bad)?;
    let mut ymd = date.splitn(3, '-');
    let year = number(ymd.next(), 4).ok_or_else(bad)?;
    let month = number(ymd.next(), 2).ok_or_else(bad)?;
    let day = number(ymd.next(), 2).ok_or_else(bad)?;
    let mut hms = time.splitn(3, ':');
    let hour = number(hms.next(), 2).ok_or_else(bad)?;
    let minute = number(hms.next(), 2).ok_or_else(bad)?;
    let (second, fraction) = match hms.next() {
        None => (0, 0),
        Some(rest) => {
            let (whole, part) = match rest.split_once('.') {
                Some((w, p)) => (w, Some(p)),
                None => (rest, None),
            };
            let second = number(Some(whole), 2).ok_or_else(bad)?;
            let fraction = match part {
                None => 0,
                Some(p) => {
                    let width = p.len();
                    let digits = number(Some(p), width).filter(|_| (1..=7).contains(&width)).ok_or_else(bad)?;
                    digits * 10_i64.pow((7 - width) as u32)
                }
            };
            (second, fraction)
        }
    };
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59 {
        return Err(format!("{text} names no date and time: the year runs from 1 to 9999, and each part must lie in its range"));
    }
    let days = days_from_civil(year, month, day) - DAYS_TO_YEAR_ONE;
    Ok(days * TICKS_PER_DAY + ((hour * 60 + minute) * 60 + second) * TICKS_PER_SECOND + fraction)
}

/// `ticks` written to the minute, `YYYY-MM-DDTHH:MM`.
pub fn minutes(ticks: i64) -> String {
    let (y, m, d, hh, mm, _, _) = parts(ticks);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}")
}

/// `ticks` written in full: to the second, and with the fraction of a
/// second when there is one, its trailing zeros dropped.
pub fn full(ticks: i64) -> String {
    let (y, m, d, hh, mm, ss, fraction) = parts(ticks);
    let mut out = format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}");
    if fraction > 0 {
        let digits = format!("{fraction:07}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
    out
}

/// Seconds from the Unix epoch to `ticks`, as a chart's time axis counts
/// them: the whole seconds and the fraction converted apart, so a date of
/// this era stays exact to a microsecond.
pub fn unix_seconds(ticks: i64) -> f64 {
    let since = ticks + DAYS_TO_YEAR_ONE * TICKS_PER_DAY;
    since.div_euclid(TICKS_PER_SECOND) as f64 + since.rem_euclid(TICKS_PER_SECOND) as f64 / TICKS_PER_SECOND as f64
}

/// The ticks of `seconds` from the Unix epoch, rounded to the tick, the
/// whole seconds and the fraction converted apart.
pub fn from_unix_seconds(seconds: f64) -> i64 {
    let whole = seconds.floor();
    let fraction = ((seconds - whole) * TICKS_PER_SECOND as f64).round() as i64;
    whole as i64 * TICKS_PER_SECOND + fraction - DAYS_TO_YEAR_ONE * TICKS_PER_DAY
}

/// A date and time to the minute, as a date chooser shows one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Minute {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
}

impl Minute {
    /// The minute `ticks` falls in.
    pub fn of(ticks: i64) -> Minute {
        let (year, month, day, hour, minute, _, _) = parts(ticks);
        Minute { year, month, day, hour, minute }
    }

    /// The minute `text` names, written as [`parse`] reads it.
    pub fn parse(text: &str) -> Result<Minute, String> {
        parse(text).map(Minute::of)
    }

    /// Written to the minute, `YYYY-MM-DDTHH:MM`; a day past its month's
    /// last is written as that last day.
    pub fn written(self) -> String {
        let day = self.day.min(days_in_month(self.year, self.month));
        format!("{:04}-{:02}-{:02}T{:02}:{:02}", self.year, self.month, day, self.hour, self.minute)
    }
}

/// The days of month `m` of year `y`.
pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The day of the week the first of month `m` of year `y` falls on, from 0
/// for Monday to 6 for Sunday.
pub fn first_weekday(y: i64, m: i64) -> i64 {
    (days_from_civil(y, m, 1) + 3).rem_euclid(7)
}

/// The year, month, day, hour, minute, second and ticks past the second of
/// `ticks`.
fn parts(ticks: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let days = ticks.div_euclid(TICKS_PER_DAY);
    let within = ticks.rem_euclid(TICKS_PER_DAY);
    let (y, m, d) = civil_from_days(days + DAYS_TO_YEAR_ONE);
    let seconds = within / TICKS_PER_SECOND;
    (y, m, d, seconds / 3600, seconds / 60 % 60, seconds % 60, within % TICKS_PER_SECOND)
}

/// The number a field of exactly `width` ASCII digits gives; none for a
/// field missing, of another width, or with anything else in it.
fn number(field: Option<&str>, width: usize) -> Option<i64> {
    let field = field?;
    if field.len() != width || !field.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(field.bytes().fold(0, |n, b| n * 10 + i64::from(b - b'0')))
}

/// Days from the Unix epoch to `y-m-d` (Howard Hinnant's days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after the Unix epoch (Howard Hinnant's civil_from_days).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ticks `text` reads as, or a failure naming why it reads as none.
    fn ticks(text: &str) -> i64 {
        match parse(text) {
            Ok(t) => t,
            Err(why) => panic!("{text} is a date and time: {why}"),
        }
    }

    #[test]
    fn dates_read_and_write_as_dotnet_counts_them() {
        assert_eq!(ticks("0001-01-01T00:00"), 0);
        assert_eq!(ticks("1970-01-01T00:00"), 621_355_968_000_000_000, "DateTime's ticks at the Unix epoch");
        assert_eq!(ticks("2026-10-08T14:30"), 639_270_666_000_000_000);
        assert_eq!(ticks("2024-02-29T23:59:59.5"), 638_448_479_995_000_000);
        assert_eq!(minutes(639_270_666_000_000_000), "2026-10-08T14:30");
        assert_eq!(full(638_448_479_995_000_000), "2024-02-29T23:59:59.5");
        assert_eq!(full(621_355_968_000_000_000), "1970-01-01T00:00:00");
        assert_eq!(unix_seconds(621_355_968_000_000_000), 0.0);
        assert_eq!(from_unix_seconds(1.5), 621_355_968_015_000_000);
        assert_eq!(unix_seconds(ticks("2026-10-08T14:31:00.25")), 1_791_469_860.25, "the seconds .NET counts from the Unix epoch");
        assert_eq!(from_unix_seconds(1_791_469_860.25), ticks("2026-10-08T14:31:00.25"));
        for (y, m, weekday) in [(2026, 10, 3), (2024, 2, 3), (1970, 1, 3), (2000, 3, 2)] {
            assert_eq!(first_weekday(y, m), weekday, "the first of {y}-{m:02} falls on the day .NET names");
        }
        assert_eq!(Minute { year: 2023, month: 2, day: 31, hour: 9, minute: 5 }.written(), "2023-02-28T09:05");
        for wrong in ["2023-02-29T00:00", "2026-13-01T00:00", "2026-10-08", "26-10-08T14:30", "2026-10-08T24:00", "2026-10-08T14:30:05.12345678", "2026-10-08T14:30:05."] {
            assert!(parse(wrong).is_err(), "{wrong} is refused");
        }
    }
}
