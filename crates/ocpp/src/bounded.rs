//! Strings and arrays with the length limits the OCPP JSON schemas declare.

use std::fmt;
use std::ops::Deref;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use crate::error::TOO_LONG_PREFIX;

/// A string of at most `N` characters (the schema's `maxLength`, counted in code points).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BoundedString<const N: usize>(String);

/// Returned when a string is longer than its `maxLength`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{TOO_LONG_PREFIX} {max}: got {len} characters")]
pub struct StringTooLong {
    pub max: usize,
    pub len: usize,
}

impl<const N: usize> BoundedString<N> {
    /// Checks the length and wraps the string.
    pub fn new(s: impl Into<String>) -> Result<Self, StringTooLong> {
        let s = s.into();
        let len = s.chars().count();
        if len <= N {
            Ok(Self(s))
        } else {
            Err(StringTooLong { max: N, len })
        }
    }

    /// The string as a slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Unwraps the inner string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl<const N: usize> Deref for BoundedString<N> {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> fmt::Display for BoundedString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const N: usize> TryFrom<String> for BoundedString<N> {
    type Error = StringTooLong;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl<const N: usize> TryFrom<&str> for BoundedString<N> {
    type Error = StringTooLong;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl<const N: usize> Serialize for BoundedString<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const N: usize> Deserialize<'de> for BoundedString<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(de::Error::custom)
    }
}

/// An array with `MIN..=MAX` items (the schema's `minItems` and `maxItems`).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedVec<T, const MIN: usize, const MAX: usize>(Vec<T>);

/// An array with at least one item and no upper bound.
pub type NonEmptyVec<T> = BoundedVec<T, 1, { usize::MAX }>;

/// Returned when an array has too few or too many items.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected {min} to {max} items, got {len}")]
pub struct BadLength {
    pub min: usize,
    pub max: usize,
    pub len: usize,
}

impl<T, const MIN: usize, const MAX: usize> BoundedVec<T, MIN, MAX> {
    /// Checks the length and wraps the items.
    pub fn new(items: Vec<T>) -> Result<Self, BadLength> {
        if (MIN..=MAX).contains(&items.len()) {
            Ok(Self(items))
        } else {
            Err(BadLength {
                min: MIN,
                max: MAX,
                len: items.len(),
            })
        }
    }

    /// Unwraps the inner vector.
    pub fn into_inner(self) -> Vec<T> {
        self.0
    }
}

impl<T, const MIN: usize, const MAX: usize> Deref for BoundedVec<T, MIN, MAX> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T, const MIN: usize, const MAX: usize> TryFrom<Vec<T>> for BoundedVec<T, MIN, MAX> {
    type Error = BadLength;

    fn try_from(items: Vec<T>) -> Result<Self, Self::Error> {
        Self::new(items)
    }
}

impl<T: Serialize, const MIN: usize, const MAX: usize> Serialize for BoundedVec<T, MIN, MAX> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T, const MIN: usize, const MAX: usize> Deserialize<'de> for BoundedVec<T, MIN, MAX>
where
    T: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Expected(usize, usize);

        impl de::Expected for Expected {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.1 {
                    usize::MAX => write!(f, "at least {} items", self.0),
                    max => write!(f, "{} to {max} items", self.0),
                }
            }
        }

        let items = Vec::<T>::deserialize(deserializer)?;
        let len = items.len();
        Self::new(items).map_err(|_| de::Error::invalid_length(len, &Expected(MIN, MAX)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCode, decode};
    use serde_json::json;

    #[test]
    fn string_limit_counts_characters() {
        assert!(BoundedString::<3>::new("abc").is_ok());
        assert!(BoundedString::<3>::new("àèì").is_ok());
        assert_eq!(
            BoundedString::<3>::new("abcd"),
            Err(StringTooLong { max: 3, len: 4 })
        );
        assert!(BoundedString::<3>::new("").is_ok());
    }

    #[test]
    fn too_long_string_is_property_violation() {
        let err = decode::<BoundedString<3>>(json!("abcd")).unwrap_err();
        assert_eq!(err.code, ErrorCode::PropertyConstraintViolation);
        assert_eq!(
            decode::<BoundedString<3>>(json!("abc")).unwrap().as_str(),
            "abc"
        );
    }

    #[test]
    fn vec_limits_are_checked() {
        assert!(BoundedVec::<u8, 1, 3>::new(vec![]).is_err());
        assert!(BoundedVec::<u8, 1, 3>::new(vec![1, 2, 3]).is_ok());
        assert!(BoundedVec::<u8, 1, 3>::new(vec![1, 2, 3, 4]).is_err());
        assert!(NonEmptyVec::<u8>::new(vec![0; 5000]).is_ok());
    }

    #[test]
    fn bad_vec_length_is_occurrence_violation() {
        for payload in [json!([]), json!([1, 2, 3, 4])] {
            let err = decode::<BoundedVec<u8, 1, 3>>(payload).unwrap_err();
            assert_eq!(err.code, ErrorCode::OccurrenceConstraintViolation);
        }
        let err = decode::<NonEmptyVec<u8>>(json!([])).unwrap_err();
        assert_eq!(err.message, "invalid length 0, expected at least 1 items");
    }

    #[test]
    fn serializes_transparently() {
        let s = BoundedString::<5>::new("hi").unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), json!("hi"));
        let v = BoundedVec::<u8, 1, 3>::new(vec![1, 2]).unwrap();
        assert_eq!(serde_json::to_value(&v).unwrap(), json!([1, 2]));
    }
}
