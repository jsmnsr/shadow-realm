# Shadow Realm

**`shadow-realm` is an experimental Windows CLI for inspecting VSS shadow
copies and performing explicitly opted-in file operations inside them.**
Inspect driver state, preview a file, diagnose volume locking, probe write
support, or delete one selected file name from a disposable lab snapshot.

**0.4.0-experimental.1 - disposable Windows x64 VMs only.**
This dependency-free Rust utility is not a supported VSS management tool,
a secure-erasure utility, or a production recovery tool. Use an elevated
terminal in a disposable VM with a checkpoint and a sacrificial virtual disk.
Never attach production disks. Working forensic copies will be intentionally
altered; preserve original images and acquisition hashes elsewhere.

The Windows SDK defines `IOCTL_VOLUME_SET_GPT_ATTRIBUTES`, but changing
arbitrary completed VSS snapshots is **not established as supported**. Being
the newest snapshot is not sufficient. The tool does not bypass driver
authorization, patch the kernel, write raw sectors, or force dismounts.

## Project layout

| Location | Purpose |
| --- | --- |
| `src/`, `Cargo.toml`, `Cargo.lock` | The Shadow Realm CLI; no runtime dependency on the demo |
| `scripts/` | Utility verification and release packaging |
| [demo/](demo/README.md) | Disposable-VM walkthrough and owned-disk demonstration |
| [demo/RESEARCH.md](demo/RESEARCH.md) | Historical lab observations and evidence limits |
| [RELEASE-NOTES.md](RELEASE-NOTES.md) | Compatibility and release changes |

The utility does not create disks or snapshots. The optional [demo](demo/README.md)
does that separately to demonstrate source-versus-snapshot deletion.

## Build and run

Use Rust 1.85+ and the Windows x64 linker for your target. From the repository
root:

```powershell
cargo build --release --locked --offline --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\shadow-realm.exe --help
```

For the GNU linker toolchain, use `x86_64-pc-windows-gnu` instead. Command
examples below assume the executable is in the current directory, as in an
extracted release. Live shadow-device access requires appropriate permissions;
perform mutations only in an elevated, checkpointed disposable VM.

## Experimental release

The Windows x64 ZIP includes the unsigned executable, matching demo script,
Rust source and lockfile, build/verification scripts, this document, release
notes, per-file SHA-256 checksums, and build provenance. No third-party Windows
binaries or VM artifacts are included. No public/open-source license is granted
by this private release. `shadow-realm.exe` is at the archive root; the optional
walkthrough, research notes, and demonstration script are under `demo/`.

Compare the downloaded ZIP with the accompanying `SHA256SUMS.txt`:

```powershell
Get-FileHash .\shadow-realm-0.4.0-experimental.1-windows-x64.zip -Algorithm SHA256
```

Checksums detect corruption, not publisher identity; the executable is not
Authenticode-signed. Extract into an ordinary directory outside all snapshots.

### Compatibility

The utility was formerly named `vss-write-demo`. Update executable paths and
automation to use `shadow-realm`; there is no legacy executable alias. The
demo script has moved to `demo\Invoke-ShadowRealmDemo.ps1`.

`--restore-attributes-only` is removed and explicitly rejected. All mutating
commands now require `--remount-snapshot`, including `restore-readonly`.
The 0.3.1 non-remount default also performed attribute-only restoration and is
no longer allowed. Neither flag restoration nor a successful remount proves
complete filesystem recovery.

## Commands

The numeric `--snapshot` value is the suffix of
`\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy42`, not a VSS GUID or a drive.
Find the intended lab copy with `Get-CimInstance Win32_ShadowCopy` and verify
its `ID`, `DeviceObject`, and `VolumeName`. Device numbers can change after a
reboot or reconnect. No automatic snapshot selection is performed.

```powershell
.\shadow-realm.exe --help
.\shadow-realm.exe inspect --snapshot 42
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\delete-me.txt' --dry-run
.\shadow-realm.exe diagnose-lock --snapshot 42 --file 'Demo\delete-me.txt' --confirm-lab
.\shadow-realm.exe probe --snapshot 42 --confirm-lab --remount-snapshot
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\delete-me.txt' --confirm-lab --confirm-delete --remount-snapshot
.\shadow-realm.exe restore-readonly --snapshot 42 --confirm-lab --remount-snapshot
```

`inspect` queries attributes and driver writable status. `delete --dry-run`
opens the target and its parents for a nonmutating preview showing identity,
size, and hard-link count. Both require access permissions but no lab consent.

`diagnose-lock` compares lock/unlock before and after the same file preview.
It does not change attributes, dismount, or delete. It is **not passive**:
locking can flush caches and briefly exclude other access. A failed lock is
recorded and the comparison continues; unlock or preview failure stops it.
Exit status is nonzero if any stage or final state verification fails.

