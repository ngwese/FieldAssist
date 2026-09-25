// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Portable file locations as relative URLs, `file://`, or `memory://`.
//!
//! Relative URLs in session and composition files keep projects portable
//! across Windows, macOS, and Linux. Absolute filesystem refs are stored only
//! as `file://…` — never as platform-native path strings.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A document or media location in portable URL form.
///
/// Stored forms:
/// - relative path with `/` separators (`takes/a.wav`)
/// - absolute `file://…`
/// - `memory://…` for in-memory / synthetic media
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    raw: String,
}

impl Location {
    /// Parse a path, `file://`, relative URL, or `memory://` string.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("empty file URL");
        }
        Ok(Self {
            raw: input.to_owned(),
        })
    }

    /// Encode a filesystem path as an absolute `file://` URL (or keep `memory://`).
    pub fn from_path(path: &Path) -> Self {
        Self {
            raw: encode_absolute(path),
        }
    }

    /// Prefer a relative URL when `path` is under `base`; otherwise absolute `file://`.
    pub fn from_path_relative_to(path: &Path, base: &Path) -> Self {
        let lossy = path.to_string_lossy();
        if lossy.starts_with("memory://") {
            return Self {
                raw: lossy.into_owned(),
            };
        }
        if let Some(rel) = relative_to(path, base) {
            return Self { raw: rel };
        }
        Self::from_path(path)
    }

    /// Serialize form used on disk and in Lua (`tostring`).
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// True when this is an in-memory / synthetic location.
    pub fn is_memory(&self) -> bool {
        self.raw.starts_with("memory://")
    }

    /// True when the stored form is a relative URL (not `file://` / `memory://` / absolute path).
    pub fn is_relative(&self) -> bool {
        if self.is_memory() || self.raw.starts_with("file://") {
            return false;
        }
        !Path::new(&self.raw).is_absolute()
    }

    /// Resolve to a filesystem path.
    ///
    /// Relative locations require `base` (normally the parent of the owning
    /// `.fasession` / `.facomp`). Relative locations are never resolved against
    /// the process working directory.
    pub fn resolve(&self, base: Option<&Location>) -> Result<PathBuf> {
        let url = self.raw.trim();
        if url.is_empty() {
            bail!("empty file URL");
        }
        if url.starts_with("memory://") {
            return Ok(PathBuf::from(url));
        }
        if let Some(rest) = url.strip_prefix("file://") {
            let decoded = percent_decode(rest).context("invalid percent-encoding in file URL")?;
            return Ok(path_from_file_url_body(&decoded));
        }
        let path = PathBuf::from(url);
        if path.is_absolute() {
            return Ok(path);
        }
        match base {
            Some(base) => {
                let base_path = base
                    .resolve(None)
                    .with_context(|| format!("invalid base location for relative URL {url}"))?;
                Ok(base_path.join(url))
            }
            None => bail!("relative URL `{url}` requires a document base location"),
        }
    }

    /// Native filesystem path for absolute / `file://` / `memory://` locations.
    ///
    /// Relative locations require [`Self::resolve`] with a base.
    pub fn as_path(&self) -> Result<PathBuf> {
        if self.is_relative() {
            bail!(
                "relative URL `{}` requires a document base location",
                self.raw
            );
        }
        self.resolve(None)
    }

    /// Resolve against an optional filesystem directory base (document parent).
    ///
    /// Relative locations join onto `base` without canonicalizing `base`, so
    /// round-trips match the path the caller supplied (important on macOS where
    /// `/var` resolves to `/private/var`).
    pub fn resolve_against_path(&self, base: Option<&Path>) -> Result<PathBuf> {
        let url = self.raw.trim();
        if url.is_empty() {
            bail!("empty file URL");
        }
        if url.starts_with("memory://") {
            return Ok(PathBuf::from(url));
        }
        if let Some(rest) = url.strip_prefix("file://") {
            let decoded = percent_decode(rest).context("invalid percent-encoding in file URL")?;
            return Ok(path_from_file_url_body(&decoded));
        }
        let path = PathBuf::from(url);
        if path.is_absolute() {
            return Ok(path);
        }
        match base {
            Some(base) => Ok(base.join(url)),
            None => bail!("relative URL `{url}` requires a document base location"),
        }
    }

    /// Parent directory as a location (filesystem URLs only).
    pub fn parent(&self) -> Result<Location> {
        if self.is_relative() {
            let parent = Path::new(&self.raw)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| anyhow::anyhow!("URL has no parent"))?;
            return Location::parse(&parent.to_string_lossy().replace('\\', "/"));
        }
        let path = self.as_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("URL has no parent"))?;
        Ok(Location::from_path(parent))
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for Location {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl AsRef<str> for Location {
    fn as_ref(&self) -> &str {
        &self.raw
    }
}

impl From<&str> for Location {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            return Self {
                raw: "memory://untitled".into(),
            };
        }
        Self::parse(value).unwrap_or_else(|_| Self {
            raw: value.to_owned(),
        })
    }
}

