//! Note identifiers.
//!
//! A [`NoteId`] is 32 lowercase hex characters (128 bits). Ids are generated
//! from the current time in nanoseconds, the process id and a process-wide
//! atomic counter, run through `std::collections::hash_map::RandomState`
//! (which is itself randomly seeded per process by the standard library). No
//! external RNG crate is needed and ids are never reused.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Number of hex characters in a textual note id.
pub const ID_LEN: usize = 32;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique, validated note id.
///
/// The inner string is always [`ID_LEN`] lowercase hex characters; the only
/// ways to build one are [`NoteId::generate`] and [`NoteId::parse`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(String);

impl NoteId {
    /// Generates a fresh id. Never returns the same value twice.
    pub fn generate() -> Self {
        let nanos = now_nanos();
        let pid = u64::from(std::process::id());
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

        // Two independently seeded hashers give us 128 bits of id.
        let high = {
            let mut hasher = RandomState::new().build_hasher();
            nanos.hash(&mut hasher);
            pid.hash(&mut hasher);
            counter.hash(&mut hasher);
            hasher.finish()
        };
        let low = {
            let mut hasher = RandomState::new().build_hasher();
            counter.hash(&mut hasher);
            pid.hash(&mut hasher);
            nanos.hash(&mut hasher);
            0x9e37_79b9_7f4a_7c15_u64.hash(&mut hasher);
            hasher.finish()
        };

        NoteId(format!("{high:016x}{low:016x}"))
    }

    /// Parses and validates a textual id: exactly 32 lowercase hex characters.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != ID_LEN {
            return None;
        }
        if !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        Some(NoteId(text.to_owned()))
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NoteId({})", self.0)
    }
}

impl AsRef<str> for NoteId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for NoteId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for NoteId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        NoteId::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid note id: {text:?}")))
    }
}

/// Seconds since the unix epoch (saturating at 0 on clock trouble).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Nanoseconds since the unix epoch (0 on clock trouble).
pub fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_valid_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = NoteId::generate();
            assert_eq!(id.as_str().len(), ID_LEN);
            assert!(NoteId::parse(id.as_str()).is_some());
            assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(id.as_str().chars().all(|c| !c.is_ascii_uppercase()));
            assert!(seen.insert(id), "generated a duplicate id");
        }
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(NoteId::parse("").is_none());
        assert!(NoteId::parse("ABC").is_none());
        assert!(NoteId::parse(&"a".repeat(31)).is_none());
        assert!(NoteId::parse(&"a".repeat(33)).is_none());
        assert!(NoteId::parse(&"A".repeat(32)).is_none());
        assert!(NoteId::parse(&"g".repeat(32)).is_none());
        assert!(NoteId::parse(&"a".repeat(32)).is_some());
        assert!(NoteId::parse("0123456789abcdef0123456789abcdef").is_some());
    }
}
