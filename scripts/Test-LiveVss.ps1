#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$ConfirmLab,
    [string]$ToolPath,
    [string]$OutputDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$ConfirmLab) {
    throw 'Use -ConfirmLab only inside a disposable Windows VM. This test changes snapshots of its existing OS volume.'
}
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Live VSS tests require an elevated terminal inside the disposable VM.'
}
if ($env:GITHUB_ACTIONS -eq 'true' -and $env:SHADOW_REALM_RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Live CI tests must run on a disposable GitHub-hosted runner, not a self-hosted machine.'
}
if (!$ToolPath) {
    $ToolPath = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\x86_64-pc-windows-msvc\release\shadow-realm.exe'
}
$ToolPath = (Resolve-Path -LiteralPath $ToolPath).ProviderPath
if (!(Test-Path -LiteralPath $ToolPath -PathType Leaf)) { throw 'ToolPath must identify the built executable.' }
if (!$OutputDirectory) {
    $OutputDirectory = Join-Path $env:TEMP ('ShadowRealm-LiveVss-' + [guid]::NewGuid().ToString('N'))
}
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must be new; existing evidence will not be overwritten.' }
$run = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$started = Get-Date
$phase = 'preflight'
$target = $null
$control = $null
$sequence = 0

function Save-Json([string]$Name, $Value) {
    ConvertTo-Json -InputObject $Value -Depth 8 |
        Set-Content -LiteralPath (Join-Path $script:run $Name) -Encoding UTF8
}

