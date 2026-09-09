# Building FieldAssist

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) (2021 edition)
- [Faust](https://faust.grame.fr/) (optional) to regenerate monitor DSP sources

## Faust monitor chains

Playback monitoring compiles Faust DSP in `dsp/` (`monitor_mono.dsp`,
`monitor_stereo.dsp`, `monitor_ms.dsp`, `monitor_foa.dsp`) to Rust via
`faust -lang rust -json` in `build.rs`. Generated `.inc.rs` and `.json` files
are committed under `src/monitor/generated/` so `cargo build` does not require
Faust. When `faust` is on `PATH` (or `FAUST` points at the binary), those
artifacts are regenerated. Stereo and M/S chains share
`dsp/headphone_crossfeed.lib`.

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
[assets/logo/04-bands.svg](assets/logo/04-bands.svg) (logo study 4). Raster
assets live in [assets/app-icon/](assets/app-icon/):

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

Install into `~/Applications` and put `FieldAssist` / `field-assist` on your
PATH (no sudo):

```bash
./script/bundle-macos --install
FieldAssist --help
field-assist path/to/audio.wav
```

That copies the bundle to `~/Applications/FieldAssist.app` and symlinks
`~/.local/bin/FieldAssist` and `~/.local/bin/field-assist` to the binary
inside it. If `~/.local/bin` is not already on your PATH, add this to
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

Install into `%LOCALAPPDATA%\FieldAssist`, put `FieldAssist` on your PATH, and
add a Start Menu shortcut (no admin):

```powershell
powershell -File script/bundle-windows.ps1 --install
FieldAssist --help
FieldAssist path\to\audio.wav
```

That copies the exe to `%LOCALAPPDATA%\FieldAssist\FieldAssist.exe`, links
`%USERPROFILE%\.local\bin\FieldAssist.exe` to it, and creates a Start Menu
shortcut. If `~\.local\bin` is not already on your PATH, add it in PowerShell
and open a new terminal:

```powershell
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable(
    "Path",
    "$userPath;$env:USERPROFILE\.local\bin",
    "User"
)
```
