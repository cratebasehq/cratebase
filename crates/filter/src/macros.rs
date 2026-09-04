//! PocketBase's date macros (`@now`, `@todayStart`, `@month`, ...). All
//! values are computed in UTC at compile time and bound as parameters:
//! datetimes in PocketBase's `YYYY-MM-DD HH:MM:SS.sssZ` format, the
//! calendar components as integers.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use serde_json::{json, Value};

fn pb(dt: DateTime<Utc>) -> Value {
    Value::String(cratebase_core::DateTime::from_utc(dt).to_pb_string())
}

fn day_start(d: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).expect("valid time"))
}

fn day_end(d: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&d.and_hms_milli_opt(23, 59, 59, 999).expect("valid time"))
}

fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).expect("day 1 exists")
}

fn month_end(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1).expect("valid date") - Duration::days(1)
}

/// The value of a date macro (name without the leading `@`), or `None` if
/// `name` is not a date macro.
pub fn date_macro(name: &str, now: DateTime<Utc>) -> Option<Value> {
    let today = now.date_naive();
    let v = match name {
        "now" => pb(now),
        "second" => json!(now.second()),
        "minute" => json!(now.minute()),
        "hour" => json!(now.hour()),
        // 0 = Sunday ... 6 = Saturday, like Go's time.Weekday.
        "weekday" => json!(now.weekday().num_days_from_sunday()),
        "day" => json!(now.day()),
        "month" => json!(now.month()),
        "year" => json!(now.year()),
        "yesterday" => pb(now - Duration::days(1)),
        "tomorrow" => pb(now + Duration::days(1)),
        "todayStart" => pb(day_start(today)),
        "todayEnd" => pb(day_end(today)),
        "monthStart" => pb(day_start(month_start(today))),
        "monthEnd" => pb(day_end(month_end(today))),
        "yearStart" => pb(day_start(
            NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("valid date"),
        )),
        "yearEnd" => pb(day_end(
            NaiveDate::from_ymd_opt(today.year(), 12, 31).expect("valid date"),
        )),
        _ => return None,
    };
    Some(v)
}

/// All date macro names, for diagnostics.
pub const DATE_MACROS: &[&str] = &[
    "now",
    "second",
    "minute",
    "hour",
    "weekday",
    "day",
    "month",
    "year",
    "yesterday",
    "tomorrow",
    "todayStart",
    "todayEnd",
    "monthStart",
    "monthEnd",
    "yearStart",
    "yearEnd",
];
