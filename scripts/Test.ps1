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
    $fetch = @('fetch', '--locked', '--target', $Target)
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Dependency fetch failed; offline verification requires a populated Cargo cache.' }
    & cargo clippy --locked --offline --target $Target --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed; install the matching toolchain component if missing.' }
    & cargo test --locked --offline --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Windows Rust tests failed.' }
    & cargo build --release --locked --offline --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
    $executable = Join-Path $root "target\$Target\release\shadow-realm.exe"
    $oldPath = $env:PATH
    try {
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
        $help = & $executable --help
        if ($LASTEXITCODE -ne 0) { throw 'Standalone executable help check failed.' }
        $debugHelp = & $executable debug --help
        if ($LASTEXITCODE -ne 0) { throw 'Debug help check failed.' }
        $versionOutput = & $executable --version
        if ($LASTEXITCODE -ne 0) { throw 'Standalone executable version check failed.' }
    } finally { $env:PATH = $oldPath }
    $helpText = $help -join "`n"
    $debugHelpText = $debugHelp -join "`n"
    foreach ($command in @('create', 'read', 'update', 'delete')) {
        if ($helpText -notmatch "(?m)^  $command\s" -or
            $debugHelpText -match "(?m)^  $command\s") {
            throw "Normal help is missing CRUD command: $command"
        }
    }
    foreach ($command in @('inspect', 'diagnose-lock', 'probe', 'restore-readonly')) {
        if ($helpText -match "(?m)^  $command\s" -or
            $debugHelpText -notmatch "(?m)^  $command\s") {
            throw "Maintenance help is not correctly gated: $command"
        }
    }
    if ($helpText.Contains('--remount-snapshot') -or $debugHelpText.Contains('--remount-snapshot')) {
        throw 'Help must not expose the removed remount flag.'
    }
    $metadata = & cargo metadata --locked --offline --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $package = ($metadata | ConvertFrom-Json).packages[0]
    $version = $package.version
    if ($package.license -cne 'MIT' -or
        !(Test-Path -LiteralPath (Join-Path $root 'LICENSE') -PathType Leaf)) {
        throw 'MIT package metadata and the distributed LICENSE file are required.'
    }
    if ($versionOutput -cne "shadow-realm $version" -or
        !$helpText.Contains('DISPOSABLE WINDOWS VM ONLY')) {
        throw 'Executable version or help safety notice mismatch.'
    }
    $demo = Get-Content -LiteralPath '.\demo\Invoke-ShadowRealmDemo.ps1' -Raw
    if (!$demo.Contains(([regex]::Escape($version))) -or
        !$demo.Contains("'--confirm-delete') 'snapshot-deletion'") -or
        !$demo.Contains("@('debug', 'inspect', '--snapshot'") -or
        !$demo.Contains("@('--version')")) {
        throw 'Demo version or CRUD/debug invocation does not match the release.'
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