function Invoke-Tool([string[]]$Arguments, [string]$Label) {
    $script:sequence++
    $name = '{0:D2}-{1}' -f $script:sequence, $Label
    Save-Json "$name-command.json" $Arguments
    $quoted = foreach ($argument in $Arguments) {
        if ($argument.Contains('"') -or $argument.Contains("`r") -or $argument.Contains("`n") -or $argument.EndsWith('\')) {
            throw 'Cannot safely quote a native command argument.'
        }
        '"' + $argument + '"'
    }
    $stdout = Join-Path $script:run "$name-stdout.txt"
    $stderr = Join-Path $script:run "$name-stderr.txt"
    $process = Start-Process -FilePath $script:ToolPath -ArgumentList $quoted -WorkingDirectory $script:run `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -NoNewWindow -Wait -PassThru
    try {
        $result = [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = [IO.File]::ReadAllText($stdout)
            Stderr = [IO.File]::ReadAllText($stderr)
        }
    } finally { $process.Dispose() }
    Save-Json "$name-result.json" $result
    Write-Host "$Label exited $($result.ExitCode)"
    if ($result.Stdout) { Write-Host $result.Stdout.TrimEnd() }
    if ($result.Stderr) { Write-Host $result.Stderr.TrimEnd() }
    return $result
}

function Invoke-Checked([string[]]$Arguments, [string]$Label) {
    if ($Arguments -contains '--confirm-lab') {
        Assert-Snapshot $script:target
        Assert-Snapshot $script:control
    }
    $result = Invoke-Tool $Arguments $Label
    if ($result.ExitCode -ne 0) { throw "$Label failed with exit code $($result.ExitCode): $($result.Stderr)" }
    return $result
}

function Read-Hash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try { return (Get-FileHash -InputStream $stream -Algorithm SHA256).Hash } finally { $stream.Dispose() }
}

function Read-Fixtures([string]$Root) {
    # Enumerate before hashing so inaccessible directories cannot look like absent files.
    if ([IO.Directory]::GetDirectories($Root).Length -ne 0) { throw "Unexpected fixture subdirectory: $Root" }
    $state = [ordered]@{}
    foreach ($path in ([IO.Directory]::GetFiles($Root) | Sort-Object)) {
        $state[[IO.Path]::GetFileName($path)] = Read-Hash $path
    }
    return $state
}

function Assert-Fixtures([string]$Root, $Expected) {
    $observed = Read-Fixtures $Root
    if ($observed.Count -ne $Expected.Count) { throw "Fixture count changed unexpectedly at $Root" }
    foreach ($name in $Expected.Keys) {
        if (!$observed.Contains($name) -or $observed[$name] -cne $Expected[$name]) {
            throw "Fixture presence/hash mismatch: $Root\$name"
        }
    }
}

function Assert-Snapshot($Snapshot) {
    $current = @(Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $Snapshot.ID)
    if ($current.Count -ne 1 -or $current[0].DeviceObject -ne $Snapshot.DeviceObject -or
        $current[0].VolumeName -ne $Snapshot.VolumeName -or $current[0].ProviderID -ne $Snapshot.ProviderID) {
        throw 'Snapshot identity, provider, or source-volume mapping changed.'
    }
}

function New-LabSnapshot([string]$Label) {
    $created = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create `
        -Arguments @{ Volume = $script:sourceRoot; Context = 'ClientAccessible' }
    Save-Json "$Label-creation.json" ($created | Select-Object ReturnValue, ShadowID)
    if ($created.ReturnValue -ne 0) { throw "$Label snapshot creation failed: VSS return code $($created.ReturnValue)." }
    $snapshot = @(Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $created.ShadowID)
    if ($snapshot.Count -ne 1 -or $snapshot[0].VolumeName -ne $script:volume.Path) {
        throw 'Created snapshot could not be tied to the selected OS volume.'
    }
    $snapshot = $snapshot[0]
    if ($snapshot.DeviceObject -notmatch 'HarddiskVolumeShadowCopy([1-9][0-9]*)$') {
        throw 'Created snapshot has an unsupported device path.'
    }
    $number = [uint32]$Matches[1]
    if ($snapshot.DeviceObject -ne ('\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy' + $number)) {
        throw 'Created snapshot is not the expected GLOBALROOT shadow device.'
    }
    $identity = $snapshot | Select-Object ID, ProviderID, VolumeName, DeviceObject,
        @{ Name = 'Number'; Expression = { $number } }
    Save-Json "$Label-snapshot.json" $identity
    return $identity
}

function Read-OnlyAttributes($Snapshot, [string]$Label) {
    Assert-Snapshot $Snapshot
    $result = Invoke-Checked @('debug', 'inspect', '--snapshot', [string]$Snapshot.Number) $Label
    $lines = @($result.Stdout -split '\r?\n')
    $attributeLine = @($lines | Where-Object { $_ -match '^GPT attributes: 0x[0-9a-fA-F]{16}$' })
    if ($attributeLine.Count -ne 1 -or $lines -notcontains 'READ_ONLY: true' -or
        $lines -notcontains 'Driver writable status: false') {
        throw 'Read-only driver state was not confirmed.'
    }
    return ($attributeLine[0] -split ' ')[-1]
}

function Assert-Isolation([string]$Label) {
    Assert-Snapshot $script:control
    if ((Read-OnlyAttributes $script:target "$Label-inspect") -ne $script:originalAttributes) {
        throw 'The target snapshot attributes differ from the original read-only state.'
    }
    Assert-Fixtures $script:liveRoot $script:baseline
    Assert-Fixtures $script:controlRoot $script:baseline
    Assert-Fixtures $script:targetRoot $script:expected
}

function Save-StorageEvents {
    $events = @()
    try {
        $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $script:started; Level = @(1, 2, 3) } |
            Where-Object ProviderName -match 'Ntfs|disk|volsnap|storport|storahci|stornvme' |
            Select-Object TimeCreated, RecordId, ProviderName, Id, Message)
    } catch {
        if ($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }
    }
    Save-Json 'storage-events.json' $events
    return $events
}