`probe` creates, flushes, reads back, and removes one uniquely named marker.
`delete` removes exactly one existing regular-file name after previewing it.
Both require an initially read-only shadow with recognized attributes.

Every attribute transition, including restoration, requires this sequence:

1. Lock only the selected shadow device; a failed lock prevents dismount and
   attribute changes in that attempt.
2. Dismount while holding the successful lock, then request the attributes.
3. Attempt unlock, close/reopen the device handle, and access the root to
   request filesystem mounting.
4. Verify the observed attributes and driver writable status.

Unlock/reopen are attempted even if dismount or attribute setting fails.
An armed restoration guard attempts the read-only sequence after errors or
panic unwinding. A failed reopen can cause one explicit open attempt when
the restoration sequence needs a handle. There is no forced dismount,
attribute-only fallback, or automatic rerun of the file operation.

Close Explorer windows and other tools accessing the snapshot before mutation.
Keep the executable and terminal working directory outside the snapshot.
Do not terminate unrelated processes or backup services to obtain a lock.

### Deletion restrictions

- Exactly one relative file path: no recursion, wildcards, rooted paths,
  streams, traversal, reserved device names, or ambiguous dots/spaces.
- Reparse points are rejected, including parent directories and the target.
  Parent handles stay pinned without write/delete sharing during the operation.
- The target is opened exclusively; file identity, size, and timestamps must
  match the pre-transition preview. Deletion is requested against that handle.
- Busy/read-only files and permission failures are not overridden. No ACL,
  ownership, or file permission changes are made.
- The selected name must be confirmed absent. A failed operation may still
  have deleted the file; do not blindly retry.

For link-based viewers such as VSCMount, resolve the link to the actual shadow
device first. The viewer's link name is not part of `--file`. The tool does not
invoke VSCMount. Deleting a name is not secure erasure; other hard links,
unallocated data, snapshots, and backups can retain the content.

## Failure and recovery limits

Exit `0` means the selected command and its required checks succeeded, `1` is
an operational failure, and `2` is invalid arguments. Windows API failures
include their error codes. Driver rejection is a valid experimental result.

**Read-only flags are not proof of clean filesystem recovery.** Termination,
power loss, driver errors, failed lock/remount, or failed restoration can leave
uncertain state. Closing the handle is not automatic rollback. A successful
command does not restore deleted contents, allocations, journals, metadata, or
snapshot bookkeeping.

If restoration fails, preserve the error and inspect the lab state. The
explicit `restore-readonly` command above is a best-effort remount-based
recovery attempt, not an undo command. If it cannot lock or any stage fails,
do not force dismount or return to an old attribute-only binary. Revert the
disposable VM checkpoint. Do not interrupt a running mutation.

## Demo and research

See [demo/README.md](demo/README.md) for the owned-disk walkthrough and
[demo/RESEARCH.md](demo/RESEARCH.md) for historical observations. Earlier
configurations both succeeded and failed; these are bounded experiments,
not a supported VSS contract or compatibility guarantee. Reboot/detach
persistence remains unestablished.

Automated tests use mock volume controls and owned temporary files/junctions.
They never open live shadow devices and do not establish filesystem safety.

## Build, verify, and package

Rust edition 2021; minimum supported compiler 1.85. The release recipe pins
Rust 1.99.1. Install the appropriate Windows x64 linker and rustfmt/clippy
components using your normal trusted toolchain provisioning. No crates are
downloaded. The release target is recorded in provenance (GNU or MSVC).

```powershell
cargo generate-lockfile --offline # when package metadata requires a lockfile update
.\scripts\Test.ps1 -Target x86_64-pc-windows-gnu
.\scripts\Package.ps1 -Target x86_64-pc-windows-gnu
```

For Visual Studio Build Tools, use `x86_64-pc-windows-msvc`. Packaging requires
the pinned compiler and a clean committed Git checkout; it reruns verification,
builds offline with the lockfile, strips symbols, remaps local build paths,
and creates only allowlisted ZIP entries under ignored `dist`. Provenance
records the source commit, compiler, target, and build flags without personal
paths. ZIP timestamps come from the commit. This is a repeatable recipe, not
a claim of bit-for-bit reproducibility across toolchains or Windows linkers.

CI runs Windows tests/Clippy/formatting at the minimum compiler and pinned
release compiler, and packages the latter. Actions are pinned to full commits;
CI has read-only repository permissions and does not publish releases.
Release publication is an explicit maintainer action after inspection.

The lock/dismount ordering follows Microsoft's
[FSCTL_LOCK_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_lock_volume)
and [FSCTL_DISMOUNT_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_dismount_volume)
documentation. Those operations can still be rejected on a shadow device.
