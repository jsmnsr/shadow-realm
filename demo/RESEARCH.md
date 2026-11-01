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

Unlike the earlier demo, this did not create an additional data VHDX. It still
used a virtualized hosted runner: the result does not establish bare-metal,
Windows 11, other-provider, reboot-persistence, or production compatibility.
It is a successful observation for the recorded configuration, not proof of
filesystem integrity.

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
