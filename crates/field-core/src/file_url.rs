// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Encode a filesystem path as a document/media URL.
///
/// Prefers a relative path (forward slashes) when `path` is inside `base`.
/// Otherwise uses an absolute `file://` URL. `memory://` paths are kept as-is.
pub fn encode_file_url(path: &Path, base: Option<&Path>) -> String {
    let lossy = path.to_string_lossy();
    if lossy.starts_with("memory://") {
        return lossy.into_owned();
    }
    if let Some(base) = base {
        if let Some(rel) = relative_to(path, base) {
            return rel;
        }
    }
    to_absolute_file_url(path)
}

/// Resolve a stored URL or path to a filesystem path.
pub fn resolve_file_url(url: &str, base: Option<&Path>) -> Result<PathBuf> {
    let url = url.trim();
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
        None => Ok(path),
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

fn to_absolute_file_url(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
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
        let base = Path::new("/sessions");
        let path = Path::new("/sessions/takes/a.wav");
        let url = encode_file_url(path, Some(base));
        assert_eq!(url, "takes/a.wav");
        let resolved = resolve_file_url(&url, Some(base)).unwrap();
        assert_eq!(resolved, base.join("takes/a.wav"));
    }

    #[test]
    fn memory_urls_are_kept() {
        let path = Path::new("memory://test");
        assert_eq!(encode_file_url(path, None), "memory://test");
        assert_eq!(
            resolve_file_url("memory://test", None).unwrap(),
            PathBuf::from("memory://test")
        );
    }

    #[test]
    fn absolute_file_url_parses() {
        let resolved = resolve_file_url("file:///tmp/a.wav", None).unwrap();
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
        let resolved = resolve_file_url(&path.to_string_lossy(), None).unwrap();
        assert_eq!(resolved, path);
    }
}
