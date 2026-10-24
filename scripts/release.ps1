<#
.SYNOPSIS
    Builds Wflow and its Windows installer from one version number.

.DESCRIPTION
    Cargo.toml is the only file that stores the version. Everything else is
    derived: build.rs reads CARGO_PKG_VERSION for the file properties, and
    installer.iss carries an @VERSION@ placeholder that CI substitutes before
    calling Inno Setup. Doing it by hand is how a release ends up with the
    binary, the installer name and the tag disagreeing, so this script keeps
    them in step:

      * optionally bump the version in Cargo.toml
      * cargo build --release
      * fill in the installer template and compile it into dist\
      * report the SHA256, which the release notes ask for
      * optionally commit the bump and create the tag

    The temporary .iss is written next to the real one and removed afterwards,
    so installer.iss never stops being a template.

.EXAMPLE
    ./scripts/release.ps1
    Build the installer for the version already in Cargo.toml.

.EXAMPLE
    ./scripts/release.ps1 -Bump patch -Commit -Tag
    0.3.10 -> 0.3.11, build, then "release: 0.3.11" and tag v0.3.11.
#>
[CmdletBinding()]
param(
    # Bump this component of the version before building. "none" builds what
    # Cargo.toml already says.
    [ValidateSet('none', 'patch', 'minor', 'major')]
    [string] $Bump = 'none',

    # Commit the version change as "release: <version>".
    [switch] $Commit,

    # Tag the commit as v<version>. Implies nothing about pushing; run git push
    # yourself once the build is verified.
    [switch] $Tag,

    # Skip cargo build (useful when the binary was just built).
    [switch] $SkipBuild,

    # Full path to ISCC.exe. Auto-detected when omitted.
    [string] $Iscc = ''
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Get-CargoVersion {
    $lines = Get-Content Cargo.toml
    # Only the line at the very start of a column belongs to [package]; every
    # dependency writes its version indented inside a table.
    $line = $lines | Where-Object { $_ -match '^version\s*=\s*"([^"]+)"' } | Select-Object -First 1
    if (-not $line) { throw 'Could not find a version in Cargo.toml.' }
    return [regex]::Match($line, '^version\s*=\s*"([^"]+)"').Groups[1].Value
}

function Set-CargoVersion([string] $Version) {
    $lines = @(Get-Content Cargo.toml)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^version\s*=\s*"') {
            $lines[$i] = "version = `"$Version`""
            break
        }
    }
    Set-Content -Path Cargo.toml -Value $lines -Encoding utf8
}

function Find-Iscc {
    param([string] $Explicit)
    if ($Explicit) {
        if (-not (Test-Path $Explicit)) { throw "ISCC not found at $Explicit" }
        return $Explicit
    }

    # Inno Setup installs machine-wide by default, but the common per-user
    # location is %LOCALAPPDATA%\Programs, and the version suffix moves
    # between releases (Inno Setup 6, Inno Setup 7), so probe by wildcard.
    $roots = @(
        'C:\Program Files (x86)',
        'C:\Program Files',
        (Join-Path $env:LOCALAPPDATA 'Programs')
    ) | Where-Object { $_ -and (Test-Path $_) }

    foreach ($root in $roots) {
        $hit = Get-ChildItem $root -Directory -Filter 'Inno Setup*' -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName 'ISCC.exe' } |
            Where-Object { Test-Path $_ } |
            Select-Object -First 1
        if ($hit) { return $hit }
    }

    $cmd = Get-Command iscc -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }

    throw @'
ISCC.exe not found. Install Inno Setup 6 (https://jrsoftware.org/isinfo.php),
or pass the full path:  ./scripts/release.ps1 -Iscc "C:\path\to\ISCC.exe"
'@
}

$before = Get-CargoVersion
Write-Host "Cargo.toml version: v$before"

$version = $before
if ($Bump -ne 'none') {
    $parts = $before.Split('.')
    if ($parts.Count -lt 3) { throw "Expected a major.minor.patch version, got '$before'." }
    $major, $minor, $patch = [int] $parts[0], [int] $parts[1], [int] $parts[2]
    switch ($Bump) {
        'major' { $major++; $minor = 0; $patch = 0 }
        'minor' { $minor++; $patch = 0 }
        'patch' { $patch++ }
    }
    $version = "$major.$minor.$patch"
    Set-CargoVersion $version
    Write-Host "Bumped v$before -> v$version" -ForegroundColor Green
}

if (-not $SkipBuild) {
    Write-Host 'cargo build --release ...'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed.' }
    # cargo rewrites Cargo.lock when the version moved; keep it in step.
    if ($version -ne $before) { cargo check --quiet | Out-Null }
}

$isccPath = Find-Iscc $Iscc
Write-Host "Inno Setup: $isccPath"

# Fill in the template instead of editing installer.iss in place.
$template = Get-Content installer.iss -Raw
$rendered = Join-Path (Get-Location) "installer-build-$version.iss"
Set-Content -Path $rendered -Value ($template -replace '@VERSION@', $version) -Encoding utf8

try {
    & $isccPath $rendered
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed.' }
}
finally {
    Remove-Item $rendered -Force -ErrorAction SilentlyContinue
}

$setup = "dist\Wflow-Setup-$version.exe"
if (-not (Test-Path $setup)) { throw "Expected $setup to exist after compiling the installer." }

$item = Get-Item $setup
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash
Write-Host ''
Write-Host ("Built {0}  ({1:N1} MB)" -f $item.Name, ($item.Length / 1MB)) -ForegroundColor Green
Write-Host "SHA256 $hash"
Write-Host 'Paste that checksum into the release notes; the updater verifies it.'

if ($Commit) {
    git add Cargo.toml Cargo.lock
    if ($version -ne $before) {
        git commit -m "release: $version"
    }
    else {
        Write-Host 'Nothing to commit: the version was not bumped.' -ForegroundColor Yellow
    }
}

if ($Tag) {
    if (-not $Commit) { Write-Host 'Tagging without committing the bump; the tag points at the current HEAD.' -ForegroundColor Yellow }
    git tag "v$version"
    Write-Host "Tagged v$version. Push with: git push && git push --tags"
}
