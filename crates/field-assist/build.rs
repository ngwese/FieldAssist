// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/app-icon.ico");
    println!("cargo:rerun-if-changed=assets/logo/04-bands.svg");

    emit_git_revision();
    embed_windows_icon();
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

fn embed_windows_icon() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = Path::new(&manifest_dir)
        .join("assets")
        .join("app-icon")
        .join("app-icon.ico");
    if !icon.is_file() {
        panic!(
            "missing {}; run python script/generate-app-icon.py",
            icon.display()
        );
    }

    let icon_escaped = icon.display().to_string().replace('\\', "/");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    let rc_path = Path::new(&out_dir).join("app_icon.rc");
    fs::write(&rc_path, format!("1 ICON \"{icon_escaped}\"\n")).expect("write app_icon.rc");

    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .expect("embed Windows app icon");
}
