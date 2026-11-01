#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('x86_64-pc-windows-gnu', 'x86_64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$oldFlags = $env:RUSTFLAGS
$oldEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
$oldTargetDir = $env:CARGO_TARGET_DIR
Push-Location $root
try {
    $status = & git status --porcelain --untracked-files=all
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Packaging requires a clean committed Git checkout.' }
    $commit = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source commit.' }
    $commitTime = & git show -s --format=%cI HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source timestamp.' }
    $compiler = & rustc --version
    if ($LASTEXITCODE -ne 0 -or $compiler -notmatch '^rustc 1\.99\.1 ') {
        throw 'Release packaging requires Rust 1.99.1.'
    }
    $cargo = & cargo --version
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Cargo version.' }
    $sysroot = & rustc --print sysroot
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify compiler sysroot.' }
    $fetch = @('fetch', '--locked', '--target', $Target)
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Dependency fetch failed; offline packaging requires a populated Cargo cache.' }
    $metadataJson = & cargo metadata --locked --offline --format-version 1 --filter-platform $Target
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo metadata.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages | Where-Object id -eq $metadata.resolve.root
    $version = $package.version
    $resolved = @($metadata.resolve.nodes | ForEach-Object id)
    $dependencies = @($metadata.packages |
        Where-Object { $_.id -ne $metadata.resolve.root -and $_.id -in $resolved } |
        Sort-Object name | Select-Object name, version, license)
    $notices = Get-Content -LiteralPath (Join-Path $root 'THIRD-PARTY-NOTICES.md') -Raw
    foreach ($dependency in $dependencies) {
        if (!$notices.Contains("| $($dependency.name) | $($dependency.version) |")) {
            throw "Third-party notices must cover $($dependency.name) $($dependency.version)."
        }
    }
    if ($version -notmatch '^\d+\.\d+\.\d+-experimental\.\d+$') {
        throw 'Only explicitly experimental versions may be packaged.'
    }
    $env:CARGO_TARGET_DIR = Join-Path $root 'target'
    $env:RUSTFLAGS = $null
    $env:CARGO_ENCODED_RUSTFLAGS = @(
        "--remap-path-prefix=$env:USERPROFILE=user",
        "--remap-path-prefix=$sysroot=rust",
        "--remap-path-prefix=$root=source",
        '-Dwarnings'
    ) -join [char]31
    & (Join-Path $PSScriptRoot 'Test.ps1') -Target $Target -Offline
    $executable = Join-Path $root "target\$Target\release\shadow-realm.exe"
    $binary = [IO.File]::ReadAllBytes($executable)
    $pe = [BitConverter]::ToInt32($binary, 0x3c)
    if ($binary[0] -ne 0x4d -or $binary[1] -ne 0x5a -or
        [BitConverter]::ToUInt32($binary, $pe) -ne 0x4550 -or
        [BitConverter]::ToUInt16($binary, $pe + 4) -ne 0x8664) {
        throw 'Release executable is not a Windows x64 PE.'
    }
    $ascii = [Text.Encoding]::ASCII.GetString($binary)
    $wide = [Text.Encoding]::Unicode.GetString($binary)
    foreach ($privatePath in @($root, $sysroot, $env:USERPROFILE)) {
        foreach ($needle in @($privatePath, $privatePath.Replace('\', '/'))) {
            if ($ascii.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                $wide.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw 'A private build path is embedded in the executable.'
            }
        }
    }
    $files = @(
        'Cargo.toml', 'Cargo.lock', 'LICENSE', 'THIRD-PARTY-NOTICES.md', 'README.md', 'RELEASE-NOTES.md',
        'demo\README.md', 'demo\RESEARCH.md', 'demo\Invoke-ShadowRealmDemo.ps1',
        'src\main.rs', 'src\app.rs', 'src\cli.rs', 'src\error.rs',
        'src\paths.rs', 'src\snapshot.rs', 'src\windows\mod.rs',
        'src\windows\handles.rs', 'src\windows\volume.rs',
        'src\windows\files.rs', 'src\windows\marker.rs',
        'tests\cli.rs',
        'tests\unit\app.rs', 'tests\unit\cli.rs', 'tests\unit\error.rs', 'tests\unit\paths.rs', 'tests\unit\snapshot.rs',
        'tests\unit\windows\files.rs', 'tests\unit\windows\handles.rs',
        'tests\unit\windows\marker.rs', 'tests\unit\windows\volume.rs',
        'scripts\Test.ps1', 'scripts\Test-LiveVss.ps1', 'scripts\Package.ps1'
    )
    $rustSources = @(Get-ChildItem -LiteralPath (Join-Path $root 'src'), (Join-Path $root 'tests') -Filter '*.rs' -File -Recurse |
        ForEach-Object { $_.FullName.Substring($root.Length + 1) })
    if (@(Compare-Object @($files | Where-Object { $_.EndsWith('.rs') }) $rustSources).Count) {
        throw 'Release allowlist does not match the Rust source and test files.'
    }
    $entries = [ordered]@{}
    foreach ($file in $files) {
        $entries[$file.Replace('\', '/')] = [IO.File]::ReadAllBytes((Join-Path $root $file))
    }
    $entries['shadow-realm.exe'] = $binary
    $provenance = [ordered]@{
        version = $version
        license = 'MIT'
        dependencies = $dependencies
        source_commit = $commit
        source_committed_at = $commitTime
        target = $Target
        rustc = $compiler
        cargo = $cargo
        build = "cargo build --release --locked --offline --target $Target"
        flags = 'warnings denied; symbols stripped; workspace, sysroot and user-profile paths remapped'
        validation = 'Windows unit and CLI tests, Clippy, rustfmt, PowerShell syntax and consent gate; no live VSS test'
        experimental = $true
        authenticode_signed = $false
    } | ConvertTo-Json -Depth 4
    $utf8 = New-Object Text.UTF8Encoding($false)
    $entries['provenance.json'] = $utf8.GetBytes($provenance + "`n")
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $checksums = foreach ($name in $entries.Keys) {
            $hash = [BitConverter]::ToString($sha.ComputeHash($entries[$name])).Replace('-', '').ToLowerInvariant()
            "$hash  $name"
        }
    } finally { $sha.Dispose() }
    $entries['SHA256SUMS.txt'] = $utf8.GetBytes(($checksums -join "`n") + "`n")
    New-Item -ItemType Directory -Force -Path '.\dist' | Out-Null
    $archiveName = "shadow-realm-$version-windows-x64.zip"
    $archivePath = Join-Path $root "dist\$archiveName"
    if (Test-Path -LiteralPath $archivePath) { throw 'Archive already exists; do not overwrite a published asset.' }
    Add-Type -AssemblyName System.IO.Compression
    $stream = [IO.File]::Open($archivePath, [IO.FileMode]::CreateNew)
    $archive = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Create, $false)
    try {
        foreach ($name in $entries.Keys) {
            $entry = $archive.CreateEntry($name, [IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset]::Parse($commitTime)
            $output = $entry.Open()
            try { $output.Write($entries[$name], 0, $entries[$name].Length) } finally { $output.Dispose() }
        }
    } finally { $archive.Dispose(); $stream.Dispose() }
    $stream = [IO.File]::OpenRead($archivePath)
    $archive = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Read, $false)
    try {
        $names = @($archive.Entries | ForEach-Object FullName)
        if ($names.Count -ne $entries.Count -or
            @(Compare-Object @($entries.Keys) $names).Count) { throw 'Archive allowlist mismatch.' }
        foreach ($entry in $archive.Entries) {
            $input = $entry.Open()
            $buffer = New-Object IO.MemoryStream
            try {
                $input.CopyTo($buffer)
                if ([Convert]::ToBase64String($buffer.ToArray()) -cne
                    [Convert]::ToBase64String($entries[$entry.FullName])) {
                    throw "Archive content mismatch: $($entry.FullName)"
                }
            } finally { $input.Dispose(); $buffer.Dispose() }
        }
    } finally { $archive.Dispose(); $stream.Dispose() }
    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $root 'dist\SHA256SUMS.txt'), "$hash  $archiveName`n", $utf8)
    Write-Host "Verified package: dist\$archiveName ($($entries.Count) allowlisted entries)"
    Write-Host "SHA256: $hash"
} finally {
    $env:RUSTFLAGS = $oldFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedFlags
    $env:CARGO_TARGET_DIR = $oldTargetDir
    Pop-Location
}
