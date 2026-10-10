//! Adversarial checks of `a3s_format` against Java `utils/UnitConverter.java`.
//!
//! Expected values were computed with IEEE-754 doubles mirroring the Java code exactly
//! (`(int)` saturation for doubles, low-32-bit wrap for `(int)` of a `long`, `%` keeping the
//! sign of the dividend, `Double.toString` shortest digits with `E` notation outside
//! `[1e-3, 1e7)`).

use a3s_format::{datetime_medium, duration_hms, java_double_to_string, size, speed};
use chrono::{FixedOffset, TimeZone as _, Utc};

#[test]
fn verify_size_boundaries_and_extremes() {
    let table: &[(i64, &str)] = &[
        (999_999_999, "999.99 MB"),
        (1_000_000_000, "1.0 GB"),
        (999_999, "999.99 KB"),
        // 1.15 * 100.0 == 114.99999999999999 in doubles: Java prints 1.14.
        (1_150, "1.14 KB"),
        (1_005_000, "1.0 MB"),
        (1_015_000, "1.01 MB"),
        (1_290, "1.29 KB"),
        (1_100, "1.1 KB"),
        (1_009, "1.0 KB"),
        (4_350_000, "4.34 MB"),
        (2_147_483_647, "2.14 GB"),
        (2_147_483_648, "2.14 GB"),
        (8_589_934_592, "8.58 GB"),
        (123_456_789_012, "123.45 GB"),
        (10_995_116_277_760, "10995.11 GB"),
        (9_007_199_254_740_993, "9007199.25 GB"),
        // `(int)` saturates at i32::MAX: 21474836.47 printed in E notation.
        (21_474_836_470_000_000, "2.147483647E7 GB"),
        (21_474_836_480_000_000, "2.147483647E7 GB"),
        (i64::MAX, "2.147483647E7 GB"),
        // Negative values fall through to the byte branch and saturate at i32::MIN.
        (-1, "-1.0 B"),
        (-1000, "-1000.0 B"),
        (-999_999_999_999, "-2.147483648E7 B"),
        (i64::MIN, "-2.147483648E7 B"),
    ];
    for &(input, expected) in table {
        assert_eq!(size(input), expected, "size({input})");
    }
}

#[test]
fn verify_speed_boundaries_and_extremes() {
    let table: &[(i64, &str)] = &[
        (999_999_999, "999.9 MB/s"),
        (1_000_000_000, "1.0 GB/s"),
        (1_250_000_000, "1.2 GB/s"),
        (999_999_999_999, "999.9 GB/s"),
        (2_147_483_648_000, "2147.4 GB/s"),
        (214_748_364_700_000_000, "2.147483647E8 GB/s"),
        (i64::MAX, "2.147483647E8 GB/s"),
        (-1000, "-1000.0 B/s"),
        (i64::MIN, "-2.147483648E9 B/s"),
    ];
    for &(input, expected) in table {
        assert_eq!(speed(input), expected, "speed({input})");
    }
}

#[test]
fn verify_duration_wraps_like_java() {
    let table: &[(i64, &str)] = &[
        (359_999, "03:59:59"),
        (360_061, "04:01:01"),
        (2_147_483_647, "03:14:07"),
        // `(int) time` wraps to i32::MIN before `% 60`.
        (2_147_483_648, "03:14:-08"),
        (4_294_967_296, "06:28:00"),
        (4_294_967_356, "06:29:00"),
        (i64::MAX, "15:30:-01"),
        (i64::MIN, "-15:-30:00"),
        (-1, "00:00:-01"),
        (-60, "00:-01:00"),
        (-3600, "-01:00:00"),
        (-86_399, "-23:-59:-59"),
        (-90_061, "-01:-01:-01"),
    ];
    for &(input, expected) in table {
        assert_eq!(duration_hms(input), expected, "duration_hms({input})");
    }
}

#[test]
fn verify_java_double_to_string_edges() {
    let table: &[(f64, &str)] = &[
        (-0.0, "-0.0"),
        (0.001, "0.001"),
        (0.000_999_9, "9.999E-4"),
        (9_999_999.99, "9999999.99"),
        (10_000_000.0, "1.0E7"),
        (-21_474_836.48, "-2.147483648E7"),
        (214_748_364.7, "2.147483647E8"),
        (100.0, "100.0"),
        (1.0e21, "1.0E21"),
        (1.0e-300, "1.0E-300"),
        (f64::MAX, "1.7976931348623157E308"),
        (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
    ];
    for &(input, expected) in table {
        assert_eq!(java_double_to_string(input), expected, "{input:?}");
    }
}

#[test]
fn verify_datetime_medium_edges() {
    let utc = Utc
        .with_ymd_and_hms(2026, 12, 31, 12, 0, 0)
        .single()
        .expect("valid");
    assert_eq!(datetime_medium(&utc), "Dec 31, 2026, 12:00:00 PM");
    let offset = FixedOffset::west_opt(5 * 3600).expect("valid offset");
    let morning = offset
        .with_ymd_and_hms(2026, 5, 1, 9, 7, 3)
        .single()
        .expect("valid");
    // Formatted in the value's own offset, not converted to UTC.
    assert_eq!(datetime_medium(&morning), "May 1, 2026, 9:07:03 AM");
    let last_minute = offset
        .with_ymd_and_hms(2026, 9, 9, 23, 59, 59)
        .single()
        .expect("valid");
    assert_eq!(datetime_medium(&last_minute), "Sep 9, 2026, 11:59:59 PM");
}
