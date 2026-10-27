# Shadow Realm

An experimental Windows utility for inspecting VSS shadow copies and deleting
one selected file name from a snapshot.

**Disposable Windows x64 VMs only.** Use an elevated terminal, a VM checkpoint,
and sacrificial data. Never use production disks or snapshots. Shadow Realm is
not a supported VSS management tool, a recovery tool, or a secure-erasure tool.

## What

Windows Volume Shadow Copy Service (VSS) captures point-in-time copies of a
volume. Shadow Realm works with an existing shadow copy to:

- Inspect its attributes and driver read-only status.
- Preview a file without changing it.
- Diagnose volume locking or probe whether writes are possible.
- Attempt to delete exactly one selected file name, then restore the original
  read-only attributes.

The utility is written in Rust with no external crate dependencies. It does
not create disks or snapshots; the optional [demo](demo/README.md) does that.

## Why

Deleting a file from a live volume can leave its earlier copy in a VSS
snapshot. Shadow Realm explores whether that snapshot copy can be removed
independently, without deleting the entire snapshot.

This is a research utility, not a promise that arbitrary snapshots can be
changed safely. Some configurations reject writes; earlier experiments also
exposed filesystem errors. See the [research notes](demo/RESEARCH.md) for the
observations and their limits. Deleting a name does not securely erase its data.

## How

### Build

Install Rust 1.85+ and the Visual Studio Windows x64 C++ build tools, then run
from the repository root:

```powershell
cargo build --release --locked --offline --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\shadow-realm.exe --help
```

Alternatively, use `shadow-realm.exe` from an extracted release. Release
binaries are unsigned; compare the ZIP's SHA-256 hash with its accompanying
`SHA256SUMS.txt`. No open-source license has been granted.

### Select and preview

In the disposable VM, list snapshots and identify the intended lab copy:

```powershell
Get-CimInstance Win32_ShadowCopy | Select-Object ID, DeviceObject, VolumeName
```

`--snapshot` takes the number at the end of `HarddiskVolumeShadowCopy42`,
not a VSS GUID or drive letter. Verify the source volume and device each time;
numbers can change after a reboot or reconnect.

From the directory containing `shadow-realm.exe`, replace `42` and the file
path with your lab target:

```powershell
.\shadow-realm.exe inspect --snapshot 42
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\delete-me.txt' --dry-run
```

### Delete in the lab

Close tools accessing the snapshot and keep the executable and terminal
outside it. Only after checking the preview:

```powershell
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\delete-me.txt' --confirm-lab --confirm-delete --remount-snapshot
```

Shadow Realm locks and dismounts the selected shadow volume, requests writable
attributes, reopens it, and deletes the file by handle. It then repeats the
remount sequence to restore read-only attributes. It rejects unsafe paths,
reparse points, changed file identities, and busy files; it does not bypass
permissions or force dismounts.

Run `--help` for `probe`, `diagnose-lock`, and `restore-readonly`. Lock diagnosis
is not passive: it can flush caches and briefly exclude other access.
All mutations require explicit lab consent and remount transitions.

For a complete source-versus-snapshot walkthrough with an owned virtual disk,
use the **[demo guide](demo/README.md)**.

## Failure and recovery limits

**Restoring read-only flags is not rollback or proof of filesystem integrity.**
A failed command may already have deleted the file or left uncertain state.
Do not interrupt a mutation, blindly retry, or fall back to attribute-only
restoration. Preserve errors and revert the disposable VM checkpoint if
restoration fails. `restore-readonly` is best-effort recovery, not undo.

Exit codes are `0` for success, `1` for operational failure, and `2` for invalid
arguments. Reboot/detach persistence and general snapshot compatibility remain
unestablished.

Development: `scripts\Test.ps1` runs offline verification without live VSS
operations. `scripts\Package.ps1` creates the release ZIP from a clean committed
checkout using Rust 1.99.1. See [release notes](RELEASE-NOTES.md) for compatibility
changes.
