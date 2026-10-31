# Unreleased

- Added `create`, `read`, and `update` for individual snapshot-relative regular
  files. Create/update take binary-safe `--source` input; read emits raw stdout
  or creates a new `--output` file without remounting.
- Mutating file commands automatically remount writable and restore read-only.
  Create never overwrites; update replaces contents and refuses multiple hard
  links. Both support `--dry-run` and retain path/identity protections.
- Moved `inspect`, `diagnose-lock`, `probe`, and `restore-readonly` under the
  `debug` subcommand; use `debug --help` for maintenance usage. CRUD stays
  top-level; `--debug` is not accepted.
- `--remount-snapshot` is now redundant and accepted only by `debug probe`
  and `debug restore-readonly`.
  Remove it from normal deletion invocations. All mutations still use both
  remount transitions; no attribute-only fallback was reintroduced.
- Updated the demo to opt into debug inspection and use automatic remounting.
  New CRUD behavior has not been certified on live VSS snapshots.

# Shadow Realm 0.4.0-experimental.1

**Experimental prerelease; disposable Windows x64 VMs only.**
Not a supported VSS management tool, production recovery method, or
secure-erasure utility. The executable is unsigned.

## Breaking safety changes

- Removed `--restore-attributes-only`. Legacy invocations fail with an
  explanatory argument error before any device is opened.
- Required explicit `--remount-snapshot` on `probe`, confirmed `delete`, and
  `restore-readonly`. The previous non-remount default had the same
  attribute-only cleanup risk and is rejected.
- Preserved the lock-before-dismount sequence on both transitions. Failed
  locks never trigger force-dismount or an attribute-only fallback.
- Clarified success and recovery messages: driver flags do not establish
  filesystem integrity; errors, forced termination, and driver failures can
  leave uncertain state.

## Naming and layout

- Renamed the utility and executable from `vss-write-demo` to `shadow-realm`.
  Update existing commands; no legacy executable alias is supplied.
- Moved the demonstration to `demo\Invoke-ShadowRealmDemo.ps1`, with its
  walkthrough and research evidence under `demo/`. The utility remains at
  the repository and release archive root.
- The demo defaults to `shadow-realm.exe` in its parent directory. Source
  builds still require an explicit `-ToolPath` pointing at the release binary.
- New demo runs use `$env:USERPROFILE\ShadowRealmLab` and the volume label
  `SHADOW-REALM`. Existing lab artifacts are not moved or modified.

## Distribution

Initial private repository import, Windows test/package scripts, pinned CI,
allowlisted source/executable/demo ZIP, SHA-256 checksums, and source/compiler
provenance. No third-party binaries, private lab logs, VM disks, or keys are
distributed. No open-source license has been selected.

## Evidence limits

Previous Windows 11/Volsnap experiments rejected an ordinary OS shadow's
writable transition; an AIM-backed shadow exposed NTFS transaction-log flush
errors after attribute-only restoration. A fresh Microsoft VHDX passed one
existing-file deletion with both remounts and bounded control/hash checks.
See [demo/RESEARCH.md](demo/RESEARCH.md) for the versions and limitations.

This release's automated checks use mock controls and owned temporary files;
the live disk-creating demo is not part of CI or host validation. Prior VM
results are not integration certification of this build. Reboot/detach
persistence and general snapshot compatibility remain unestablished.
