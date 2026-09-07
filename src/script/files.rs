// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

/// Recursively list files under `root` as `/`-separated paths relative to `root`.
///
/// `include(dirname, basename)` is called for each file. `dirname` is the
/// parent relative to `root` (empty when the file sits in `root` itself).
/// Return `true` to keep the file. Directories, `.` / `..`, and unreadable
/// entries are skipped. Symlinks are not followed.
pub fn find_files_matching(
    root: &Path,
    mut include: impl FnMut(&str, &str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let meta = std::fs::metadata(root).map_err(|err| format!("{}: {err}", root.display()))?;
    if !meta.is_dir() {
        return Err(format!(
            "find_files expects a directory, got {}",
            root.display()
        ));
    }
    let mut found = Vec::new();
    let mut stack = vec![WalkDir {
        abs: root.to_path_buf(),
        rel: String::new(),
    }];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir.abs) {
            Ok(entries) => entries,
            Err(err) if dir.rel.is_empty() => {
                return Err(format!("{}: {err}", dir.abs.display()));
            }
            Err(_) => continue,
        };
        let mut children: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
        children.sort_by_key(|entry| entry.file_name());
        for entry in children.into_iter().rev() {
            let name = entry.file_name();
            let Some(basename) = name.to_str() else {
                continue;
            };
            if basename == "." || basename == ".." {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            let rel = join_rel(&dir.rel, basename);
            if file_type.is_dir() {
                stack.push(WalkDir {
                    abs: entry.path(),
                    rel,
                });
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if include(&dir.rel, basename)? {
                found.push(rel);
            }
        }
    }
    found.sort();
    Ok(found)
}

pub fn find_files(root: &Path, extensions: Option<&[String]>) -> Result<Vec<String>, String> {
    find_files_matching(root, |_, basename| {
        Ok(match extensions {
            None => true,
            Some(exts) => extension_matches(basename, exts),
        })
    })
}

pub fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn extension_matches(basename: &str, extensions: &[String]) -> bool {
    let Some(ext) = Path::new(basename).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    extensions.iter().any(|wanted| wanted == &ext)
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

struct WalkDir {
    abs: PathBuf,
    rel: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tree(dir: &Path) {
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("take.wav"), b"wav").unwrap();
        std::fs::write(dir.join("nested").join("more.flac"), b"flac").unwrap();
        std::fs::write(dir.join("nested").join("notes.txt"), b"txt").unwrap();
        std::fs::write(dir.join("session.fasession"), b"{}").unwrap();
        std::fs::write(dir.join("edit.facomp"), b"{}").unwrap();
    }

    #[test]
    fn lists_all_files_relative_to_root() {
        let dir = std::env::temp_dir().join("fieldassist-find-files-all");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_tree(&dir);
        let found = find_files(&dir, None).unwrap();
        assert_eq!(
            found,
            [
                "edit.facomp",
                "nested/more.flac",
                "nested/notes.txt",
                "session.fasession",
                "take.wav",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filters_by_extension_without_dot() {
        let dir = std::env::temp_dir().join("fieldassist-find-files-ext");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_tree(&dir);
        let found =
            find_files(&dir, Some(&["wav".into(), "flac".into(), "facomp".into()])).unwrap();
        assert_eq!(found, ["edit.facomp", "nested/more.flac", "take.wav"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_file_path() {
        let dir = std::env::temp_dir().join("fieldassist-find-files-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("take.wav");
        std::fs::write(&file, b"wav").unwrap();
        let err = find_files(&file, None).unwrap_err();
        assert!(err.contains("expects a directory"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
