#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$ConfirmLab,
    [string]$ToolPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (!$ConfirmLab) {
    throw 'Use -ConfirmLab only inside a disposable Windows VM. This demo creates a disk and intentionally changes its snapshot.'
}
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script from an elevated PowerShell terminal.'
}
if ([string]::IsNullOrWhiteSpace($ToolPath)) {
    $ToolPath = Join-Path (Split-Path $PSScriptRoot -Parent) 'shadow-realm.exe'
}
$ToolPath = (Resolve-Path -LiteralPath $ToolPath).ProviderPath
if (!(Test-Path -LiteralPath $ToolPath -PathType Leaf)) { throw 'ToolPath must identify the supplied executable.' }
foreach ($command in 'Get-Disk', 'Initialize-Disk', 'New-Partition', 'Format-Volume', 'Get-CimInstance', 'diskpart.exe', 'vssadmin.exe') {
    Get-Command $command -ErrorAction Stop | Out-Null
}
if (!('ShadowRealmDemo.NativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
namespace ShadowRealmDemo {
    public static class NativeMethods {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.U1)]
        public static extern bool CreateSymbolicLink(string link, string target, uint flags);
    }
}
'@
}

function Invoke-Native {
    param([string]$Executable, [string[]]$Arguments, [string]$LogName)
    $quoted = foreach ($argument in $Arguments) {
        if ($argument.Contains('"') -or $argument.Contains("`r") -or $argument.Contains("`n") -or $argument.EndsWith('\')) {
            throw 'Unexpected native argument; refusing ambiguous quoting.'
        }
        '"' + $argument + '"'
    }
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = $quoted -join ' '
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (!$process.Start()) { throw "Could not start $Executable." }
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $result = [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $outTask.Result
            Stderr = $errTask.Result
        }
    } finally {
        $process.Dispose()
    }
    if ($LogName) {
        $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $script:run "$LogName.json") -Encoding UTF8
    }
    if ($result.Stdout) { Write-Host $result.Stdout.TrimEnd() }
    if ($result.Stderr) { Write-Host $result.Stderr.TrimEnd() -ForegroundColor Yellow }
    return $result
}

function Read-Hash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try { return (Get-FileHash -InputStream $stream -Algorithm SHA256).Hash } finally { $stream.Dispose() }
}

function Assert-LabDisk {
    $matching = @(Get-Disk | Where-Object { $_.Location -eq $script:vhd })
    if ($matching.Count -ne 1 -or $matching[0].UniqueId -ne $script:diskId -or
        $matching[0].Size -ne 2GB -or $matching[0].BusType -ne 'File Backed Virtual' -or
        $matching[0].IsBoot -or $matching[0].IsSystem) {
        throw 'The selected disk is not the dedicated 2 GiB lab VHDX.'
    }
    return $matching[0]
}

function Assert-ReadOnly([string]$LogName) {
    $result = Invoke-Native $ToolPath @('debug', 'inspect', '--snapshot', [string]$script:number) $LogName
    $lines = @($result.Stdout -split '\r?\n')
    if ($result.ExitCode -ne 0 -or $lines -notcontains 'GPT attributes: 0xf000000000000000' -or
        $lines -notcontains 'READ_ONLY: true' -or $lines -notcontains 'Driver writable status: false') {
        throw 'Expected read-only shadow attributes were not confirmed. No automatic recovery will be attempted.'
    }
}

