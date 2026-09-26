# SPDX-FileCopyrightText: 2026 Greg Wuller
# SPDX-License-Identifier: MIT
#
# Build a FieldAssist MSIX package (and optionally install it per-user).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$DefaultOpPasswordRef = "op://Private/FieldAssist - PFX Signing Password/password"

function Test-IsWindows {
    if ($null -ne (Get-Variable -Name IsWindows -Scope Global -ErrorAction SilentlyContinue)) {
        return [bool]$IsWindows
    }
    return $env:OS -eq "Windows_NT"
}

function Show-Usage {
    @"
Usage: script/bundle-windows.ps1 [options]

Build a signed FieldAssist MSIX at target/release/FieldAssist-<version>.msix.
Also builds release binaries for field-play and field-batch (unless
--skip-build).

  --install       Trust the publisher cert for the current user (if needed)
                  and install/upgrade the MSIX with Add-AppxPackage
  --skip-build    Use existing FieldAssist.exe / field-play.exe /
                  field-batch.exe under the release directory (or
                  --bin-dir)
  --bin-dir DIR   Directory containing the three release executables
                  (default: target/release, or CARGO_TARGET_DIR/release)
  --out PATH      Output MSIX path (default:
                  target/release/FieldAssist-<cargo-version>.msix)
  --pfx PATH      Code-signing PFX (default: FIELDASSIST_SIGNING_PFX,
                  else crates/field-assist/assets/windows/FieldAssist.pfx)
  --from-op       Read the PFX password from 1Password via ``op read``
                  (default ref: $DefaultOpPasswordRef)
  --op-ref REF    1Password secret reference for --from-op
  -h, --help      Show this help

Environment:
  FIELDASSIST_SIGNING_PFX       Path to the signing PFX
  FIELDASSIST_SIGNING_PASSWORD  Password for the PFX (empty allowed;
                                ignored when --from-op is set)
"@
}

function Get-PasswordFrom1Password {
    param([Parameter(Mandatory = $true)][string]$OpRef)

    $op = Get-Command op -ErrorAction SilentlyContinue
    if ($null -eq $op) {
        Write-Error "1Password CLI (op) not found on PATH; install it or set FIELDASSIST_SIGNING_PASSWORD"
        exit 1
    }

    Write-Host "Reading PFX password from 1Password ($OpRef)..."
    $password = & op read $OpRef
    if ($LASTEXITCODE -ne 0) {
        Write-Error "op read failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
    if ($null -eq $password) {
        Write-Error "op read returned no password for $OpRef"
        exit 1
    }
    # op may emit a trailing newline
    return ([string]$password).TrimEnd("`r", "`n")
}

function Get-WindowsSdkTool {
    param([Parameter(Mandatory = $true)][string]$Name)

    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -ne $cmd) {
        return $cmd.Source
    }

    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (-not (Test-Path -LiteralPath $kitsRoot)) {
        Write-Error "Windows 10 SDK not found (missing $kitsRoot). Install the Windows SDK so makeappx.exe and signtool.exe are available."
        exit 1
    }

    $latest = Get-ChildItem -LiteralPath $kitsRoot -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1
    if ($null -eq $latest) {
        Write-Error "no versioned Windows SDK bin directory under $kitsRoot"
        exit 1
    }

    $path = Join-Path $latest.FullName "x64\$Name"
    if (-not (Test-Path -LiteralPath $path)) {
        Write-Error "missing $path"
        exit 1
    }
    return $path
}

function Convert-CargoVersionToMsix {
    param([Parameter(Mandatory = $true)][string]$CargoVersion)

    if ($CargoVersion -notmatch '^(\d+)\.(\d+)\.(\d+)') {
        Write-Error "unsupported Cargo version for MSIX identity: $CargoVersion"
        exit 1
    }
    return "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"
}

function Resize-Png {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][int]$Size
    )

    Add-Type -AssemblyName System.Drawing
    $img = [System.Drawing.Image]::FromFile((Resolve-Path -LiteralPath $Source).Path)
    try {
        $bitmap = New-Object System.Drawing.Bitmap $Size, $Size
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $graphics.DrawImage($img, 0, 0, $Size, $Size)
            }
            finally {
                $graphics.Dispose()
            }
            $bitmap.Save($Destination, [System.Drawing.Imaging.ImageFormat]::Png)
        }
        finally {
            $bitmap.Dispose()
        }
    }
    finally {
        $img.Dispose()
    }
}

