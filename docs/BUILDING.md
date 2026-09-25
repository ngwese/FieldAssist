# Building FieldAssist

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) (2021 edition)
- [Faust](https://faust.grame.fr/) **2.85.9** (optional) only when regenerating
  monitor DSP sources

### On Linux (Ubuntu)

Install these packages before building:

```bash
sudo apt install libasound2-dev libfontconfig-dev libxcb1-dev \
  libxkbcommon-dev libxkbcommon-x11-dev
```

## Faust monitor chains

Playback monitoring uses Faust DSP in
`crates/field-audio-monitor/dsp/` (`monitor_mono.dsp`, `monitor_stereo.dsp`,
`monitor_ms.dsp`, `monitor_foa.dsp`, `monitor_foa_fuma.dsp`). Generated
`.inc.rs` and `.json` files are committed under
`crates/field-audio-monitor/src/generated/` so ordinary `cargo build` does
**not** require Faust and does **not** rewrite those files when Faust happens
to be installed locally.

Stereo and M/S chains share
`crates/field-audio-monitor/dsp/headphone_crossfeed.lib`; FOA chains share
`crates/field-audio-monitor/dsp/bformat.lib`. All chains share
`crates/field-audio-monitor/dsp/meters.lib` (`Output/Gain` and RMS meters).

### Regenerating with `FAUST_REGENERATE`

By default, `field-audio-monitor`'s `build.rs` only copies the committed
artifacts into `OUT_DIR`. Faust is never invoked unless you opt in.

After changing a `.dsp` or `.lib` file:

1. Install Faust **2.85.9** (or point `FAUST` at that version's binary).
2. Regenerate and publish into `src/generated/`:

```bash
FAUST_REGENERATE=1 cargo build -p field-audio-monitor
```

3. Commit the resulting `crates/field-audio-monitor/src/generated/` changes
   together with the DSP edit.

| Variable | Purpose |
| --- | --- |
| `FAUST_REGENERATE=1` | Opt in to run Faust and update committed artifacts |
| `FAUST` | Optional path to the Faust binary when it is not on `PATH` |

If `FAUST_REGENERATE` is set but Faust cannot be found, the build fails with
an error. Leaving the variable unset keeps builds fast and keeps
`git status` clean even on machines that have Faust installed.

## Build

```bash
cargo build --release
```

```bash
cargo run --release
cargo run --release -- path/to/audio.wav
cargo run --release -- --list-devices
cargo run --release -- --dump-init
```

## App icon

The in-app mark, Windows `.exe` icon, and macOS `.app` icon all come from
[crates/field-assist/assets/logo/04-bands.svg](../crates/field-assist/assets/logo/04-bands.svg)
(logo study 4). Raster assets live in
[crates/field-assist/assets/app-icon/](../crates/field-assist/assets/app-icon/):

| File | Used by |
| --- | --- |
| `app-icon.ico` | Embedded into the Windows executable at compile time |
| `AppIcon.icns` | Copied into `FieldAssist.app/Contents/Resources/` |
| `app-icon.png` | 512px PNG of the same artwork |

After changing the SVG, regenerate those files (requires
[resvg_py](https://pypi.org/project/resvg_py/)):

```bash
pip install resvg_py
python script/generate-app-icon.py
```

On Windows, `build.rs` embeds `app-icon.ico` into the binary. On macOS,
`script/bundle-macos` copies `AppIcon.icns` into the bundle. The title-bar
glyph (Windows and Linux only) loads the SVG directly.

## macOS app bundle

On macOS, package a double-clickable `.app` that can also be launched from the
terminal:

```bash
./script/bundle-macos
open target/release/FieldAssist.app
```

Install into `~/Applications` and put the CLIs on your PATH (no sudo):

```bash
./script/bundle-macos --install
FieldAssist --help
field-assist path/to/audio.wav
field-play path/to/take.wav
field-batch --eval 'return app.name'
```

That copies the bundle to `~/Applications/FieldAssist.app`, symlinks
`~/.local/bin/FieldAssist` and `~/.local/bin/field-assist` to the binary
inside it, and installs release `field-play` and `field-batch` into
`~/.local/bin`. If `~/.local/bin` is not already on your PATH, add this to
`~/.zshrc` and open a new terminal:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Windows executable

On Windows, package a double-clickable `.exe` that can also be launched from
the terminal:

```powershell
powershell -File script/bundle-windows.ps1
```

That writes `target\release\FieldAssist.exe`. Double-click it, or run it from
a console:

```powershell
.\target\release\FieldAssist.exe
.\target\release\FieldAssist.exe --help
.\target\release\FieldAssist.exe --dump-init
.\target\release\FieldAssist.exe path\to\audio.wav
```

Install into `%LOCALAPPDATA%\FieldAssist`, put the CLIs on your PATH, and
add a Start Menu shortcut (no admin):

```powershell
powershell -File script/bundle-windows.ps1 --install
FieldAssist --help
FieldAssist path\to\audio.wav
field-play path\to\take.wav
field-batch --eval 'return app.name'
```

That copies the exe to `%LOCALAPPDATA%\FieldAssist\FieldAssist.exe`, links
`%USERPROFILE%\.local\bin\FieldAssist.exe` to it, installs release
`field-play` and `field-batch` into `%USERPROFILE%\.local\bin`, and creates
a Start Menu shortcut. If `~\.local\bin` is not already on your PATH, add it
in PowerShell and open a new terminal:

```powershell
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable(
    "Path",
    "$userPath;$env:USERPROFILE\.local\bin",
    "User"
)
```

## Releases

Binary GitHub Releases (archives + installers) are automated with cargo-dist.
See [RELEASING.md](RELEASING.md) for cutting a `release/X.Y.Z` branch, dry-run
builds, and promoting a version tag (no `v` prefix).
