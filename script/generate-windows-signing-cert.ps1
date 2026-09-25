# SPDX-FileCopyrightText: 2026 Greg Wuller
# SPDX-License-Identifier: MIT
#
# Create the stable FieldAssist code-signing certificate used for MSIX
# packages. Run once; reuse the same PFX for all future versions so upgrades
# replace the installed package.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Test-IsWindows {
    if ($null -ne (Get-Variable -Name IsWindows -Scope Global -ErrorAction SilentlyContinue)) {
        return [bool]$IsWindows
    }
    return $env:OS -eq "Windows_NT"
}

function Show-Usage {
    @"
Usage: script/generate-windows-signing-cert.ps1 [options]

Generate CN=FieldAssist code-signing certificate for MSIX packages.

  --force         Overwrite an existing FieldAssist.pfx / FieldAssist.cer
  --password P    PFX password (default: empty, or FIELDASSIST_SIGNING_PASSWORD)
  -h, --help      Show this help

Writes:
  crates/field-assist/assets/windows/FieldAssist.pfx  (gitignored; private key)
  crates/field-assist/assets/windows/FieldAssist.cer  (public; commit this)

After generating, store the PFX as GitHub secrets for CI:
  WINDOWS_SIGNING_PFX       base64 of the .pfx bytes
  WINDOWS_SIGNING_PASSWORD  the password used here (empty string if none)
"@
}

if (-not (Test-IsWindows)) {
    Write-Error "script/generate-windows-signing-cert.ps1 only runs on Windows"
    exit 1
}

$force = $false
$passwordPlain = $null

for ($i = 0; $i -lt $args.Count; $i++) {
    switch ($args[$i]) {
        "--force" { $force = $true }
        "--password" {
            $i++
            if ($i -ge $args.Count) {
                Write-Error "--password requires a value"
                exit 1
            }
            $passwordPlain = $args[$i]
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
$outDir = Join-Path $root "crates\field-assist\assets\windows"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$pfxPath = Join-Path $outDir "FieldAssist.pfx"
$cerPath = Join-Path $outDir "FieldAssist.cer"

if ((Test-Path -LiteralPath $pfxPath) -or (Test-Path -LiteralPath $cerPath)) {
    if (-not $force) {
        Write-Error @"
signing materials already exist:
  $pfxPath
  $cerPath

Reusing a different certificate breaks MSIX upgrades. Pass --force only if
you intend to replace the publisher identity for everyone.
"@
        exit 1
    }
    Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $pfxPath
    Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $cerPath
}

if ($null -eq $passwordPlain) {
    if ($null -ne $env:FIELDASSIST_SIGNING_PASSWORD) {
        $passwordPlain = $env:FIELDASSIST_SIGNING_PASSWORD
    }
    else {
        $passwordPlain = ""
    }
}

Write-Host "Creating self-signed code-signing certificate (CN=FieldAssist)..."
# Long validity: replacing the publisher certificate breaks MSIX upgrades
# for everyone who already trusted the previous one.
$notAfter = (Get-Date).AddYears(10)
$cert = New-SelfSignedCertificate `
    -Type Custom `
    -Subject "CN=FieldAssist" `
    -KeyUsage DigitalSignature `
    -FriendlyName "FieldAssist MSIX" `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -NotAfter $notAfter `
    -TextExtension @(
        "2.5.29.37={text}1.3.6.1.5.5.7.3.3",
        "2.5.29.19={text}"
    )

if ($passwordPlain -eq "") {
    $securePassword = New-Object System.Security.SecureString
}
else {
    $securePassword = ConvertTo-SecureString -String $passwordPlain -Force -AsPlainText
}
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $securePassword | Out-Null
Export-Certificate -Cert $cert -FilePath $cerPath -Type CERT | Out-Null

# Keep the private key only in the PFX file we just wrote.
Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($cert.Thumbprint)" -Force

Write-Host "Wrote $pfxPath"
Write-Host "Wrote $cerPath"
Write-Host "Thumbprint: $($cert.Thumbprint)"
Write-Host @"

Next steps:
  1. Commit FieldAssist.cer (public). Keep FieldAssist.pfx private.
  2. For local packs: set FIELDASSIST_SIGNING_PFX to the .pfx path
     (or leave the default next to the .cer) and optionally
     FIELDASSIST_SIGNING_PASSWORD.
  3. For CI, add GitHub secrets WINDOWS_SIGNING_PFX (base64 of the
     .pfx bytes) and WINDOWS_SIGNING_PASSWORD (empty if none).
"@
