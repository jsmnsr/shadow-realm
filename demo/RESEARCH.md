# Shadow Realm research notes

## Live OS-volume CI result

The [manual GitHub Actions run on 2026-11-01 UTC](https://github.com/jsmnsr/shadow-realm/actions/runs/41306942239)
passed against commit `9f4678133a01378cec3e349b4447063c32735f31`.
This was a real snapshot test, not mocked volume controls.

| Component | Recorded configuration |
| --- | --- |
| OS | Windows Server 2022 Datacenter, build 20348 |
| Volsnap | 10.0.20348.1 |
| VSS provider used by both snapshots | Microsoft Software Shadow Copy provider 1.0, version 1.0.0.7 |
| Source | Existing NTFS OS volume; boot/system disk reported as `Msft Virtual Disk`, SAS |

Read exports, all three mutation dry-runs, writable probing, create, shorter
and empty updates, and deletion passed. Original read-only attributes were
confirmed after mutations and after a 15-second delay. Fixture hashes in the
live source and a separate control snapshot stayed unchanged. No matching
storage/filesystem warning or error events appeared in the collection window.
The run's artifact contains the binary hash, snapshot/provider identities,
command results, fixture hashes, and inspections.

The [automatic push run on 2026-11-01 UTC](https://github.com/jsmnsr/shadow-realm/actions/runs/41307507609)
also passed the live checks at commit `36339693f3ad6b9915adfe24a141f8f1fbaa4394`.
That runner reported an NVMe virtual OS disk. The Rust implementation and
live-test script were unchanged between these two successful runs.

Unlike the earlier demo, this did not create an additional data VHDX. It still
used a virtualized hosted runner: the result does not establish bare-metal,
Windows 11, other-provider, reboot-persistence, or production compatibility.
It is a successful observation for the recorded configuration, not proof of
filesystem integrity.

## Local Windows 11 OS-volume result

A separately authorized physical-host experiment on 2026-11-01 UTC **failed
during read-only restoration**. This was an explicit exception to the normal
disposable-VM guidance, not an expansion of supported use. The repository's
VM-only integration harness and native safeguards were left unchanged.

| Component | Recorded configuration |
| --- | --- |
| OS | Windows 11 Pro, build 26300 |
| Volsnap | 10.0.26100.8521 |
| VSS provider | Microsoft Software Shadow Copy provider 1.0, version 1.0.0.7 |
| Source | Physical NVMe boot/system disk; NTFS C: volume |
| Test snapshots | Two fresh `ClientAccessible` snapshots; persistent, `NoWriters=True` |
| Build | Commit `36339693f3ad6b9915adfe24a141f8f1fbaa4394`, Rust 1.92.0, release x64 MSVC |

The tested executable's SHA-256 was
`76C61CCD0D9A0438A15E7868683A6242AF621601FA354241E3E4FE3BD9FE7009`.
Read export and all three mutation dry-runs passed. The probe successfully
locked and dismounted the target, changed attributes from
`0xf000000000000000` to `0xe000000000000000`, reopened it, and completed its
owned marker-file operation. The subsequent restoration attempt failed at
`FSCTL_LOCK_VOLUME` with Win32 **5 (Access is denied)**, before the read-only
attribute request. A separate inspection confirmed `READ_ONLY: false` and
`Driver writable status: true`.

The harness stopped there; ordinary create, update, and delete were not run.
There was no forced dismount, attribute-only recovery, or mutation retry.
After separate cleanup approval, VSS deleted only the two new test snapshots,
and the four unchanged source fixtures and their owned directory were removed.
Before cleanup, all source, control, and target fixture hashes still matched;
the control remained read-only. All five pre-existing snapshot identities
remained after cleanup, C: reported healthy, and no matching storage/filesystem
warning or error events appeared between test start and the delayed cleanup
check. These bounded checks do not establish complete filesystem integrity.
Original failure logs and separate cleanup-success evidence were retained.

This did **not** reproduce the older write-enable error 87: write enablement
worked here, but the complete write/restore lifecycle failed. The reason for
the restoration lock rejection is unknown; no handle or driver-path trace
identified it. This is a current observed restoration failure on this
configuration, not proof of a universal Windows 11 or provider limitation.

### Instrumented restoration-lock follow-up

A second, separately authorized host experiment on the same day reproduced
the restoration failure on a fresh target/control pair. A temporary diagnostic
executable reused the unchanged production volume adapter, transition policy,
and marker operation. It added timestamped tracing, a pause after restoration
access-denied for a filtered Sysinternals Handle capture, and at most one
same-handle retry followed by one close/reopen retry. This was diagnostic-only;
the production CLI's behavior was not changed during that experiment.

| Time from diagnostic start | Observation |
| --- | --- |
| 0.278 seconds | Initial restoration lock returned Win32 5 again. |
| 1.975 seconds | After handle capture, one lock retry began on the same still-open volume handle. |
| 2.024 seconds | That lock succeeded; the normal locked dismount/read-only transition proceeded. |
| 2.141 seconds | Restoration and its read-only checks completed successfully. |

The retry started approximately 1.70 seconds after the initial lock failure.
No handle reopen, access-mode change, permission change, security-service
change, or forced dismount was needed to make it succeed. The fresh-handle
fallback was not reached. A separate production-CLI inspection confirmed
`0xf000000000000000`, `READ_ONLY: true`, and driver writable status `false`.
NTFS Operational events independently recorded both successful explicit-lock
dismounts and subsequent mounts.

The handle capture found only the diagnostic process's direct volume handle
(`FILE_GENERIC_READ`), not an open marker or directory handle. Source inspection
also confirms that the marker file and root-directory enumeration handles are
closed before restoration. This does not exclude short-lived competing opens
that ended before enumeration, or kernel/filter references not exposed as
process handles. In particular, no evidence identifies Defender, an indexer,
or another specific process as the blocker.

**The reproduced lock failure was transient; the blocking owner or internal
condition remains unknown.** Reopening was not required in this reproduction.
This motivated the bounded restoration-lock backoff described below, rather
than a fixed mandatory delay or a forced dismount. This diagnostic recovery
is not a full CRUD pass or live validation of that subsequent implementation.

The two diagnostic snapshots and four source fixtures were removed afterward.
All five pre-existing snapshot identities remained, source/control/target
fixture hashes matched, C: reported healthy, and the delayed System-log check
found no matching storage warnings. Evidence, diagnostic source, and input
hashes were retained locally.

## Earlier bounded experiments

Earlier disposable-VM experiments used Windows 11 build 22631 and Volsnap
10.0.22621.1:

| Configuration | Observation |
| --- | --- |
| Ordinary OS-volume shadow | Writable transition rejected with Win32 error 87. |
| AIM-backed shadow | Marker writes were possible, but attribute-only restoration was followed by repeated NTFS event 140 transaction-log flush failures (`STATUS_INVALID_DEVICE_REQUEST`). |
| Fresh Microsoft VHDX | Existing-file deletion with both remount transitions succeeded; the snapshot survived, source/sentinel hashes were preserved, read-only status returned, and no lab NTFS warnings appeared during the short follow-up. |

These observations concern earlier builds and specific configurations, not
live integration validation of this release. No reboot/detach persistence was
established. A baseline snapshot was not rediscovered after VHDX reattach;
that does not establish persistence of selective deletion. Static driver
analysis and the SDK control definition do not establish a supported contract.

Local `scripts\Test.ps1` verification uses mock volume controls and owned
temporary files/junctions; it never opens live shadow devices. CI also runs
the OS-volume test above against real VSS snapshots in a separate disposable
VM job on pushes, pull requests, and manual runs. Neither establishes general
filesystem safety. Independently compare source and other-snapshot controls
when experimenting.

### Windows 11 error 87 follow-up

The original 2026-10-21 transcripts were revisited on 2026-11-01. The failing
OS-volume target was a pre-existing snapshot created four days earlier, not
a fresh snapshot created by the current integration harness.

An initial probe omitted remounting, but a later **v0.3.0** probe successfully
locked and dismounted the same snapshot before
`IOCTL_VOLUME_SET_GPT_ATTRIBUTES` rejected the transition from
`0xf000000000000000` to `0xe000000000000000` with Win32 error 87.
The marker operation never ran, and the tool reported the original attributes
restored. Missing remounting therefore does not explain the later failure.
The old attribute-only restoration option was a separate cleanup problem,
not the stage that rejected write enablement.

The failing OS snapshot and the AIM snapshots used the same Microsoft software
provider. The OS snapshot reported `Persistent=True`, `ClientAccessible=True`,
and `NoWriters=False`; the AIM snapshot that accepted marker writes reported
`NoWriters=True`. Current CI instead creates fresh `ClientAccessible` snapshots.
Snapshot context, age, lifecycle, backing storage, and tool version are
confounding differences, not independently established causes.

The retained disassembly of the VM's actual Volsnap **10.0.22621.1** contains
multiple input-validation and internal-state branches in `VspSetGptAttributes`
that return `STATUS_INVALID_PARAMETER`, which maps to Win32 error 87.
The log does not identify which branch or downstream operation rejected the
request. Static analysis alone cannot supply the missing runtime state.

**The exact rejection condition remains unresolved.** Successful writes to
other snapshots on that Windows 11 VM rule out a blanket Windows 11 inability
to perform the transition, but do not establish OS-volume compatibility.
The original VM could not be accessed for a verified replay. The current-code
Server 2022 runs above passed; retain the older failure as a historical result,
not a confirmed current regression or a demonstrated fix.
Identifying the cause requires a controlled Windows 11 reproduction, ideally from a
clone of the original VM/checkpoint, capturing the failing driver path.
Fresh writer-aware versus no-writer OS snapshots are a useful comparison;
changing context is not yet a demonstrated fix. Do not bypass driver checks
or restore the removed attribute-only cleanup path.

## Why remount transitions are mandatory

Attribute-only restoration was followed by NTFS transaction-log flush errors
in the AIM-backed experiment. Shadow Realm therefore automatically remounts
for every mutating command, including debug read-only recovery.
The redundant `--remount-snapshot` flag has been removed.
The old `--restore-attributes-only` option is still rejected.

Each transition locks the selected shadow device before dismounting and
changing attributes, then attempts unlock, close/reopen, root access, and
state verification. A failed lock never permits a forced dismount or an
attribute-only fallback. Restoration guards are best-effort error cleanup,
not transaction rollback.

Read-only transitions now retry only `FSCTL_LOCK_VOLUME` failures rooted in
Win32 `ERROR_ACCESS_DENIED` (5). They use the same volume handle, with delays of
100, 200, 400, 800, 1000, 1000, and 1000 milliseconds: at most eight attempts
and 4.5 seconds of deliberate waiting. Windows control-call duration is
additional, so this is not a hard wall-clock timeout. Each retry is logged.
Success proceeds through the normal locked transition and verification;
exhaustion retains the final error and the existing uncertain-state warning.
Automatic cleanup (including unwinding) and `debug restore-readonly` share
this policy. Write-enable locks, lock-only diagnosis, file operations, and
other transition failures are not retried. No handle-reopen fallback, forced
dismount, or attribute-only fallback is added.

The ordering follows Microsoft's
[FSCTL_LOCK_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_lock_volume)
and [FSCTL_DISMOUNT_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_dismount_volume)
documentation. These controls can still be rejected on a shadow device.
`IOCTL_VOLUME_SET_GPT_ATTRIBUTES` is defined by the Windows SDK, but changing
arbitrary completed VSS snapshots is not established as supported.

## Reproducing the demonstration

Use the [owned-disk walkthrough](README.md) only in a checkpointed disposable
VM. A successful run provides bounded directory, hash, identity, driver-state,
and event-log observations; it is not a complete filesystem-health assessment.
Preserve errors as experimental results rather than forcing unsupported
operations to succeed.

No private lab logs, disk images, original forensic evidence, or third-party
binaries are included here. Keep original acquisitions and hashes elsewhere;
working copies used for mutation are intentionally altered.
