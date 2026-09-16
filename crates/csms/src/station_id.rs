//! The identity a charging station connects with (the last path segment of its URL).

use std::fmt;

/// Longest station identity accepted.
// TODO(spec): 48 characters is the identity limit as we read OCPP 2.0.1 Part 4; confirm
// it, and whether the allowed character set should be restricted too.
pub const MAX_STATION_ID_LEN: usize = 48;

/// A charging station identity: 1 to [`MAX_STATION_ID_LEN`] characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StationId(String);

impl StationId {
    /// The identity as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Returned when a station identity is empty or too long.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("station id must be 1 to {MAX_STATION_ID_LEN} characters, got {0}")]
pub struct InvalidStationId(pub usize);

impl TryFrom<String> for StationId {
    type Error = InvalidStationId;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let len = id.chars().count();
        if (1..=MAX_STATION_ID_LEN).contains(&len) {
            Ok(Self(id))
        } else {
            Err(InvalidStationId(len))
        }
    }
}

impl TryFrom<&str> for StationId {
    type Error = InvalidStationId;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        Self::try_from(id.to_owned())
    }
}

impl fmt::Display for StationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ids_up_to_the_limit() {
        assert!(StationId::try_from("CS-001").is_ok());
        assert!(StationId::try_from("é".repeat(MAX_STATION_ID_LEN)).is_ok());
    }

    #[test]
    fn rejects_empty_and_long_ids() {
        assert_eq!(StationId::try_from(""), Err(InvalidStationId(0)));
        let long = "a".repeat(MAX_STATION_ID_LEN + 1);
        assert_eq!(
            StationId::try_from(long),
            Err(InvalidStationId(MAX_STATION_ID_LEN + 1))
        );
    }
}
