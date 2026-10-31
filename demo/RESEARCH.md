# Shadow Realm research notes

## Bounded evidence, not guarantees

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

Automated tests use mock volume controls and owned temporary files/junctions.
They never open live shadow devices. They do not establish filesystem safety.
Independently compare source and other-snapshot controls when experimenting.

## Why remount transitions are mandatory

Attribute-only restoration was followed by NTFS transaction-log flush errors
in the AIM-backed experiment. Shadow Realm therefore automatically remounts
for every mutating command, including debug read-only recovery.
`--remount-snapshot` is now a redundant debug-only compatibility flag.
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
