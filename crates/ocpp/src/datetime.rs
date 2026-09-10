//! The `date-time` format used by OCPP 2.0.1 timestamps.

use std::fmt;
use std::ops::Deref;

use chrono::{SecondsFormat, SubsecRound, Utc};
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use crate::error::BAD_DATE_TIME_PREFIX;

/// A UTC timestamp with at most millisecond precision.
///
/// OCPP 2.0.1 formats times as RFC 3339 and limits fractional seconds to three digits.
/// Any UTC offset is accepted on input; output always uses `Z`, with no fraction when the
/// milliseconds are zero and exactly three digits otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateTime(chrono::DateTime<Utc>);

impl DateTime {
    /// The current time, truncated to milliseconds.
    pub fn now() -> Self {
        Utc::now().into()
    }

    /// The inner chrono timestamp.
    pub fn into_inner(self) -> chrono::DateTime<Utc> {
        self.0
    }
}

impl From<chrono::DateTime<Utc>> for DateTime {
    fn from(t: chrono::DateTime<Utc>) -> Self {
        Self(t.trunc_subsecs(3))
    }
}

impl Deref for DateTime {
    type Target = chrono::DateTime<Utc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
}

impl Serialize for DateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        chrono::DateTime::parse_from_rfc3339(&s)
            .map(|t| t.with_timezone(&Utc).into())
            .map_err(|e| de::Error::custom(format!("{BAD_DATE_TIME_PREFIX} `{s}`: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCode, decode};
    use serde_json::json;

    fn round_trip(s: &str) -> String {
        let t: DateTime = decode(json!(s)).unwrap();
        serde_json::to_value(t)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn keeps_whole_seconds_and_millis() {
        assert_eq!(round_trip("2026-09-28T10:00:00Z"), "2026-09-28T10:00:00Z");
        assert_eq!(
            round_trip("2026-09-28T10:00:00.250Z"),
            "2026-09-28T10:00:00.250Z"
        );
    }

    #[test]
    fn normalizes_offsets_and_extra_precision() {
        assert_eq!(
            round_trip("2026-09-28T12:00:00+02:00"),
            "2026-09-28T10:00:00Z"
        );
        assert_eq!(
            round_trip("2026-09-28T10:00:00.123456Z"),
            "2026-09-28T10:00:00.123Z"
        );
    }

    #[test]
    fn invalid_date_time_is_property_violation() {
        for s in ["2026-09-28 10:00", "2026-13-01T00:00:00Z", "yesterday"] {
            let err = decode::<DateTime>(json!(s)).unwrap_err();
            assert_eq!(err.code, ErrorCode::PropertyConstraintViolation, "{s}");
        }
        let err = decode::<DateTime>(json!(12)).unwrap_err();
        assert_eq!(err.code, ErrorCode::TypeConstraintViolation);
    }

    #[test]
    fn now_has_millisecond_precision() {
        assert_eq!(DateTime::now().timestamp_subsec_nanos() % 1_000_000, 0);
    }
}
