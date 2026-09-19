// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Prefixed identity newtypes shared across session and composition.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Stable identity for a composition across saves and sessions (`comp:<uuid>`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositionId(pub Uuid);

impl CompositionId {
    /// Mint a new random composition id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Build from a fixed u128 (tests).
    pub fn from_u128(n: u128) -> Self {
        Self(Uuid::from_u128(n))
    }
}

impl Default for CompositionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CompositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "comp:{}", self.0)
    }
}

impl fmt::Debug for CompositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CompositionId({self})")
    }
}

impl Serialize for CompositionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CompositionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for CompositionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_prefixed_uuid("comp", s).map(Self)
    }
}

/// Parse `prefix:uuid` or accept a bare uuid for transitional test fixtures.
fn parse_prefixed_uuid(prefix: &str, s: &str) -> Result<Uuid, String> {
    let s = s.trim();
    let uuid_text = if let Some(rest) = s.strip_prefix(&format!("{prefix}:")) {
        rest
    } else if s.contains(':') {
        return Err(format!("expected `{prefix}:<uuid>`, got `{s}`"));
    } else {
        // Bare uuid only for tests / accidental hand edits; prefer prefixed form.
        s
    };
    Uuid::parse_str(uuid_text).map_err(|err| format!("invalid {prefix} uuid `{s}`: {err}"))
}

/// Serialize a UUID newtype as `prefix:uuid`.
pub fn serialize_prefixed_uuid<S: Serializer>(
    prefix: &str,
    id: &Uuid,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{prefix}:{id}"))
}

/// Deserialize a UUID newtype from `prefix:uuid` (or bare uuid).
pub fn deserialize_prefixed_uuid<'de, D: Deserializer<'de>>(
    prefix: &str,
    deserializer: D,
) -> Result<Uuid, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_prefixed_uuid(prefix, &text).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_id_round_trips_prefixed() {
        let id = CompositionId::from_u128(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"comp:00000000-0000-0000-0000-00000000002a\"");
        let parsed: CompositionId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(id.to_string(), "comp:00000000-0000-0000-0000-00000000002a");
    }
}