function Show-Stage([string]$Name, [bool]$SourceTargetExpected, [bool]$SnapshotTargetExpected) {
    # Windows PowerShell's provider does not reliably resolve GLOBALROOT paths.
    $sourceItems = @([IO.DirectoryInfo]::new($script:sourceDirectory).GetFileSystemInfos())
    $viewItems = @([IO.DirectoryInfo]::new($script:viewDirectory).GetFileSystemInfos())
    $directItems = @([IO.DirectoryInfo]::new($script:directDirectory).GetFileSystemInfos())
    foreach ($items in @(@{ Files = $sourceItems }, @{ Files = $viewItems }, @{ Files = $directItems })) {
        if (@($items.Files | Where-Object { $_.Attributes -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint) }).Count) {
            throw 'An unexpected directory or reparse point appeared among the test files.'
        }
    }
    $sourceNames = @($sourceItems | ForEach-Object Name | Sort-Object)
    $viewNames = @($viewItems | ForEach-Object Name | Sort-Object)
    $directNames = @($directItems | ForEach-Object Name | Sort-Object)
    $sourceExpected = @($script:controls)
    $snapshotExpected = @($script:controls)
    if ($SourceTargetExpected) { $sourceExpected += $script:targetName }
    if ($SnapshotTargetExpected) { $snapshotExpected += $script:targetName }
    if (($sourceNames -join '|') -ne (($sourceExpected | Sort-Object) -join '|') -or
        ($viewNames -join '|') -ne (($snapshotExpected | Sort-Object) -join '|') -or
        ($directNames -join '|') -ne ($viewNames -join '|')) {
        throw "Unexpected source/snapshot directory contents at stage '$Name'."
    }
    foreach ($fileName in $sourceExpected) {
        if ((Read-Hash (Join-Path $script:sourceDirectory $fileName)) -ne $script:hashes[$fileName]) {
            throw "Source hash changed: $fileName."
        }
    }
    foreach ($fileName in $snapshotExpected) {
        if ((Read-Hash (Join-Path $script:viewDirectory $fileName)) -ne $script:hashes[$fileName] -or
            (Read-Hash (Join-Path $script:directDirectory $fileName)) -ne $script:hashes[$fileName]) {
            throw "Snapshot hash changed: $fileName."
        }
    }
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    Write-Host "SOURCE: $script:sourceDirectory"
    $sourceItems | Select-Object Name, Length | Format-Table -AutoSize | Out-Host
    Write-Host "SNAPSHOT VIEW: $script:viewDirectory"
    $viewItems | Select-Object Name, Length | Format-Table -AutoSize | Out-Host
    $script:stages += [pscustomobject]@{
        Stage = $Name
        SourceFiles = $sourceNames
        SnapshotViewFiles = $viewNames
        DirectSnapshotFiles = $directNames
        SourceTargetPresent = ($sourceNames -contains $script:targetName)
        SnapshotTargetPresent = ($viewNames -contains $script:targetName)
        AllRemainingFileHashesMatchBaseline = $true
    }
    $script:stages | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $script:run 'stages.json') -Encoding UTF8
}

