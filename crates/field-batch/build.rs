// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    emit_git_revision();
}

fn emit_git_revision() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let git_root = find_git_root(&manifest).unwrap_or_else(|| manifest.clone());
    let git_dir = git_root.join(".git");
    let head = git_dir.join("HEAD");
    if head.is_file() {
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(contents) = fs::read_to_string(&head) {
            if let Some(reference) = contents.strip_prefix("ref: ") {
                let ref_path = git_dir.join(reference.trim());
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }
    let index = git_dir.join("index");
    if index.is_file() {
        println!("cargo:rerun-if-changed={}", index.display());
    }

    let revision = git_output(&git_root, &["rev-parse", "--short=7", "HEAD"])
        .unwrap_or_else(|| "unknown".into());
    let dirty = git_output(
        &git_root,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .is_some_and(|out| !out.is_empty());
    println!("cargo:rustc-env=GIT_REVISION={revision}");
    println!(
        "cargo:rustc-env=GIT_DIRTY={}",
        if dirty { "true" } else { "false" }
    );
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(text)
}
