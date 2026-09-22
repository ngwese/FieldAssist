// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Headless Lua batch runtime for FieldAssist sessions and workflows.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::Parser;
use field_scripting::{HostProfile, ScriptHost};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

#[derive(Parser, Debug)]
#[command(
    name = "field-batch",
    version = env!("CARGO_PKG_VERSION"),
    about = "Headless Lua runtime for FieldAssist sessions and workflows",
    long_about = "Runs Lua against the shared field.* scripting host. With no \
script argument, opens an interactive REPL. On Unix, usable as a shebang \
interpreter (`#!/usr/bin/env field-batch`). Startup loads init.lua (user \
config else embedded); does not load FieldAssist workflow bundles."
)]
struct Args {
    /// Lua script to run. When omitted, open a REPL.
    script: Option<PathBuf>,

    /// Arguments forwarded to the script as `app.args`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    script_args: Vec<String>,

    /// Config directory for `field.include` and `init.lua` (default: FieldAssist
    /// config dir; user file else embedded default).
    #[arg(long)]
    config_dir: Option<PathBuf>,

    /// Evaluate a single expression and exit (testing / non-interactive).
    #[arg(long)]
    eval: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("field-batch: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = Args::parse();
    let config_dir = args.config_dir.or_else(field_scripting::user_config_dir);
    let mut host = ScriptHost::new(HostProfile {
        name: "field-batch",
        config_dir,
    })
    .map_err(|err| anyhow::anyhow!("create script host: {err}"))?;

    host.load_init()
        .map_err(|err| anyhow::anyhow!("load init.lua: {err}"))?;
    flush_alerts(&host);

    if let Some(expr) = args.eval.as_ref() {
        host.set_args(args.script_args.clone());
        let out = host.eval(expr);
        for line in &out.prints {
            println!("{line}");
        }
        if let Some(err) = out.error {
            bail!("{err}");
        }
        if let Some(result) = out.result {
            println!("{result}");
        }
        flush_alerts(&host);
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(script) = args.script.as_ref() {
        host.set_args(args.script_args.clone());
        host.load_file(script)
            .map_err(|err| anyhow::anyhow!("load {}: {err}", script.display()))?;
        flush_alerts(&host);
        return Ok(ExitCode::SUCCESS);
    }

    repl(&mut host)?;
    Ok(ExitCode::SUCCESS)
}

fn repl(host: &mut ScriptHost) -> Result<()> {
    println!(
        "field-batch {} ({}) — type expressions, Ctrl-D to exit",
        env!("CARGO_PKG_VERSION"),
        build_revision()
    );
    let mut editor = DefaultEditor::new()?;
    loop {
        match editor.readline("> ") {
            Ok(line) => {
                let _ = editor.add_history_entry(line.as_str());
                let out = host.eval(&line);
                for printed in &out.prints {
                    println!("{printed}");
                }
                if let Some(err) = &out.error {
                    eprintln!("error: {err}");
                } else if let Some(result) = &out.result {
                    println!("{result}");
                }
                flush_alerts(host);
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(err) => bail!("readline: {err}"),
        }
    }
    Ok(())
}

fn build_revision() -> String {
    let revision = env!("GIT_REVISION");
    if env!("GIT_DIRTY") == "true" {
        format!("{revision}-dirty")
    } else {
        revision.to_string()
    }
}

fn flush_alerts(host: &ScriptHost) {
    for line in host.take_prints() {
        println!("{line}");
    }
    for (subject, body) in host.take_alerts() {
        eprintln!("alert: {subject}: {body}");
    }
    for entry in host.take_logs() {
        eprintln!(
            "{} [{}] {}",
            entry.level.as_str(),
            entry.topic,
            entry.message
        );
    }
}