$help = Invoke-Native $ToolPath @('--help') ''
if ($help.ExitCode -ne 0 -or $help.Stdout -notmatch '^Shadow Realm 0\.4\.0-experimental\.1 --') {
    throw 'Use the matching v0.4.0-experimental.1 executable supplied with this demo.'
}
$parent = Join-Path $env:USERPROFILE 'ShadowRealmLab'
$driveRoot = [IO.Path]::GetPathRoot($parent)
if ($driveRoot -notmatch '^[A-Za-z]:\\$') { throw 'The lab directory must be on a local drive.' }
if ((Get-Volume -DriveLetter $driveRoot.Substring(0, 1)).SizeRemaining -lt 5GB) {
    throw 'At least 5 GiB of free space is required for the expandable lab disk and logs.'
}
$run = Join-Path $parent ('demo-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$vhd = Join-Path $run 'demo.vhdx'
$diskId = $null
$number = $null
$stages = @()
$hashes = @{}
$controls = @('keep-one.txt', 'keep-two.txt')
$targetName = 'delete-me.txt'
$phase = 'creating lab directory'
$transcribing = $false
New-Item -ItemType Directory -Path $run | Out-Null

try {
    Start-Transcript -LiteralPath (Join-Path $run 'transcript.txt') | Out-Null
    $transcribing = $true
    Write-Host "Disposable lab directory: $run"
    $originalDisks = @(Get-Disk | Select-Object Number, UniqueId, PartitionStyle, Size)
    $originalSnapshots = @(Get-CimInstance Win32_ShadowCopy | Select-Object ID, VolumeName)
    $originalDisks | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $run 'original-disks.json') -Encoding UTF8
    $originalSnapshots | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $run 'original-snapshots.json') -Encoding UTF8
    $usedLetters = @((Get-PSDrive -PSProvider FileSystem).Name) + @((Get-Volume).DriveLetter) + @((Get-Partition).DriveLetter)
    $letter = @('T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'S', 'R') | Where-Object { $_ -notin $usedLetters } | Select-Object -First 1
    if (!$letter) { throw 'No free lab drive letter is available.' }
    $sourceRoot = "${letter}:\"
    $phase = 'creating and attaching VHDX'
    $diskpartScript = Join-Path $run 'create.diskpart'
    @("create vdisk file=`"$vhd`" maximum=2048 type=expandable", "select vdisk file=`"$vhd`"", 'attach vdisk', 'detail vdisk', 'exit') |
        Set-Content -LiteralPath $diskpartScript -Encoding ascii
    $creation = Invoke-Native 'diskpart.exe' @('/s', $diskpartScript) 'diskpart'
    if ($creation.ExitCode -ne 0 -or !(Test-Path -LiteralPath $vhd -PathType Leaf)) { throw 'VHDX creation failed.' }
    $new = @(Get-Disk | Where-Object { $_.Location -eq $vhd })
    if ($new.Count -ne 1 -or $new[0].PartitionStyle -ne 'RAW' -or $new[0].Number -in $originalDisks.Number) {
        throw 'The VHDX did not map uniquely to a new, uninitialized disk; nothing will be formatted.'
    }
    $diskId = $new[0].UniqueId
    $disk = Assert-LabDisk
    $phase = 'formatting only the new lab disk'
    $disk | Initialize-Disk -PartitionStyle GPT -PassThru | Out-Null
    $disk = Assert-LabDisk
    if (Get-PSDrive -Name $letter -ErrorAction SilentlyContinue) { throw 'The chosen drive letter became occupied.' }
    $partition = New-Partition -DiskNumber $disk.Number -UseMaximumSize -DriveLetter $letter
    $partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SHADOW-REALM' -Confirm:$false | Out-Host
    $volume = Get-Volume -DriveLetter $letter
    if ((Get-Partition -DriveLetter $letter).DiskNumber -ne $disk.Number) { throw 'Lab drive mapping changed.' }

    $phase = 'writing owned test files'
    $sourceDirectory = Join-Path $sourceRoot 'Demo'
    New-Item -ItemType Directory -Path $sourceDirectory | Out-Null
    foreach ($name in @($targetName) + $controls) {
        $path = Join-Path $sourceDirectory $name
        $bytes = [Text.Encoding]::ASCII.GetBytes("Disposable VSS demo file: $name`r`nRun: $run`r`n")
        $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        $hashes[$name] = Read-Hash $path
    }
    $hashes | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'baseline-hashes.json') -Encoding UTF8
    $phase = 'configuring lab-only shadow storage'
    $storage = Invoke-Native 'vssadmin.exe' @('resize', 'shadowstorage', "/For=${letter}:", "/On=${letter}:", '/MaxSize=512MB') 'shadow-storage'
    if ($storage.ExitCode -ne 0) { throw 'Lab shadow-storage setup failed.' }
    $associations = @(Get-CimInstance Win32_ShadowStorage | Where-Object { $_.Volume.DeviceID -eq $volume.Path })
    if ($associations.Count -ne 1 -or $associations[0].DiffVolume.DeviceID -ne $volume.Path -or $associations[0].MaxSpace -ne 512MB) {
        throw 'Shadow storage is not confined to the new lab volume.'
    }
    $phase = 'creating snapshot'
    $created = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create -Arguments @{ Volume = $sourceRoot; Context = 'ClientAccessible' }
    if ($created.ReturnValue -ne 0) { throw "Snapshot creation failed: $($created.ReturnValue)." }
    $snapshot = @(Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $created.ShadowID)
    if ($snapshot.Count -ne 1 -or $snapshot[0].VolumeName -ne $volume.Path -or $snapshot[0].ID -in @($originalSnapshots | ForEach-Object ID)) {
        throw 'New snapshot identity/source validation failed.'
    }
    $snapshot = $snapshot[0]
    if ($snapshot.DeviceObject -notmatch 'HarddiskVolumeShadowCopy([0-9]+)$') { throw 'Unexpected shadow-device path.' }
    $number = [int]$Matches[1]
    $view = Join-Path $run 'SnapshotView'
    if (![ShadowRealmDemo.NativeMethods]::CreateSymbolicLink($view, ($snapshot.DeviceObject + '\'), 1)) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Snapshot-view link creation failed: $([ComponentModel.Win32Exception]::new($errorCode).Message) (Win32 $errorCode)."
    }
    if (!([IO.File]::GetAttributes($view) -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Snapshot view is not a reparse point.'
    }
    $viewDirectory = Join-Path $view 'Demo'
    $directDirectory = $snapshot.DeviceObject + '\Demo'
    [pscustomobject]@{
        RunDirectory = $run; VhdPath = $vhd; DiskUniqueId = $diskId; SourceRoot = $sourceRoot
        SnapshotId = $snapshot.ID; SnapshotDevice = $snapshot.DeviceObject; SnapshotView = $view
        ToolPath = $ToolPath; ToolSha256 = (Get-FileHash -LiteralPath $ToolPath -Algorithm SHA256).Hash
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'manifest.json') -Encoding UTF8
    Assert-ReadOnly 'initial-inspection'
    Show-Stage 'A - snapshot created; all three files exist in both locations' $true $true

    $phase = 'deleting only the owned source target'
    Assert-LabDisk | Out-Null
    if ((Get-Partition -DriveLetter $letter).DiskNumber -ne $disk.Number) { throw 'Source mapping changed.' }
    Remove-Item -LiteralPath (Join-Path $sourceDirectory $targetName)
    Show-Stage 'B - source target deleted; snapshot still contains it' $false $true

    $phase = 'deleting the target from the snapshot with remount restoration'
    $current = @(Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $snapshot.ID)
    if ($current.Count -ne 1 -or $current[0].DeviceObject -ne $snapshot.DeviceObject -or $current[0].VolumeName -ne $volume.Path) {
        throw 'Snapshot mapping changed before deletion.'
    }
    Write-Host 'Do not open SnapshotView in Explorer until this step finishes.'
    $started = Get-Date
    $deletion = Invoke-Native $ToolPath @('delete', '--snapshot', [string]$number, '--file', 'Demo\delete-me.txt', '--confirm-lab', '--confirm-delete') 'snapshot-deletion'
    $state = Invoke-Native $ToolPath @('debug', 'inspect', '--snapshot', [string]$number) 'post-command-inspection'
    if ($deletion.ExitCode -ne 0 -or $state.ExitCode -ne 0) {
        throw 'Snapshot deletion/restoration failed. Stop; preserve the logs. No retry, force-dismount, or attribute-only fallback was attempted.'
    }
    Assert-ReadOnly 'final-inspection'
    Show-Stage 'C - snapshot target deleted; both control files remain' $false $false
    $phase = 'checking delayed state and filesystem warnings'
    Start-Sleep -Seconds 15
    $surviving = @(Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $snapshot.ID)
    if ($surviving.Count -ne 1 -or $surviving[0].DeviceObject -ne $snapshot.DeviceObject -or $surviving[0].VolumeName -ne $volume.Path) {
        throw 'The same snapshot did not remain accessible.'
    }
    Assert-ReadOnly 'delayed-inspection'
    $events = @()
    try {
        $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Ntfs'; Id = 140; StartTime = $started })
    } catch {
        if ($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }
    }
    $warnings = @($events | Where-Object { $_.Message.Contains("HarddiskVolumeShadowCopy$number") -or $_.Message.Contains('SHADOW-REALM') })
    if ($warnings.Count) {
        $warnings | Select-Object TimeCreated, Id, Message | ConvertTo-Json -Depth 3 |
            Set-Content -LiteralPath (Join-Path $run 'ntfs-warnings.json') -Encoding UTF8
        throw 'NTFS flush warnings appeared for the lab. Do not treat this as a clean result.'
    }
    $allSnapshots = @(Get-CimInstance Win32_ShadowCopy)
    foreach ($original in $originalSnapshots) {
        if (@($allSnapshots | Where-Object { $_.ID -eq $original.ID -and $_.VolumeName -eq $original.VolumeName }).Count -ne 1) {
            throw 'An original snapshot identity is missing; investigate before claiming isolation.'
        }
    }
    foreach ($original in $originalDisks) {
        $existing = Get-Disk -Number $original.Number
        if ($existing.UniqueId -ne $original.UniqueId -or $existing.Size -ne $original.Size -or $existing.PartitionStyle -ne $original.PartitionStyle) {
            throw 'An original disk identity/layout changed.'
        }
    }
    [pscustomobject]@{
        Success = $true; CompletedUtc = [DateTime]::UtcNow.ToString('o')
        SnapshotId = $snapshot.ID; SnapshotDevice = $snapshot.DeviceObject
        SourceTargetAbsent = $true; SnapshotTargetAbsent = $true
        ControlsPreserved = $true; DriverReadOnly = $true; LabNtfs140Warnings = 0
        PersistenceAcrossRebootOrDetachTested = $false; Stages = $stages
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $run 'result.json') -Encoding UTF8
    Write-Host "`nDEMO PASSED. The source deletion left the snapshot copy intact; the tool then removed that snapshot copy." -ForegroundColor Green
    Write-Host "View the surviving controls: $viewDirectory"
    Write-Host "Logs and hashes: $run"
    Write-Host 'The VHDX and snapshot remain attached. Reboot/detach persistence and secure erasure are NOT demonstrated.'
    Write-Host 'Do not recursively delete the lab directory: it contains a symbolic link into the snapshot.'
} catch {
    [pscustomobject]@{ Success = $false; Phase = $phase; Error = $_.Exception.Message; RecordedUtc = [DateTime]::UtcNow.ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'failure.json') -Encoding UTF8
    Write-Host "DEMO FAILED during '$phase'. Artifacts retained at $run. No automatic rollback or cleanup was attempted." -ForegroundColor Red
    throw
} finally {
    if ($transcribing) { Stop-Transcript | Out-Null }
}
