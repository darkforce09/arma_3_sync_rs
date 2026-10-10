//! Human-readable formatting of sizes, speeds, durations and dates.
//!
//! [`size`], [`speed`] and [`duration_hms`] are character-identical to the Java client's
//! `utils/UnitConverter.java`, including its truncation and day-dropping quirks.
//! [`datetime_medium`] is a fixed, locale-independent replacement for Java's localized
//! `FormatStyle.MEDIUM` date-time format.

use std::fmt::Display;

use chrono::{DateTime, TimeZone};

/// Formats a byte count with decimal units, truncated to two decimals: `1130000` → `1.12 MB`.
///
/// Negative counts fall through to the byte branch (`-5` → `-5.0 B`).
// Mirrors utils/UnitConverter.java:7-28
pub fn size(bytes: i64) -> String {
    let bytes = bytes as f64;
    let (divisor, unit) = unit_for(bytes, "");
    let mut value = bytes / divisor;
    value = f64::from((value * 100.0) as i32) / 100.0;
    format!("{}{unit}", java_double_to_string(value))
}

/// Formats a transfer speed: one truncated decimal for GB/s and MB/s, an integer value printed
/// as a double for KB/s and B/s (`12345` → `12.0 KB/s`).
// Mirrors utils/UnitConverter.java:30-47
pub fn speed(bytes_per_sec: i64) -> String {
    let speed = bytes_per_sec as f64;
    let (divisor, unit) = unit_for(speed, "/s");
    let value = speed / divisor;
    let value = if divisor >= 1e6 {
        f64::from((value * 10.0) as i32) / 10.0
    } else {
        f64::from(value as i32)
    };
    format!("{}{unit}", java_double_to_string(value))
}

/// Selects the decimal unit for a byte count, as in `UnitConverter`.
fn unit_for(value: f64, suffix: &str) -> (f64, String) {
    let (divisor, unit) = if value >= 1e9 {
        (1e9, " GB")
    } else if value >= 1e6 {
        (1e6, " MB")
    } else if value >= 1e3 {
        (1e3, " KB")
    } else {
        (1.0, " B")
    };
    (divisor, format!("{unit}{suffix}"))
}

/// Formats a duration in seconds as `HH:MM:SS`.
///
/// Days are dropped (`86400` → `00:00:00`) and negative parts keep their sign (`-5` →
/// `00:00:-05`), as in Java. Seconds are cast to `i32` before the modulo, so they wrap above
/// `i32::MAX`.
// Mirrors utils/UnitConverter.java:49-57
pub fn duration_hms(seconds: i64) -> String {
    let secs = (seconds as i32) % 60;
    let minutes = ((seconds / 60) % 60) as i32;
    let hours = ((seconds / 3600) % 24) as i32;
    format!(
        "{}:{}:{}",
        two_digits(hours),
        two_digits(minutes),
        two_digits(secs)
    )
}

/// `DecimalFormat("00")`: at least two digits, with a leading `-` for negative values.
fn two_digits(value: i32) -> String {
    if value < 0 {
        format!("-{:02}", value.unsigned_abs())
    } else {
        format!("{value:02}")
    }
}

/// Formats a date and time as `Oct 10, 2026, 2:05:09 PM` (en-US medium style).
///
/// Java uses `DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM)` with the default locale
/// (`service/CommonService.java:26-27`); the Rust client uses this fixed format everywhere.
pub fn datetime_medium<Tz: TimeZone>(t: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    t.format("%b %-d, %Y, %-I:%M:%S %p").to_string()
}

