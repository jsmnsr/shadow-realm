# Shadow Realm

Shadow Realm is an experimental Windows x64 command-line tool for creating,
reading, updating, and deleting individual files in an existing Volume Shadow
Copy Service (VSS) snapshot.

Reads never remount the snapshot. Create, update, and delete temporarily
remount it writable, perform the operation, then attempt to restore its original
read-only attributes, including after a failure. Operations target the shadow
copy, not the original live file. Shadow Realm does not create snapshots.

**Disposable Windows VMs only.** Run in an elevated PowerShell terminal with a
VM checkpoint and sacrificial data, never production snapshots. Restoring
read-only attributes is not rollback: a failed operation can leave partial
changes or uncertain filesystem state. If restoration fails, preserve the errors
and revert the VM checkpoint. Deleting a file is not secure erasure.

See the [demo](demo/README.md) for a disposable lab setup and the
[research notes](demo/RESEARCH.md) for known limitations.
The [live integration test](demo/README.md#live-os-volume-integration-test)
exercises OS-volume snapshots in a disposable VM and in push/pull-request CI.

## Commands

| Command | What it does |
| --- | --- |
| `create` | Creates a new file from a local `--source` file. Never overwrites an existing name. |
| `read` | Writes exact file bytes to stdout, or to a new local `--output` file. Never remounts or overwrites an output file. |
| `update` | Replaces an existing file's entire contents from `--source`, including truncation. Not atomic. |
| `delete` | Deletes one existing file name. |

Every command takes `--snapshot NUMBER`. File commands also take `--file`, a
path relative to the snapshot root, such as `Demo\data.bin`. Only regular files
are supported; parent directories must already exist. Unsafe paths, reparse
points, and busy files are rejected rather than bypassed.

Create and update copy exact bytes, not metadata, and buffer the source in
memory before remounting. Update rejects files with multiple hard links.
Use `--output` for binary reads rather than PowerShell redirection.

Mutations require `--confirm-lab`; delete also requires `--confirm-delete`.
Use `--dry-run` **instead of** confirmation flags to preview a mutation without
changing attributes or contents. A dry run does not prove the write will succeed.

### Examples

Find the intended lab snapshot:

```powershell
Get-CimInstance Win32_ShadowCopy | Select-Object ID, DeviceObject, VolumeName
```

For a device ending in `HarddiskVolumeShadowCopy42`, use `--snapshot 42`, not
its VSS GUID or drive letter. Recheck the device and source volume each time;
numbers can change.

From the folder containing `shadow-realm.exe`, with local `payload.bin` and
`replacement.bin` files and an existing `Demo` directory in the chosen snapshot:

```powershell
# Preview, then create a new file.
.\shadow-realm.exe create --snapshot 42 --file 'Demo\new.bin' --source '.\payload.bin' --dry-run
.\shadow-realm.exe create --snapshot 42 --file 'Demo\new.bin' --source '.\payload.bin' --confirm-lab

# Export, replace, then delete that file.
.\shadow-realm.exe read --snapshot 42 --file 'Demo\new.bin' --output '.\export.bin'
.\shadow-realm.exe update --snapshot 42 --file 'Demo\new.bin' --source '.\replacement.bin' --confirm-lab
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\new.bin' --confirm-lab --confirm-delete
```

Keep the executable and terminal outside the snapshot and close other tools
accessing it. Busy snapshots are never force-dismounted.

### Debug commands

Low-level maintenance lives under `debug`:

| Command | What it does |
| --- | --- |
| `debug inspect` | Reports snapshot attributes and driver writable status. |
| `debug diagnose-lock` | Tests locking before and after previewing `--file`; does not remount. |
| `debug probe` | Remounts writable, creates/verifies/removes an owned test file, then restores read-only. |
| `debug restore-readonly` | Attempts read-only recovery through a remount. Not undo. |

All take `--snapshot`; all except `inspect` also require `--confirm-lab`.
Lock diagnosis can flush caches and briefly exclude other access.
Use `create --help`, `read --help`, or `debug --help` for the full options.

Exit codes are `0` for success, `1` for operational failure, and `2` for invalid
arguments. `--version` prints the executable version.

## Install

### Build from source

For the unreleased commands described above, use the source revision matching
this README. Install [Rust](https://www.rust-lang.org/tools/install) 1.85+ and
the Visual Studio C++ Build Tools with the Windows SDK, then run from the
repository root:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\shadow-realm.exe --help
```

The executable is in `target\x86_64-pc-windows-msvc\release`. Run the examples
from that folder, or copy `shadow-realm.exe` to a folder on your `PATH`.

### Prebuilt releases

Download a Windows x64 ZIP from [Releases](https://github.com/jsmnsr/shadow-realm/releases),
compare its SHA-256 hash with the accompanying `SHA256SUMS.txt`, and extract it.
Run `.\shadow-realm.exe --help` from the extracted folder; no installer is needed.
Binaries are unsigned. Published releases may predate this CLI, so follow their
bundled README and [release notes](RELEASE-NOTES.md).

Licensed under [MIT](LICENSE); see [third-party notices](THIRD-PARTY-NOTICES.md)
for dependency licenses.
