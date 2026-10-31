# Shadow Realm

An experimental Windows utility for creating, reading, updating, and deleting
individual files in a selected VSS shadow copy.

**Disposable Windows x64 VMs only.** Use an elevated terminal, a VM checkpoint,
and sacrificial data. Never use production disks or snapshots. Shadow Realm is
not a supported VSS management tool, a recovery tool, or a secure-erasure tool.

## What

Windows Volume Shadow Copy Service (VSS) captures point-in-time copies of a
volume. Shadow Realm works with an existing shadow copy to:

- Read arbitrary regular-file contents without remounting.
- Create a new file or replace an existing file's contents from a local source.
- Delete exactly one selected file name.
- Automatically remount writable for mutations, then restore the original
  read-only attributes, including after an operation fails.
- Preview mutations with `--dry-run`; access low-level maintenance under `debug`.

The utility is written in Rust and uses Microsoft's `windows-sys` bindings for
Windows APIs. It does not create disks or snapshots; the optional
[demo](demo/README.md) does that.

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
cargo build --release --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\shadow-realm.exe --help
```

Alternatively, use `shadow-realm.exe` from an extracted release. Release
binaries are unsigned; compare the ZIP's SHA-256 hash with its accompanying
`SHA256SUMS.txt`.

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
.\shadow-realm.exe read --snapshot 42 --file 'Demo\delete-me.txt'
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\delete-me.txt' --dry-run
```

### File operations in the lab

Close tools accessing the snapshot and keep the executable and terminal
outside it. Use snapshot-relative `--file` paths. Create and update copy exact
bytes from `--source`, supporting text and binary files:

```powershell
.\shadow-realm.exe create --snapshot 42 --file 'Demo\new.bin' --source '.\payload.bin' --dry-run
.\shadow-realm.exe create --snapshot 42 --file 'Demo\new.bin' --source '.\payload.bin' --confirm-lab
.\shadow-realm.exe read --snapshot 42 --file 'Demo\new.bin' --output '.\export.bin'
.\shadow-realm.exe update --snapshot 42 --file 'Demo\new.bin' --source '.\replacement.bin' --confirm-lab
.\shadow-realm.exe delete --snapshot 42 --file 'Demo\new.bin' --confirm-lab --confirm-delete
```

Create fails if the name already exists; parent directories must already exist.
Update requires an existing regular file and replaces its entire contents,
including truncating it when the source is shorter or empty. It rejects files
with multiple hard links rather than changing other names. The source is
buffered in memory and closed before any remount; available memory limits its
size. No source metadata or alternate streams are copied.

Read never changes shadow attributes, locks the volume, or remounts. Without
`--output`, stdout contains only the file's bytes; diagnostics go to stderr.
`--output` creates a new local file and never overwrites an existing one. Use it
for binary exports because older PowerShell redirection can re-encode bytes.
An output failure can leave an incomplete export.

Create, update, and delete accept `--dry-run` instead of confirmations. A dry
run validates the path and, for create/update, reads the source, but does not
test write permissions or mutate the snapshot. Actual mutations require
`--confirm-lab`; delete also retains `--confirm-delete`.

Shadow Realm locks and dismounts the selected shadow volume, requests writable
attributes, reopens it, and performs the file operation. It then repeats the
remount sequence to restore read-only attributes. It rejects unsafe paths,
reparse points, changed file identities, and busy files; it does not bypass
permissions or force dismounts. Create refuses changed parent identities;
update/delete recheck the selected file identity after remounting.

### Debug commands

Normal help exposes CRUD only. Maintenance commands live under the `debug`
subcommand; `debug --help` lists them:

```powershell
.\shadow-realm.exe debug --help
.\shadow-realm.exe debug inspect --snapshot 42
.\shadow-realm.exe debug diagnose-lock --snapshot 42 --file 'Demo\existing.txt' --confirm-lab
.\shadow-realm.exe debug probe --snapshot 42 --confirm-lab
.\shadow-realm.exe debug restore-readonly --snapshot 42 --confirm-lab
```

Clap generates usage, options, and help from the command definitions. Use
`create --help`, `read --help`, `update --help`, `delete --help`, or
`debug <command> --help` for command-specific flags. `help <command>` also
works; `--version` prints the executable version. Missing commands (including
bare `debug`) show help on stderr and return usage-error exit code `2`;
explicit help and version requests return `0`.

Lock diagnosis is not passive: it can flush caches and briefly exclude other
access. Probe and read-only recovery also remount automatically.
The redundant `--remount-snapshot` flag has been removed. Every mutation,
including debug probing and recovery, always uses remount-based transitions;
there is no switch to disable them.
CRUD stays at the top level; `--debug` is not an accepted flag.
Attribute-only transitions remain prohibited.

For a complete source-versus-snapshot walkthrough with an owned virtual disk,
use the **[demo guide](demo/README.md)**.

## Failure and recovery limits

**Restoring read-only flags is not rollback or proof of filesystem integrity.**
A failed command may already have created, partially overwritten, or deleted
the file, or left uncertain state. Updates are not atomic or transactional.
Do not interrupt a mutation, blindly retry, or fall back to attribute-only
restoration. Preserve errors and revert the disposable VM checkpoint if
restoration fails. `debug restore-readonly` is best-effort recovery, not undo.

Exit codes are `0` for success, `1` for operational failure, and `2` for invalid
arguments. Reboot/detach persistence and general snapshot compatibility remain
unestablished.

## Development

We follow the [Microsoft Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/).
Small, crate-private modules separate CLI parsing, path validation, snapshot
policy, and Windows adapters. Unsafe code is limited to documented Win32
boundaries; policy and parsing forbid it.

All Rust tests live under `tests\`: `cli.rs` exercises the executable, while
`unit\` mirrors the source modules. Unit tests are linked as test-only child
modules so they can exercise private internals without widening the public API.

Prefer established, actively maintained crates over reimplementing platform
APIs or general-purpose infrastructure. Evaluate maintenance, licensing, and
compiler compatibility; enable only needed features and commit `Cargo.lock`.
Microsoft's `windows-sys` supplies Win32 bindings; `clap` owns the command tree,
argument constraints, typed validation, and generated help/version output.
`anyhow` preserves runtime error context/backtraces, and `tempfile` manages test fixtures.
Our code retains ownership, validation, snapshot policy, and explicit cleanup
failure reporting.

`scripts\Test.ps1` fetches locked dependencies, then verifies the utility without
live VSS operations. `scripts\Package.ps1` creates the release ZIP from a clean
committed checkout using Rust 1.99.1. Both accept `-Offline` when the dependencies
are already cached. For a manual offline build, first run
`cargo fetch --locked --target x86_64-pc-windows-msvc`, then add `--offline` to
the build command. See [release notes](RELEASE-NOTES.md) for compatibility changes.

## License

[MIT](LICENSE). The license does not change the experimental safety limits above.
Dependency licenses are included in [third-party notices](THIRD-PARTY-NOTICES.md).
