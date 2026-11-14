# Build a release binary to share (Windows, PowerShell):  .\scripts\release.ps1
#
# Rust embeds the full path of every source file it compiles (for panic messages), which would
# put your user name and folders into the .exe ("C:\Users\<you>\.cargo\..."). This maps those
# paths to neutral names first. Output: target\release\sigit.exe
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path "$PSScriptRoot\..").Path
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $HOME '.rustup' }
# When several prefixes match, rustc uses the last: general first, specific last.
$env:RUSTFLAGS = @(
    "--remap-path-prefix=$HOME=home"
    "--remap-path-prefix=$cargoHome=cargo"
    "--remap-path-prefix=$rustupHome=rustup"
    "--remap-path-prefix=$project=src"
) -join ' '
Push-Location $project
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    # Check: no trace of this machine's user folder may be left in the binary.
    $exe = Join-Path $project 'target\release\sigit.exe'
    $bytes = [System.IO.File]::ReadAllBytes($exe)
    $text = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($text.IndexOf($HOME, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        Write-Error "The binary still contains your home folder path; don't share it."
    }
    Write-Host "Built $exe (no local paths inside)"
} finally {
    Pop-Location
}
