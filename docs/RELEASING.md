# Releasing FieldAssist

Binary distributions are built by the GitHub Actions **Release** workflow
using the local pack scripts (`script/bundle-windows.ps1`,
`script/bundle-macos`, `script/bundle-linux`, `script/bundle-deb`) and
published to GitHub Releases. Crates are **not** published to crates.io.

Version tags match the Cargo version exactly (no `v` prefix): `0.12.0`, not
`v0.12.0`.

## Version layout

| Branch | Workspace version | Tags |
| --- | --- | --- |
| `main` | next prerelease, e.g. `0.12.0-pre` | none for trunk |
| `release/X.Y.Z` | stable `X.Y.Z` | `X.Y.Z` after a successful promote |

`main` always points at the **next** intended release as a Cargo prerelease.
Do not promote releases from `main`.

## Merge policy

Pull requests into `main` and `release/**` must land via **rebase merge**
(or fast-forward). Squash and merge commits are disabled so the SHA that
passed PR CI is the SHA on the target branch. CI runs on `pull_request`
only — there is no duplicate run after merge.

## Quick path: `script/release`

From a clean tree on an up-to-date clone:

```bash
./script/release          # interactive; releases 0.12.0 from 0.12.0-pre
./script/release -y       # no confirmation prompt
./script/release --with-dry-run -y   # extra multi-OS artifact dry-run first
```

The script syncs `main`, cuts `release/X.Y.Z`, opens a PR for lightweight
CI (`ci.yml`), promotes the GitHub Release (tag = `X.Y.Z`), bumps the PR to
the next `*-pre`, waits for CI again, and rebase-merges into `main`. See
`./script/release --help`.

## Cut a release branch (manual)

From an up-to-date `main` at `X.Y.Z-pre`:

```bash
git checkout main && git pull
git checkout -b release/X.Y.Z

# Strip the -pre suffix in the root Cargo.toml [workspace.package] version
# (e.g. 0.12.0-pre → 0.12.0), then refresh the changelog. Prefer
# `./script/release`, which keeps the `# CHANGELOG` header and
# `<!-- git-cliff: end of header -->` marker consistent after prepend.
# Manual equivalent:
git cliff --unreleased --tag X.Y.Z --prepend CHANGELOG.md

git add Cargo.toml CHANGELOG.md
git commit -m "chore: prepare release X.Y.Z"
git push -u origin release/X.Y.Z
```

Open a PR into `release/X.Y.Z` for review if you prefer, or push commits
directly while iterating. Keep developing on `main` in parallel; cherry-pick
fixes between branches as needed.

## Dry-run (build without tagging)

The Release workflow is started manually. From the `release/X.Y.Z` tip
(after the version bump):

1. GitHub → **Actions** → **Release** → **Run workflow**
2. Use branch `release/X.Y.Z`
3. Set **Release Tag** to `dry-run`

Or:

```bash
gh workflow run Release --ref release/X.Y.Z -f tag=dry-run
```

That builds the Windows MSIX, macOS DMG, and Linux tarball + `.deb` on
each OS, but does **not** create a git tag or GitHub Release. Inspect the
workflow artifacts, fix any failures on the release branch, and re-run.

## Promote (tag after a green build)

When dry-run (and PR CI) are green:

```bash
gh workflow run Release --ref release/X.Y.Z -f tag=X.Y.Z
```

The workflow builds again, then creates the GitHub Release for tag `X.Y.Z`
(creating the git tag as part of that release). The `version-tags` ruleset
blocks rewriting or deleting tags; prefer not to `git push --tags` by hand.

## After promote

1. Merge release-branch fixes into `main` (rebase / FF only).
2. On `main`, bump `[workspace.package] version` to the next prerelease
   (e.g. `0.13.0-pre`) and commit.
3. Optionally delete `release/X.Y.Z` once it is fully merged.

## Local checks

```bash
git cliff          # preview changelog
```

## Release artifacts

Each GitHub Release ships only the platform install packages:

| Platform | Assets |
| --- | --- |
| Windows | `FieldAssist-<version>.msix`, `.sha256`, `FieldAssist-windows.cer` |
| macOS | `FieldAssist-<version>.dmg`, `.sha256`, `FieldAssist-macos.cer` |
| Linux | `FieldAssist-<version>-amd64.deb`, `.sha256`, `FieldAssist-<version>-x86_64-linux.tar.gz`, `.sha256` |

The DMG embeds `field-play` / `field-batch` inside the app bundle.
Notarization is out of scope for v1 CI.

The Linux `.deb` (package name `fieldassist`) installs the three binaries
into `/usr/bin`. It is built on Ubuntu 24.04 so the glibc baseline and
`Depends` work on Ubuntu 24.04, Ubuntu 26.04, and Debian stable (Trixie):

```bash
sudo apt install ./FieldAssist-X.Y.Z-amd64.deb
```

The Linux tarball contains the three binaries plus `install.sh`. End-user
install:

```bash
tar -xzf FieldAssist-X.Y.Z-x86_64-linux.tar.gz
cd FieldAssist-X.Y.Z-x86_64-linux
sudo ./install.sh
```

That installs into `/usr/local/bin`. Use `./install.sh --prefix DIR` for a
custom prefix. The GUI needs a desktop session with the usual ALSA and X11
client libraries; the install script does not install packages.

### Windows signing secrets

The MSIX publisher certificate is generated once with
`script/generate-windows-signing-cert.ps1`. Commit only the public
`.cer`. Store the private key as repository secrets (never in git):

| Secret | Value |
| --- | --- |
| `WINDOWS_SIGNING_PFX` | Base64 of the `.pfx` bytes |
| `WINDOWS_SIGNING_PASSWORD` | PFX password (empty string if none) |

Encode the PFX for the secret:

```powershell
[Convert]::ToBase64String(
  [IO.File]::ReadAllBytes(
    "crates\field-assist\assets\windows\FieldAssist.pfx"
  )
) | Set-Clipboard
```

End-user install (one-time elevated cert trust; package install/upgrade
is per-user):

```powershell
Import-Certificate -FilePath .\FieldAssist-windows.cer `
  -CertStoreLocation Cert:\LocalMachine\TrustedPeople
Add-AppxPackage -Path .\FieldAssist-X.Y.Z.msix
```

### macOS signing secrets

The DMG / `.app` signing certificate is generated once with
`script/generate-macos-signing-cert`. Commit only the public `.cer`.
Store the private key as repository secrets (never in git):

| Secret | Value |
| --- | --- |
| `MACOS_SIGNING_P12` | Base64 of the `.p12` bytes |
| `MACOS_SIGNING_PASSWORD` | PKCS#12 password (must be non-empty) |

Encode the p12 for the secret:

```bash
base64 -i crates/field-assist/assets/macos/FieldAssist.p12 | pbcopy
```

Local packs can use `--from-op` against vault `Private` items
`FieldAssist - macOS Signing Password` and
`FieldAssist - macOS Signing Certificate` instead of the env vars.

End-user install: open the DMG, drag FieldAssist to Applications, then on
first launch use Open Anyway (or trust `FieldAssist-macos.cer` in Keychain).
Use **FieldAssist → Install CLI Tools** for `/usr/local/bin` symlinks.
