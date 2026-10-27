#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('x86_64-pc-windows-gnu', 'x86_64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$oldFlags = $env:RUSTFLAGS
$oldEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
$oldTargetDir = $env:CARGO_TARGET_DIR
Push-Location $root
try {
    $env:RUSTFLAGS = "$oldFlags -D warnings".Trim()
    if ($oldEncodedFlags) {
        $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedFlags + [char]31 + '-Dwarnings'
    }
    $env:CARGO_TARGET_DIR = Join-Path $root 'target'
    foreach ($script in @('demo\Invoke-ShadowRealmDemo.ps1', 'scripts\Test.ps1', 'scripts\Package.ps1')) {
        $tokens = $null
        $errors = $null
        [void][Management.Automation.Language.Parser]::ParseFile(
            (Join-Path $root $script), [ref]$tokens, [ref]$errors)
        if ($errors.Count) { throw "PowerShell syntax errors in ${script}: $errors" }
    }
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    & cargo clippy --locked --offline --target $Target --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed; install the matching toolchain component if missing.' }
    & cargo test --locked --offline --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Windows unit tests failed.' }
    & cargo build --release --locked --offline --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
    $executable = Join-Path $root "target\$Target\release\shadow-realm.exe"
    $oldPath = $env:PATH
    try {
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
        $help = & $executable --help
        if ($LASTEXITCODE -ne 0) { throw 'Standalone executable help check failed.' }
    } finally { $env:PATH = $oldPath }
    $metadata = & cargo metadata --locked --offline --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $version = ($metadata | ConvertFrom-Json).packages[0].version
    if ($help[0] -cne "Shadow Realm $version -- DISPOSABLE WINDOWS VM ONLY") {
        throw 'Executable version/help mismatch.'
    }
    $demo = Get-Content -LiteralPath '.\demo\Invoke-ShadowRealmDemo.ps1' -Raw
    if (!$demo.Contains(([regex]::Escape($version))) -or
        !$demo.Contains("'--remount-snapshot') 'snapshot-deletion'")) {
        throw 'Demo version or remount invocation does not match the release.'
    }
    if (!$demo.Contains('$ToolPath = Join-Path (Split-Path $PSScriptRoot -Parent) ''shadow-realm.exe''')) {
        throw 'Demo default executable path does not match the release layout.'
    }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = 'powershell.exe'
    $info.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
        (Join-Path $root 'demo\Invoke-ShadowRealmDemo.ps1') + '"'
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (!$process.Start()) { throw 'Could not start demo consent-gate check.' }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        if ($process.ExitCode -eq 0 -or $stderr.Result -notmatch 'Use -ConfirmLab only inside a disposable Windows VM') {
            throw "Demo did not stop at its consent gate: $($stdout.Result) $($stderr.Result)"
        }
    } finally {
        $process.Dispose()
    }
    Write-Host 'Verification passed. No live VSS operations or disk-creating demo were run.'
} finally {
    $env:RUSTFLAGS = $oldFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedFlags
    $env:CARGO_TARGET_DIR = $oldTargetDir
    Pop-Location
}