function Ensure-PublisherTrusted {
    param(
        [Parameter(Mandatory = $true)][string]$CerPath,
        [Parameter(Mandatory = $true)][string]$MsixPath
    )

    $cerFull = (Resolve-Path -LiteralPath $CerPath).Path
    $cer = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($cerFull)
    $thumb = $cer.Thumbprint

    # AppX deployment validates the package signature against the Local
    # Machine Trusted People store. That import is a one-time elevated
    # step; Add-AppxPackage itself stays per-user and needs no admin.
    $machinePeople = Get-ChildItem Cert:\LocalMachine\TrustedPeople -ErrorAction SilentlyContinue |
        Where-Object { $_.Thumbprint -eq $thumb }
    if ($null -ne $machinePeople) {
        Write-Host "Publisher certificate already in LocalMachine\TrustedPeople (thumbprint $thumb)"
        return
    }

    Write-Host "Trusting publisher certificate in LocalMachine\TrustedPeople (one-time, requires elevation)..."
    try {
        $store = New-Object System.Security.Cryptography.X509Certificates.X509Store(
            [System.Security.Cryptography.X509Certificates.StoreName]::TrustedPeople,
            [System.Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine
        )
        $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
        $store.Add($cer)
        $store.Close()
        return
    }
    catch {
        Write-Host "Direct import failed ($($_.Exception.Message)); prompting for elevation..."
    }

    $tmp = Join-Path $env:TEMP "fieldassist-trust-cert.ps1"
    $trustScript = @"
`$ErrorActionPreference = 'Stop'
Import-Certificate -FilePath '$cerFull' -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
"@
    Set-Content -LiteralPath $tmp -Value $trustScript -Encoding UTF8
    try {
        $proc = Start-Process -FilePath "powershell.exe" `
            -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$tmp`"" `
            -Verb RunAs -Wait -PassThru
        if ($proc.ExitCode -ne 0) {
            throw "elevated Import-Certificate exited $($proc.ExitCode)"
        }
    }
    catch {
        Write-Error @"
could not import $CerPath into LocalMachine\TrustedPeople: $($_.Exception.Message)

AppX requires the publisher certificate in the machine Trusted People store
once (UAC). Install/upgrade of the MSIX afterward is per-user and needs no
admin. From an elevated PowerShell:

  Import-Certificate -FilePath '$CerPath' ``
    -CertStoreLocation Cert:\LocalMachine\TrustedPeople
  Add-AppxPackage -Path '$MsixPath' -ForceUpdateFromAnyVersion
"@
        exit 1
    }
    finally {
        Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $tmp
    }

    $machinePeople = Get-ChildItem Cert:\LocalMachine\TrustedPeople -ErrorAction SilentlyContinue |
        Where-Object { $_.Thumbprint -eq $thumb }
    if ($null -eq $machinePeople) {
        Write-Error "certificate was not present in LocalMachine\TrustedPeople after elevation"
        exit 1
    }
}

if (-not (Test-IsWindows)) {
    Write-Error "script/bundle-windows.ps1 only runs on Windows"
    exit 1
}

$install = $false
$skipBuild = $false
$binDir = $null
$outPath = $null
$pfxPath = $null
$fromOp = $false
$opRef = $DefaultOpPasswordRef