impl From<String> for Location {
    fn from(value: String) -> Self {
        Location::from(value.as_str())
    }
}

impl Serialize for Location {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Location {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Location::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn encode_absolute(path: &Path) -> String {
    let lossy = path.to_string_lossy();
    if lossy.starts_with("memory://") {
        return lossy.into_owned();
    }
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn relative_to(path: &Path, base: &Path) -> Option<String> {
    if let Ok(rel) = path.strip_prefix(base) {
        if !rel.as_os_str().is_empty() {
            return Some(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    let canon_path = path.canonicalize().ok()?;
    let canon_base = base.canonicalize().ok()?;
    let rel = canon_path.strip_prefix(canon_base).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn path_from_file_url_body(rest: &str) -> PathBuf {
    if cfg!(windows) {
        let trimmed = rest.trim_start_matches('/');
        if trimmed.len() >= 2 && trimmed.as_bytes().get(1) == Some(&b':') {
            PathBuf::from(trimmed.replace('/', "\\"))
        } else {
            PathBuf::from(rest.replace('/', "\\"))
        }
    } else if rest.starts_with('/') {
        PathBuf::from(rest)
    } else {
        PathBuf::from(format!("/{rest}"))
    }
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_url_round_trips() {
        let base = Location::from_path(Path::new("/sessions"));
        let path = Path::new("/sessions/takes/a.wav");
        let url = Location::from_path_relative_to(path, Path::new("/sessions"));
        assert_eq!(url.as_str(), "takes/a.wav");
        let resolved = url.resolve(Some(&base)).unwrap();
        assert_eq!(resolved, PathBuf::from("/sessions/takes/a.wav"));
    }

    #[test]
    fn relative_without_base_errors() {
        let url = Location::parse("takes/a.wav").unwrap();
        let err = url.resolve(None).unwrap_err().to_string();
        assert!(err.contains("requires a document base"), "{err}");
        let err = url.as_path().unwrap_err().to_string();
        assert!(err.contains("requires a document base"), "{err}");
    }

    #[test]
    fn memory_urls_are_kept() {
        let loc = Location::from_path(Path::new("memory://test"));
        assert_eq!(loc.as_str(), "memory://test");
        assert_eq!(loc.resolve(None).unwrap(), PathBuf::from("memory://test"));
    }

    #[test]
    fn absolute_file_url_parses() {
        let loc = Location::parse("file:///tmp/a.wav").unwrap();
        let resolved = loc.as_path().unwrap();
        if cfg!(windows) {
            assert!(resolved.to_string_lossy().contains("tmp"));
        } else {
            assert_eq!(resolved, PathBuf::from("/tmp/a.wav"));
        }
    }

    #[test]
    fn plain_absolute_path_is_accepted() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\audio\a.wav")
        } else {
            PathBuf::from("/audio/a.wav")
        };
        let loc = Location::parse(&path.to_string_lossy()).unwrap();
        let resolved = loc.as_path().unwrap();
        assert_eq!(resolved, path);
    }

    #[test]
    fn serde_round_trip() {
        let loc = Location::parse("takes/a.wav").unwrap();
        let json = serde_json::to_string(&loc).unwrap();
        assert_eq!(json, "\"takes/a.wav\"");
        let back: Location = serde_json::from_str(&json).unwrap();
        assert_eq!(back, loc);
    }
}
