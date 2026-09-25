# Releasing FieldAssist

Binary distributions are built with [cargo-dist](https://opensource.axo.dev/cargo-dist/)
and published to GitHub Releases. Crates are **not** published to crates.io.

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
CI (`ci.yml`), promotes the cargo-dist GitHub Release (tag = `X.Y.Z`), bumps
the PR to the next `*-pre`, waits for CI again, and rebase-merges into
`main`. See `./script/release --help`.

## Cut a release branch (manual)

From an up-to-date `main` at `X.Y.Z-pre`:

```bash
git checkout main && git pull
git checkout -b release/X.Y.Z

# Strip the -pre suffix in the root Cargo.toml [workspace.package] version
# (e.g. 0.12.0-pre → 0.12.0), then refresh the changelog:
git cliff --unreleased --tag X.Y.Z --prepend CHANGELOG.md

git add Cargo.toml CHANGELOG.md
git commit -m "chore: prepare release X.Y.Z"
git push -u origin release/X.Y.Z
```

Open a PR into `release/X.Y.Z` for review if you prefer, or push commits
directly while iterating. Keep developing on `main` in parallel; cherry-pick
fixes between branches as needed.

## Dry-run (build without tagging)

With `dispatch-releases = true`, the Release workflow is started manually.
From the `release/X.Y.Z` tip (after the version bump):

1. GitHub → **Actions** → **Release** → **Run workflow**
2. Use branch `release/X.Y.Z`
3. Set **Release Tag** to `dry-run`

Or:

```bash
gh workflow run Release --ref release/X.Y.Z -f tag=dry-run
```

That builds archives and installers for Linux x64, macOS Apple Silicon, and
Windows x64, but does **not** create a git tag or GitHub Release. The
follow-up `Windows MSIX` workflow packs a signed `.msix` from the Windows
zips and uploads it as a workflow artifact (inspectable; not attached to a
Release on dry-run). Fix any failures on the release branch and re-run.

## Promote (tag after a green build)

When dry-run (and PR CI) are green:

```bash
gh workflow run Release --ref release/X.Y.Z -f tag=X.Y.Z
```

cargo-dist builds again, then creates the GitHub Release for tag `X.Y.Z`
(creating the git tag as part of that release). The `version-tags` ruleset
blocks rewriting or deleting tags; prefer not to `git push --tags` by hand.

## After promote

1. Merge release-branch fixes into `main` (rebase / FF only).
2. On `main`, bump `[workspace.package] version` to the next prerelease
   (e.g. `0.13.0-pre`) and commit.
3. Optionally delete `release/X.Y.Z` once it is fully merged.

## Local checks

```bash
dist plan          # what would be announced
dist generate --check
git cliff          # preview changelog
```

## Artifacts vs macOS `.app` / Windows MSIX

GitHub Releases ship platform archives of `FieldAssist`, `field-play`, and
`field-batch`, plus Unix shell installers. Windows no longer publishes a
PowerShell installer (Defender blocks unsigned remote scripts); the
`Windows MSIX` workflow packs a signed `.msix` after a successful Release
run and attaches it (with `FieldAssist.cer` and a `.sha256`) when the run
published a tag.

Finder-friendly macOS `.app` bundles remain a local concern via
`./script/bundle-macos` (see [BUILDING.md](BUILDING.md)). Notarization is
out of scope for v1 CI.

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
Import-Certificate -FilePath .\FieldAssist.cer `
  -CertStoreLocation Cert:\LocalMachine\TrustedPeople
Add-AppxPackage -Path .\FieldAssist-X.Y.Z.msix
```