for ($i = 0; $i -lt $args.Count; $i++) {
    switch ($args[$i]) {
        "--install" { $install = $true }
        "-Install" { $install = $true }
        "--skip-build" { $skipBuild = $true }
        "--from-op" { $fromOp = $true }
        "--bin-dir" {
            $i++
            if ($i -ge $args.Count) {
                Write-Error "--bin-dir requires a path"
                exit 1
            }
            $binDir = $args[$i]
        }
        "--out" {
            $i++
            if ($i -ge $args.Count) {
                Write-Error "--out requires a path"
                exit 1
            }
            $outPath = $args[$i]
        }
        "--pfx" {
            $i++
            if ($i -ge $args.Count) {
                Write-Error "--pfx requires a path"
                exit 1
            }
            $pfxPath = $args[$i]
        }
        "--op-ref" {
            $i++
            if ($i -ge $args.Count) {
                Write-Error "--op-ref requires a value"
                exit 1
            }
            $opRef = $args[$i]
            $fromOp = $true
        }
        "-h" { Show-Usage; exit 0 }
        "--help" { Show-Usage; exit 0 }
        default {
            Write-Error "unknown argument: $($args[$i])"
            Show-Usage
            exit 1
        }
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $root

$cargoToml = Get-Content -Raw (Join-Path $root "Cargo.toml")
if ($cargoToml -notmatch '(?m)^version = "([^"]+)"') {
    Write-Error "could not read version from Cargo.toml"
    exit 1
}
$cargoVersion = $Matches[1]
$msixVersion = Convert-CargoVersionToMsix $cargoVersion

$assetsWindows = Join-Path $root "crates\field-assist\assets\windows"
$manifestTemplate = Join-Path $assetsWindows "AppxManifest.xml"
$cerPath = Join-Path $assetsWindows "FieldAssist.cer"
$defaultPfx = Join-Path $assetsWindows "FieldAssist.pfx"
$iconPng = Join-Path $root "crates\field-assist\assets\app-icon\app-icon.png"

if (-not (Test-Path -LiteralPath $manifestTemplate)) {
    Write-Error "missing manifest template: $manifestTemplate"
    exit 1
}
if (-not (Test-Path -LiteralPath $iconPng)) {
    Write-Error "missing app icon PNG: $iconPng"
    exit 1
}

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }
$releaseDir = Join-Path $targetDir "release"
if ($null -eq $binDir) {
    $binDir = $releaseDir
}
else {
    $binDir = (Resolve-Path -LiteralPath $binDir).Path
}

if (-not $skipBuild) {
    Write-Host "Building FieldAssist $cargoVersion, field-play, and field-batch (release)..."
    cargo build --release -p FieldAssist -p field-play -p field-batch
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$binary = Join-Path $binDir "FieldAssist.exe"
$fieldPlayBin = Join-Path $binDir "field-play.exe"
$fieldBatchBin = Join-Path $binDir "field-batch.exe"
foreach ($path in @($binary, $fieldPlayBin, $fieldBatchBin)) {
    if (-not (Test-Path -LiteralPath $path)) {
        Write-Error "missing release binary: $path"
        exit 1
    }
}

if ($null -eq $outPath) {
    $outPath = Join-Path $releaseDir "FieldAssist-$cargoVersion.msix"
}
else {
    $outParent = Split-Path -Parent $outPath
    if ($outParent -and -not (Test-Path -LiteralPath $outParent)) {
        New-Item -ItemType Directory -Force -Path $outParent | Out-Null
    }
}

if ($null -eq $pfxPath) {
    if ($env:FIELDASSIST_SIGNING_PFX) {
        $pfxPath = $env:FIELDASSIST_SIGNING_PFX
    }
    else {
        $pfxPath = $defaultPfx
    }
}
if (-not (Test-Path -LiteralPath $pfxPath)) {
    Write-Error @"
missing signing PFX: $pfxPath

Generate once with:
  powershell -File script/generate-windows-signing-cert.ps1 --from-op
Then set FIELDASSIST_SIGNING_PFX / FIELDASSIST_SIGNING_PASSWORD, pass
--from-op, or place FieldAssist.pfx next to the committed FieldAssist.cer.
"@
    exit 1
}
if ($fromOp) {
    $pfxPassword = Get-PasswordFrom1Password -OpRef $opRef
}
elseif ($null -ne $env:FIELDASSIST_SIGNING_PASSWORD) {
    $pfxPassword = $env:FIELDASSIST_SIGNING_PASSWORD
}
else {
    $pfxPassword = ""
}

$makeappx = Get-WindowsSdkTool "makeappx.exe"
$signtool = Get-WindowsSdkTool "signtool.exe"

$stagingRoot = Split-Path -Parent $outPath
if (-not $stagingRoot) {
    $stagingRoot = $releaseDir
}
$staging = Join-Path $stagingRoot "msix-staging"
if (Test-Path -LiteralPath $staging) {
    Remove-Item -Recurse -Force -LiteralPath $staging
}
$assetsDir = Join-Path $staging "Assets"
New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null

Copy-Item -Force -LiteralPath $binary -Destination (Join-Path $staging "FieldAssist.exe")
Copy-Item -Force -LiteralPath $fieldPlayBin -Destination (Join-Path $staging "field-play.exe")
Copy-Item -Force -LiteralPath $fieldBatchBin -Destination (Join-Path $staging "field-batch.exe")

$manifestText = (Get-Content -Raw -LiteralPath $manifestTemplate) -replace '__VERSION__', $msixVersion
$manifestOut = Join-Path $staging "AppxManifest.xml"
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($manifestOut, $manifestText, $utf8NoBom)

Resize-Png -Source $iconPng -Destination (Join-Path $assetsDir "Square44x44Logo.png") -Size 44
Resize-Png -Source $iconPng -Destination (Join-Path $assetsDir "Square150x150Logo.png") -Size 150
Resize-Png -Source $iconPng -Destination (Join-Path $assetsDir "StoreLogo.png") -Size 50

if (Test-Path -LiteralPath $outPath) {
    Remove-Item -Force -LiteralPath $outPath
}

Write-Host "Packing MSIX ($msixVersion)..."
& $makeappx pack /d $staging /p $outPath /o
if ($LASTEXITCODE -ne 0) {
    Write-Error "makeappx failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "Signing $outPath..."
$signArgs = @(
    "sign",
    "/fd", "SHA256",
    "/a",
    "/f", $pfxPath
)
if ($pfxPassword -ne "") {
    $signArgs += @("/p", $pfxPassword)
}
$signArgs += $outPath
& $signtool @signArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "signtool failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "Created $outPath"

if (-not $install) {
    exit 0
}

if (-not (Test-Path -LiteralPath $cerPath)) {
    Write-Error "missing publisher certificate for trust: $cerPath"
    exit 1
}

Ensure-PublisherTrusted -CerPath $cerPath -MsixPath $outPath
Write-Host "Installing $outPath for current user..."
Add-AppxPackage -Path $outPath -ForceUpdateFromAnyVersion
Write-Host "Installed FieldAssist $cargoVersion (MSIX $msixVersion)"
Write-Host "Uninstall with: Get-AppxPackage FieldAssist | Remove-AppxPackage"
