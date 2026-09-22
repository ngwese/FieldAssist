// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::process::Command;

fn empty_config_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp config dir")
}

#[test]
fn eval_flag_prints_app_name() {
    let config = empty_config_dir();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    let output = Command::new(bin)
        .args([
            "--config-dir",
            config.path().to_str().unwrap(),
            "--eval",
            "app.name",
        ])
        .output()
        .expect("spawn field-batch");
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("field-batch"), "{stdout}");
}

#[test]
fn runs_script_file() {
    let config = empty_config_dir();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("t.lua");
    std::fs::write(&script, "print(app.name)").unwrap();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    let output = Command::new(bin)
        .args(["--config-dir", config.path().to_str().unwrap()])
        .arg(&script)
        .output()
        .expect("spawn field-batch");
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("field-batch"), "{stdout}");
}

#[test]
fn shebang_style_argv_runs_script() {
    let config = empty_config_dir();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("shebang.lua");
    std::fs::write(&script, "print(app.name)\nassert(app.args[1] == \"x\")\n").unwrap();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    let output = Command::new(bin)
        .args(["--config-dir", config.path().to_str().unwrap()])
        .arg(&script)
        .arg("x")
        .output()
        .expect("spawn field-batch");
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("field-batch"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn script_args_after_filename() {
    let config = empty_config_dir();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("args.lua");
    std::fs::write(
        &script,
        r#"
        assert(app.name == "field-batch")
        assert(app.args[1] == "--flag")
        assert(app.args[2] == "val")
        print("ok")
        "#,
    )
    .unwrap();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    let output = Command::new(bin)
        .args([
            std::ffi::OsStr::new("--config-dir"),
            config.path().as_os_str(),
            script.as_os_str(),
            std::ffi::OsStr::new("--flag"),
            std::ffi::OsStr::new("val"),
        ])
        .output()
        .expect("spawn");
    assert!(output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
}

#[test]
fn eval_session_new() {
    let config = empty_config_dir();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    let output = Command::new(bin)
        .args([
            "--config-dir",
            config.path().to_str().unwrap(),
            "--eval",
            r#"(function() local s=field.session.new(); s.properties={k="v"}; return s.properties.k end)()"#,
        ])
        .output()
        .expect("spawn");
    assert!(output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("v"));
}

#[test]
fn repl_banner_precedes_init_logs() {
    let config = empty_config_dir();
    let bin = env!("CARGO_BIN_EXE_field-batch");
    // stdin EOF exits the REPL immediately after startup.
    let output = Command::new(bin)
        .args(["--config-dir", config.path().to_str().unwrap()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn field-batch");
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout
            .lines()
            .next()
            .is_some_and(|l| l.starts_with("field-batch ")),
        "banner should be first on stdout, got:\n{stdout}"
    );
    assert!(
        stderr.contains("info [init]"),
        "init.lua should still log after banner, got stderr:\n{stderr}"
    );
}
