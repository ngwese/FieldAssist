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

## macOS app bundle and DMG

On macOS, package a double-clickable `.app` and a Finder-friendly DMG:

```bash
./script/bundle-macos --from-op
open target/release/FieldAssist.app
# Or pack and open the DMG in one step:
./script/bundle-macos --from-op --skip-build --open
```

That builds release `FieldAssist`, `field-play`, and `field-batch`, puts all
three into `FieldAssist.app/Contents/MacOS/`, signs with the FieldAssist
code-signing certificate, and writes
`target/release/FieldAssist-<version>.dmg` (drag the app onto Applications).

Install into `~/Applications` and put developer CLI symlinks on your PATH
(no sudo; uses `~/.local/bin`):

```bash
./script/bundle-macos --from-op --install
FieldAssist --help
field-assist path/to/audio.wav
field-play path/to/take.wav
field-batch --eval 'return app.name'
```

That copies the bundle to `~/Applications/FieldAssist.app` and symlinks
`~/.local/bin/FieldAssist`, `field-assist`, `field-play`, and `field-batch`
to the binaries inside it. End users who install from the DMG should instead
use **FieldAssist → Install CLI Tools**, which writes the same CLI names
into `/usr/local/bin` (administrator password once).

If `~/.local/bin` is not already on your PATH, add this to `~/.zshrc` and
open a new terminal:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Signing certificate

Signer identity is the cert subject (`CN=FieldAssist`). Reuse one
certificate for all versions.

Generate once (writes a gitignored `.p12` and a public `.cer` to commit):

```bash
./script/generate-macos-signing-cert --from-op
```

Local packs look for
`crates/field-assist/assets/macos/FieldAssist.p12`, or
`FIELDASSIST_MACOS_SIGNING_P12` / `FIELDASSIST_MACOS_SIGNING_PASSWORD`.
Prefer `--from-op`, which reads:

| 1Password item (vault `Private`) | Used for |
| --- | --- |
| `FieldAssist - macOS Signing Password` | PKCS#12 password (`op read …/password`) |
| `FieldAssist - macOS Signing Certificate` | PKCS#12 document (`op document get`) |

Without a PKCS#12 the script falls back to ad-hoc signing (fine for local
smoke tests; not for release). The PKCS#12 password must be non-empty —
Apple's `security import` rejects empty passwords.

## Windows MSIX

On Windows, package a signed per-user MSIX that registers Start Menu,
file associations, and CLI aliases. Requires the [Windows 10 SDK](https://developer.microsoft.com/windows/downloads/windows-sdk/)
(`makeappx.exe` / `signtool.exe`).

```powershell
powershell -File script/bundle-windows.ps1
```

That builds release `FieldAssist.exe`, `field-play.exe`, and
`field-batch.exe`, packs them into
`target\release\FieldAssist-<version>.msix`, and signs the package with
the FieldAssist publisher certificate.

Install for the current user (UAC once to trust the publisher certificate;
package install itself needs no admin):

```powershell
powershell -File script/bundle-windows.ps1 --install
FieldAssist --help
FieldAssist path\to\audio.wav
field-play path\to\take.wav
field-batch --eval 'return app.name'
```

That trusts
[FieldAssist.cer](../crates/field-assist/assets/windows/FieldAssist.cer)
in `Cert:\LocalMachine\TrustedPeople` (elevated, one time) and runs
`Add-AppxPackage`. The package includes:

- Start Menu entry for FieldAssist
- File associations for `.facomp`, `.fasession`, and common audio
  extensions (Open with; existing defaults stay)
- App execution aliases `FieldAssist.exe`, `field-play.exe`, and
  `field-batch.exe` under `%LOCALAPPDATA%\Microsoft\WindowsApps` (already
  on the user PATH)

Upgrade by installing a newer MSIX signed with the same certificate
(no re-trust).

### Remove a conflicting install

`Add-AppxPackage` fails if an older FieldAssist package is already
registered (especially one signed with a **different** publisher
certificate, or left over from an earlier local pack). Remove it for the
current user, then re-run `--install`:

```powershell
# List what is installed
Get-AppxPackage *FieldAssist* | Format-List Name, Version, Publisher, PackageFullName

# Remove every FieldAssist MSIX for this user
Get-AppxPackage *FieldAssist* | Remove-AppxPackage
```

If `Remove-AppxPackage` reports the package is in use, quit FieldAssist
(and any `field-play` / `field-batch` processes) and try again.

Older pre-MSIX installs (from the previous
`%LOCALAPPDATA%\FieldAssist` copy) are not AppX packages. Clean those up
separately if Start Menu or PATH still point at them:

```powershell
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue `
  "$env:LOCALAPPDATA\FieldAssist"
Remove-Item -Force -ErrorAction SilentlyContinue `
  "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\FieldAssist.lnk"
Remove-Item -Force -ErrorAction SilentlyContinue `
  "$env:USERPROFILE\.local\bin\FieldAssist.exe",
  "$env:USERPROFILE\.local\bin\field-play.exe",
  "$env:USERPROFILE\.local\bin\field-batch.exe"
```

After a clean uninstall, install again:

```powershell
powershell -File script/bundle-windows.ps1 --from-op --install
```

### Signing certificate

Publisher identity is the cert subject (`CN=FieldAssist`). Reuse one
certificate for all versions so upgrades replace the installed package.

Generate once (writes a gitignored `.pfx` and a public `.cer` to commit):

```powershell
powershell -File script/generate-windows-signing-cert.ps1 --from-op
```

Local packs look for
`crates/field-assist/assets/windows/FieldAssist.pfx`, or
`FIELDASSIST_SIGNING_PFX` / `FIELDASSIST_SIGNING_PASSWORD` (or pass
`--from-op` to read the password from 1Password).

## Releases

Binary GitHub Releases (archives + installers) are automated with cargo-dist.
See [RELEASING.md](RELEASING.md) for cutting a `release/X.Y.Z` branch, dry-run
builds, and promoting a version tag (no `v` prefix). The usual path is
`./script/release`.
