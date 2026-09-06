// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result};
use clap::Parser;

pub const APP_NAME: &str = "FieldAssist";
pub const APP_COPYRIGHT: &str = "Copyright © 2026 Greg Wuller";

mod app;
mod assets;
mod audio;
mod commands;
mod components;
mod model;
mod playback;
mod progress;
mod render;
mod script;
mod session;

#[derive(Parser, Debug)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    after_help = "Supports WAV, FLAC, MP3, OGG, M4A, and other formats enabled by Symphonia."
)]
struct Args {
    /// Path to an audio file
    file: Option<PathBuf>,

    /// List available output audio devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Print the embedded init.lua to stdout and exit
    #[arg(long)]
    dump_init: bool,

    /// Output device name or index (default: system default)
    #[arg(long, id = "output")]
    output_device: Option<String>,
}

fn main() -> Result<()> {
    #[cfg(windows)]
    attach_parent_console();

    let args = Args::parse();

    if args.list_devices {
        playback::print_output_devices()?;
        return Ok(());
    }

    if args.dump_init {
        print!("{}", script::EMBEDDED_INIT);
        return Ok(());
    }

    let (composition, load_elapsed) = match &args.file {
        Some(path) => {
            let started = Instant::now();
            let composition = model::Composition::load_from_path(path)
                .with_context(|| format!("failed to load {}", path.display()))?;
            (
                Some(composition),
                Some(started.elapsed().as_secs_f64()),
            )
        }
        None => (None, None),
    };
    let device = playback::resolve_output_device(args.output_device.as_deref())?;
    app::run(composition, load_elapsed, device);
    Ok(())
}

/// Attach to the parent terminal when launched from a console so clap and
/// `println!` still work. Double-click has no parent console; this is a no-op.
#[cfg(windows)]
fn attach_parent_console() {
    use std::fs::OpenOptions;
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetConsoleMode, SetConsoleMode, SetStdHandle, ATTACH_PARENT_PROCESS,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    extern "C" {
        fn _dup2(fd1: i32, fd2: i32) -> i32;
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    }
    const O_TEXT: i32 = 0x4000;

    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }

    let Ok(out) = OpenOptions::new().read(true).write(true).open("CONOUT$") else {
        return;
    };
    let handle = out.into_raw_handle();
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
        let fd = _open_osfhandle(handle as isize, O_TEXT);
        if fd >= 0 {
            _dup2(fd, 1);
            _dup2(fd, 2);
        }
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) != 0 {
            let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}