try {
    $sourceRoot = $env:SystemDrive + '\'
    $driveLetter = $env:SystemDrive.TrimEnd(':')
    $volume = Get-Volume -DriveLetter $driveLetter
    if ($volume.FileSystem -ne 'NTFS') { throw 'The OS-volume integration test requires NTFS.' }
    $beforeSnapshots = @(Get-CimInstance Win32_ShadowCopy | Select-Object ID, VolumeName, DeviceObject, ProviderID)
    $beforeSnapshotIds = @($beforeSnapshots | ForEach-Object ID)
    Save-Json 'manifest.json' ([ordered]@{
        Started = $started.ToUniversalTime().ToString('o')
        ToolSha256 = (Get-FileHash -LiteralPath $ToolPath -Algorithm SHA256).Hash
        SourceCommit = $env:GITHUB_SHA
        OperatingSystem = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber
        VolsnapVersion = (Get-Item -LiteralPath "$env:SystemRoot\System32\drivers\volsnap.sys").VersionInfo.FileVersion
        Disk = Get-Partition -DriveLetter $driveLetter | Get-Disk | Select-Object Number, FriendlyName, BusType, IsBoot, IsSystem
        SourceVolume = $volume | Select-Object Path, FileSystem, Size, SizeRemaining
        Providers = @(Get-CimInstance Win32_ShadowProvider | Select-Object ID, Name, Type, Version, VersionID)
        ExistingSnapshots = $beforeSnapshots
        PersistenceAcrossRebootTested = $false
    })
    $phase = 'creating owned fixtures'
    $relative = 'ShadowRealmLiveVss-' + [guid]::NewGuid().ToString('N')
    $liveRoot = (New-Item -ItemType Directory -Path (Join-Path $sourceRoot $relative)).FullName
    Save-Json 'fixture-directory.json' ([ordered]@{ Source = $liveRoot; SnapshotRelative = $relative })
    $payload = Join-Path $run 'payload.bin'
    $short = Join-Path $run 'short.bin'
    $empty = Join-Path $run 'empty.bin'
    [IO.File]::WriteAllBytes($payload, [byte[]](0..255))
    [IO.File]::WriteAllBytes($short, [byte[]]@(255, 0))
    [IO.File]::WriteAllBytes($empty, [byte[]]@())
    foreach ($name in @('read.bin', 'update.bin', 'delete.bin', 'sentinel.bin')) {
        [IO.File]::WriteAllBytes((Join-Path $liveRoot $name), [byte[]](0..255))
    }
    $baseline = Read-Fixtures $liveRoot
    $expected = [ordered]@{}
    foreach ($name in $baseline.Keys) { $expected[$name] = $baseline[$name] }
    Save-Json 'baseline.json' $baseline
    $phase = 'creating control and target snapshots'
    $control = New-LabSnapshot 'control'
    $target = New-LabSnapshot 'target'
    if ($target.ID -eq $control.ID -or $target.ID -in $beforeSnapshotIds -or $control.ID -in $beforeSnapshotIds) {
        throw 'Snapshot creation did not produce two new, distinct identities.'
    }
    $controlRoot = $control.DeviceObject + '\' + $relative
    $targetRoot = $target.DeviceObject + '\' + $relative
    $number = [string]$target.Number
    $originalAttributes = Read-OnlyAttributes $target 'baseline-inspect'
    $controlAttributes = Read-OnlyAttributes $control 'control-inspect'
    Assert-Isolation 'baseline'

    $phase = 'read and dry-run checks'
    $export = Join-Path $run 'read-export.bin'
    $null = Invoke-Checked @('read', '--snapshot', $number, '--file', "$relative\read.bin", '--output', $export) 'read'
    if ((Read-Hash $export) -cne $baseline['read.bin']) { throw 'Read export differs from the original bytes.' }
    Assert-Isolation 'read'
    foreach ($command in @('create', 'update', 'delete')) {
        $file = if ($command -eq 'create') { 'new.bin' } else { "$command.bin" }
        $arguments = @($command, '--snapshot', $number, '--file', "$relative\$file", '--dry-run')
        if ($command -ne 'delete') { $arguments += @('--source', $payload) }
        $null = Invoke-Checked $arguments "$command-dry-run"
        Assert-Isolation "$command-dry-run"
    }

    $phase = 'writable probe'
    $null = Invoke-Checked @('debug', 'probe', '--snapshot', $number, '--confirm-lab') 'probe'
    Assert-Isolation 'probe'
    $phase = 'create'
    $null = Invoke-Checked @('create', '--snapshot', $number, '--file', "$relative\new.bin", '--source', $payload, '--confirm-lab') 'create'
    $expected['new.bin'] = Read-Hash $payload
    Assert-Isolation 'create'
    $export = Join-Path $run 'created-export.bin'
    $null = Invoke-Checked @('read', '--snapshot', $number, '--file', "$relative\new.bin", '--output', $export) 'read-created'
    if ((Read-Hash $export) -cne $expected['new.bin']) { throw 'Created-file export differs from source bytes.' }
    foreach ($source in @($short, $empty)) {
        $phase = 'update-' + [IO.Path]::GetFileNameWithoutExtension($source)
        $null = Invoke-Checked @('update', '--snapshot', $number, '--file', "$relative\update.bin", '--source', $source, '--confirm-lab') $phase
        $expected['update.bin'] = Read-Hash $source
        Assert-Isolation $phase
    }
    $phase = 'delete'
    $null = Invoke-Checked @('delete', '--snapshot', $number, '--file', "$relative\delete.bin", '--confirm-lab', '--confirm-delete') 'delete'
    $expected.Remove('delete.bin')
    Assert-Isolation 'delete'

    $phase = 'delayed state and storage-event checks'
    Start-Sleep -Seconds 15
    Assert-Isolation 'delayed'
    if ((Read-OnlyAttributes $control 'control-final-inspect') -ne $controlAttributes) {
        throw 'The control snapshot attributes changed.'
    }
    $afterSnapshots = @(Get-CimInstance Win32_ShadowCopy)
    foreach ($before in $beforeSnapshots) {
        if (@($afterSnapshots | Where-Object {
            $_.ID -eq $before.ID -and $_.VolumeName -eq $before.VolumeName -and
            $_.DeviceObject -eq $before.DeviceObject -and $_.ProviderID -eq $before.ProviderID
        }).Count -ne 1) { throw 'An existing snapshot identity changed or disappeared.' }
    }
    $events = @(Save-StorageEvents)
    if ($events.Count) { throw 'New storage/filesystem warnings were recorded; inspect storage-events.json before claiming compatibility.' }
    Save-Json 'result.json' ([ordered]@{
        Success = $true; Completed = [DateTime]::UtcNow.ToString('o')
        Target = $target; Control = $control; OriginalAttributes = $originalAttributes
        ExpectedTargetFiles = $expected; SourceAndControlHashesUnchanged = $true
        PersistenceAcrossRebootTested = $false
    })
    Write-Host "LIVE VSS TEST PASSED for this configuration only. Evidence: $run"
} catch {
    $failure = $_
    Save-Json 'failure.json' ([ordered]@{
        Success = $false; Phase = $phase; Error = $failure.ToString()
        Details = $failure.ScriptStackTrace; Time = [DateTime]::UtcNow.ToString('o')
        Target = $target; Control = $control
    })
    try {
        $null = Save-StorageEvents
        if ($null -ne $target) {
            Assert-Snapshot $target
            $null = Invoke-Tool @('debug', 'inspect', '--snapshot', [string]$target.Number) 'failure-inspect'
        }
    } catch {
        Save-Json 'diagnostic-failure.json' ([ordered]@{ Error = $_.ToString(); Details = $_.ScriptStackTrace })
        Write-Warning "Failure diagnostics also failed: $_"
    }
    Write-Host "LIVE VSS TEST FAILED during '$phase'. Evidence retained at $run. No force, retry, or cleanup mutation was attempted."
    throw $failure
}
