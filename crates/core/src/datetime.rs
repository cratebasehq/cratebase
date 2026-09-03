//! PocketBase-formatted timestamps: `2026-09-03 12:44:06.146Z` (space
//! separator, millisecond precision, literal `Z`). Parsing is lenient so
//! clients may send RFC3339 or date-only values.

use std::fmt;
use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3fZ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(pub chrono::DateTime<Utc>);

impl DateTime {
    pub fn now() -> Self {
        DateTime(Utc::now())
    }

    pub fn from_utc(dt: chrono::DateTime<Utc>) -> Self {
        DateTime(dt)
    }

    pub fn inner(&self) -> chrono::DateTime<Utc> {
        self.0
    }

    /// Parse any of: PocketBase format, RFC3339 (`T`, offsets, `Z`),
    /// `YYYY-MM-DD HH:MM:SS[.fff]`, `YYYY-MM-DD`. Empty string is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(DateTime(dt.with_timezone(&Utc)));
        }
        let trimmed = s.strip_suffix('Z').unwrap_or(s);
        for fmt in [
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M",
        ] {
            if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, fmt) {
                return Some(DateTime(Utc.from_utc_datetime(&naive)));
            }
        }
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
            return Some(DateTime(
                Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap()),
            ));
        }
        None
    }

    pub fn to_pb_string(&self) -> String {
        self.0.format(FORMAT).to_string()
    }

    pub fn to_rfc3339(&self) -> String {
        self.0
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_pb_string())
    }
}

impl FromStr for DateTime {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DateTime::parse(s).ok_or_else(|| format!("invalid datetime: {s:?}"))
    }
}

impl Serialize for DateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_pb_string())
    }
}

impl<'de> Deserialize<'de> for DateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        DateTime::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid datetime {s:?}")))
    }
}

impl Default for DateTime {
    fn default() -> Self {
        DateTime(Utc.timestamp_opt(0, 0).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_pocketbase() {
        let dt = DateTime::parse("2026-09-03T12:44:06.146Z").unwrap();
        assert_eq!(dt.to_string(), "2026-09-03 12:44:06.146Z");
        assert_eq!(DateTime::parse(&dt.to_string()).unwrap(), dt);
    }

    #[test]
    fn parses_lenient_inputs() {
        assert!(DateTime::parse("2026-09-03").is_some());
        assert!(DateTime::parse("2026-09-03 12:44:06").is_some());
        assert!(DateTime::parse("2026-09-03 12:44:06.146Z").is_some());
        assert!(DateTime::parse("2026-09-03T12:44:06+07:00").is_some());
        assert!(DateTime::parse("").is_none());
        assert!(DateTime::parse("nope").is_none());
    }

    #[test]
    fn serde_round_trip() {
        let dt = DateTime::parse("2026-09-03 12:44:06.146Z").unwrap();
        let json = serde_json::to_string(&dt).unwrap();
        assert_eq!(json, "\"2026-09-03 12:44:06.146Z\"");
        let back: DateTime = serde_json::from_str(&json).unwrap();
        assert_eq!(back, dt);
    }
}