/// Reproduces Java's `Double.toString`.
///
/// Prints the shortest digits that round-trip, always with a fractional part, in computerized
/// scientific notation (`1.0E7`, `1.0E-4`) when the magnitude is `>= 1e7` or `< 1e-3`.
pub fn java_double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_owned();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let magnitude = v.abs();
    if magnitude == 0.0 || (1e-3..1e7).contains(&magnitude) {
        // `Debug` prints the shortest round-trip digits without an exponent in this range.
        let mut out = format!("{v:?}");
        if !out.contains('.') {
            out.push_str(".0");
        }
        out
    } else {
        let formatted = format!("{v:e}");
        let (mantissa, exponent) = formatted
            .split_once('e')
            .expect("LowerExp output always contains an exponent");
        if mantissa.contains('.') {
            format!("{mantissa}E{exponent}")
        } else {
            format!("{mantissa}.0E{exponent}")
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    #[test]
    fn a_f1_size_table() {
        let table: &[(i64, &str)] = &[
            (0, "0.0 B"),
            (1, "1.0 B"),
            (999, "999.0 B"),
            (1000, "1.0 KB"),
            (1001, "1.0 KB"),
            (1499, "1.49 KB"),
            (1130, "1.12 KB"),
            (70_000, "70.0 KB"),
            (999_999, "999.99 KB"),
            (1_000_000, "1.0 MB"),
            (1_130_000, "1.12 MB"),
            (1_234_567, "1.23 MB"),
            (2_010_000, "2.0 MB"),
            (4_294_967_296, "4.29 GB"),
            (93_000_000_000, "93.0 GB"),
            (1_000_000_000_000, "1000.0 GB"),
            (-5, "-5.0 B"),
        ];
        for &(input, expected) in table {
            assert_eq!(size(input), expected, "size({input})");
        }
    }

    #[test]
    fn a_f2_speed_table() {
        let table: &[(i64, &str)] = &[
            (0, "0.0 B/s"),
            (999, "999.0 B/s"),
            (1000, "1.0 KB/s"),
            (1999, "1.0 KB/s"),
            (12_345, "12.0 KB/s"),
            (999_999, "999.0 KB/s"),
            (1_000_000, "1.0 MB/s"),
            (1_250_000, "1.2 MB/s"),
            (1_999_999, "1.9 MB/s"),
            (5_550_000_000, "5.5 GB/s"),
            (-1, "-1.0 B/s"),
        ];
        for &(input, expected) in table {
            assert_eq!(speed(input), expected, "speed({input})");
        }
    }

    #[test]
    fn a_f3_duration_table() {
        let table: &[(i64, &str)] = &[
            (0, "00:00:00"),
            (59, "00:00:59"),
            (60, "00:01:00"),
            (3599, "00:59:59"),
            (3600, "01:00:00"),
            (86_399, "23:59:59"),
            (86_400, "00:00:00"),
            (90_061, "01:01:01"),
            (-5, "00:00:-05"),
        ];
        for &(input, expected) in table {
            assert_eq!(duration_hms(input), expected, "duration_hms({input})");
        }
    }

    #[test]
    fn a_f4_java_double_to_string() {
        let table: &[(f64, &str)] = &[
            (1.0, "1.0"),
            (0.1, "0.1"),
            (123.45, "123.45"),
            (1.0e7, "1.0E7"),
            (21_474_836.47, "2.147483647E7"),
            (0.0001, "1.0E-4"),
            (-5.0, "-5.0"),
            (0.0, "0.0"),
            (0.001, "0.001"),
            (9_999_999.0, "9999999.0"),
            (1.5e-5, "1.5E-5"),
            (-2.5e10, "-2.5E10"),
        ];
        for &(input, expected) in table {
            assert_eq!(java_double_to_string(input), expected, "{input:?}");
        }
        assert_eq!(java_double_to_string(f64::NAN), "NaN");
        assert_eq!(java_double_to_string(f64::INFINITY), "Infinity");
        assert_eq!(java_double_to_string(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn a_f5_datetime_medium() {
        let offset = FixedOffset::east_opt(2 * 3600).expect("valid offset");
        let afternoon = offset
            .with_ymd_and_hms(2026, 10, 10, 14, 5, 9)
            .single()
            .expect("unambiguous");
        assert_eq!(datetime_medium(&afternoon), "Oct 10, 2026, 2:05:09 PM");
        let midnight = offset
            .with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
            .single()
            .expect("unambiguous");
        assert_eq!(datetime_medium(&midnight), "Jan 2, 2026, 12:00:00 AM");
    }
}
