# Shadow Realm demo

This optional walkthrough demonstrates the [Shadow Realm utility](../README.md)
using an owned virtual disk. The CLI itself does not depend on this directory.

**Disposable Windows x64 VMs only. Do not run this demo on a normal host.**
Use an elevated terminal in a disposable VM with a checkpoint. Never attach
production disks. Working forensic copies will be intentionally altered;
preserve original images and acquisition hashes elsewhere.

## Prerequisites

- Windows PowerShell 5.1+ and administrator elevation.
- At least 5 GiB free on the user-profile drive.
- Windows storage cmdlets, `diskpart.exe`, `vssadmin.exe`, and CIM support.
- The matching Shadow Realm `0.4.0-experimental.2` executable.

Keep the executable and terminal working directory outside all snapshots.
Close Explorer windows and other tools accessing the selected snapshot before
mutation. Do not terminate unrelated processes or backup services to obtain
a lock.

## Run from an extracted release

The release archive keeps `shadow-realm.exe` at its root and this script under
`demo/`. From the archive root, inside the disposable VM:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\demo\Invoke-ShadowRealmDemo.ps1 -ConfirmLab
```

The process-scoped execution-policy override does not change machine policy.
The default executable path is relative to the script's parent directory,
not the current working directory. Use `-ToolPath` to override it.

## Run from source

Build at the repository root, then invoke the demo with the built executable:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\demo\Invoke-ShadowRealmDemo.ps1 -ConfirmLab -ToolPath .\target\x86_64-pc-windows-msvc\release\shadow-realm.exe
```

For a GNU build, substitute `x86_64-pc-windows-gnu` in both paths.
Without `-ConfirmLab`, the script stops before creating any lab resources.

## Live OS-volume integration test

`scripts\Test-LiveVss.ps1` tests the VM's existing Windows OS volume instead
of creating another VHDX. It creates a uniquely named fixture directory and
two new snapshots: one target and one untouched control. It checks read,
dry-run, probe, create, shorter/empty updates, and delete, comparing fixture
hashes against the live source and control after each mutation.

Run only in an elevated terminal inside a checkpointed disposable VM:

```powershell
.\scripts\Test-LiveVss.ps1 -ConfirmLab -ToolPath .\target\x86_64-pc-windows-msvc\release\shadow-realm.exe
```

The **Shadow Realm Windows verification** workflow runs the `live-vss` job
automatically on pushes and pull requests, alongside the unit/CLI and packaging
jobs. Manual workflow runs also run all checks; no opt-in input is needed.
Live tests run only on a fresh GitHub-hosted `windows-2022` VM, never on a
self-hosted runner. The job uploads command logs, OS/provider/storage
metadata, hashes, inspections, and storage events even when the test fails.
Local runs retain equivalent evidence in a new directory under `%TEMP%`.

The test never forces dismounts, changes shadow-storage quotas, or retries a
failed mutation. It leaves its fixture directory and snapshots for inspection;
discard the VM or revert its checkpoint afterward. Do not treat rejected
snapshot creation or writable transitions as passing tests. A passing run
applies only to the recorded configuration and a short observation window,
not reboot persistence, physical disks, other VSS providers, or production safety.

## What the demonstration does

The script creates a new 2 GiB dynamic Microsoft VHDX under
`$env:USERPROFILE\ShadowRealmLab`, verifies disk identity before formatting,
chooses an unused letter, and creates three files. It labels the volume
`SHADOW-REALM`, configures a 512 MiB self-contained VSS diff area, creates a
snapshot, and exposes a native symbolic-link view (`CreateSymbolicLink`,
not VSCMount).

| Stage | Source | Snapshot |
| --- | --- | --- |
| A: snapshot created | Target and both controls present | Target and both controls present |
| B: owned source target deleted | Only controls remain | Target and both controls remain |
| C: Shadow Realm deletes the snapshot target | Only controls remain | Only controls remain |

Shadow Realm uses both remount transitions for the snapshot deletion. The
two control files must remain hash-unchanged. GLOBALROOT enumeration uses
.NET rather than PowerShell's provider.

The script checks read-only driver status, snapshot identity, and lab NTFS
event 140 during a 15-second interval. This bounded, message-filtered event
check is not an exhaustive filesystem-health assessment. It stops on errors
without forced dismount or attribute-only fallback.

## Artifacts and recovery

The disk, snapshot, logs, hashes, and view remain for inspection. The run
directory contains the disk image, transcript, identity manifest, baseline
hashes, stage listings, and command output. `result.json` is written only
after the script's checks pass; failures inside the lab phase are recorded
in `failure.json`.

These local artifacts can contain personal paths and device identities;
**do not publish them or commit them to this directory**. The `demo/` folder
contains only the reusable script and documentation, not live lab output.
Existing runs under the former `VssLab` directory are not migrated.

**Do not recursively delete the run directory:** it contains a link into the
snapshot. Revert the disposable VM checkpoint to discard the experiment.
Read-only flags are not proof of clean filesystem recovery; the script does
not automatically roll back changes. If restoration fails, preserve the
errors and follow the utility's [safety guidance](../README.md).
Do not force dismount, use an old attribute-only binary, or blindly retry a
failed deletion.

A passing demo does not demonstrate secure erasure, general snapshot
compatibility, or persistence across reboot/detach.
