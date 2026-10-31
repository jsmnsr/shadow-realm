use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn lock_diagnosis_only_locks_unlocks_and_previews() {
    let mut volume = MockVolume::default();
    let mut previews = 0;
    diagnose_lock(&mut volume, || {
        previews += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(previews, 1);
    assert_eq!(volume.events, ["lock", "unlock", "lock", "unlock"]);
    assert_eq!(volume.sets, Vec::<u64>::new());
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert!(!volume.locked);
}

#[test]
fn lock_diagnosis_reports_each_failed_stage_without_force_or_retry() {
    for failures in [
        vec![("lock", 1)],
        vec![("lock", 2)],
        vec![("lock", 1), ("lock", 2)],
    ] {
        let mut volume = MockVolume {
            failures: failures.clone(),
            ..Default::default()
        };
        let mut previews = 0;
        let error = diagnose_lock(&mut volume, || {
            previews += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(previews, 1);
        assert_eq!(
            format!("{error:#}").contains("BEFORE PREVIEW: LOCK FAILED"),
            failures.contains(&("lock", 1))
        );
        assert_eq!(
            format!("{error:#}").contains("AFTER PREVIEW: LOCK FAILED"),
            failures.contains(&("lock", 2))
        );
        assert_eq!(
            volume
                .events
                .iter()
                .filter(|&&event| event == "lock")
                .count(),
            2
        );
        assert_eq!(
            volume
                .events
                .iter()
                .filter(|&&event| event == "unlock")
                .count(),
            2 - failures.len()
        );
        assert!(
            volume
                .events
                .iter()
                .all(|event| matches!(*event, "lock" | "unlock"))
        );
        assert_eq!(volume.sets, Vec::<u64>::new());
        assert!(!volume.locked);
    }
}

#[test]
fn lock_diagnosis_stops_immediately_on_unlock_failure() {
    for failed_unlock in [1, 2] {
        let mut volume = MockVolume {
            failures: vec![("unlock", failed_unlock)],
            ..Default::default()
        };
        let mut previews = 0;
        let error = diagnose_lock(&mut volume, || {
            previews += 1;
            Ok(())
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("UNLOCK FAILED"));
        assert_eq!(previews, failed_unlock - 1);
        assert_eq!(volume.events.len(), failed_unlock * 2);
        assert_eq!(volume.sets, Vec::<u64>::new());
        // The real SnapshotVolume owns its handle, which closes on run's exit.
        assert!(volume.locked);
    }
}

#[test]
fn lock_diagnosis_preserves_preview_and_prior_lock_errors() {
    for first_lock_fails in [false, true] {
        let mut volume = MockVolume::default();
        if first_lock_fails {
            volume.failures.push(("lock", 1));
        }
        let error =
            diagnose_lock(&mut volume, || Err(anyhow::anyhow!("missing target"))).unwrap_err();
        assert!(format!("{error:#}").contains("PREVIEW FAILED: missing target"));
        assert_eq!(
            format!("{error:#}").contains("BEFORE PREVIEW: LOCK FAILED"),
            first_lock_fails
        );
        assert!(!format!("{error:#}").contains("AFTER PREVIEW"));
        assert_eq!(volume.sets, Vec::<u64>::new());
        assert!(!volume.locked);
    }
}

#[test]
fn lock_diagnosis_refuses_unexpected_or_writable_snapshots() {
    for (attributes, writable_override) in [
        (KNOWN_ATTRIBUTES & !READ_ONLY, None),
        (KNOWN_ATTRIBUTES, Some(true)),
        (KNOWN_ATTRIBUTES & !SHADOW_COPY, None),
    ] {
        let mut volume = MockVolume {
            attributes,
            writable_override,
            ..Default::default()
        };
        assert!(diagnose_lock(&mut volume, || panic!("preview must not run")).is_err());
        assert_eq!(volume.events, Vec::<&str>::new());
        assert_eq!(volume.sets, Vec::<u64>::new());
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent fault-injection switches model failures at different driver stages"
)]
struct MockVolume {
    attributes: u64,
    sets: Vec<u64>,
    fail_enable: bool,
    fail_restore: bool,
    ignore_enable: bool,
    deny_enabled_write: bool,
    writable_override: Option<bool>,
    events: Vec<&'static str>,
    failures: Vec<(&'static str, usize)>,
    panic_on: Option<&'static str>,
    locked: bool,
}

impl Default for MockVolume {
    fn default() -> Self {
        Self {
            attributes: KNOWN_ATTRIBUTES,
            sets: Vec::new(),
            fail_enable: false,
            fail_restore: false,
            ignore_enable: false,
            deny_enabled_write: false,
            writable_override: None,
            events: Vec::new(),
            failures: Vec::new(),
            panic_on: None,
            locked: false,
        }
    }
}

impl MockVolume {
    fn step(&mut self, name: &'static str) -> Result<()> {
        self.events.push(name);
        if self.panic_on == Some(name) {
            self.panic_on = None;
            panic!("simulated {name} panic");
        }
        let count = self.events.iter().filter(|event| **event == name).count();
        if self.failures.contains(&(name, count)) {
            Err(anyhow::anyhow!("simulated {name} failure"))
        } else {
            Ok(())
        }
    }
}

impl Volume for MockVolume {
    fn attributes(&mut self) -> Result<u64> {
        Ok(self.attributes)
    }

    fn set_attributes(&mut self, attributes: u64) -> Result<()> {
        self.step("set")?;
        self.sets.push(attributes);
        let enabling = attributes & READ_ONLY == 0;
        if !(enabling && self.ignore_enable) {
            self.attributes = attributes;
        }
        if enabling && self.fail_enable {
            Err(anyhow::anyhow!("simulated partial enable failure"))
        } else if !enabling && self.fail_restore {
            Err(anyhow::anyhow!("simulated restoration failure"))
        } else {
            Ok(())
        }
    }

    fn is_writable(&mut self) -> Result<bool> {
        if self.deny_enabled_write {
            return Ok(false);
        }
        Ok(self
            .writable_override
            .unwrap_or(self.attributes & READ_ONLY == 0))
    }

    fn lock(&mut self) -> Result<()> {
        self.step("lock")?;
        assert!(!self.locked);
        self.locked = true;
        Ok(())
    }

    fn dismount(&mut self) -> Result<()> {
        assert!(self.locked, "must lock successfully before dismounting");
        self.step("dismount")
    }

    fn unlock(&mut self) -> Result<()> {
        self.step("unlock")?;
        self.locked = false;
        Ok(())
    }

    fn reopen(&mut self) -> Result<()> {
        // Closing the old handle releases its lock even if explicit unlock failed.
        self.locked = false;
        self.step("reopen")
    }
}

#[test]
fn successful_operation_restores_exact_original_attributes() {
    for original in [KNOWN_ATTRIBUTES, KNOWN_ATTRIBUTES & !HIDDEN] {
        let mut volume = MockVolume {
            attributes: original,
            ..Default::default()
        };
        let mut ran = false;
        with_writable_snapshot(&mut volume, || {
            ran = true;
            Ok(())
        })
        .unwrap();
        assert!(ran);
        assert_eq!(volume.sets, [original & !READ_ONLY, original]);
        assert_eq!(volume.attributes, original);
    }
}

#[test]
fn failed_file_operation_still_restores() {
    let mut volume = MockVolume::default();
    assert!(
        with_writable_snapshot(&mut volume, || Err(anyhow::anyhow!("file creation denied")))
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("file creation denied"))
    );
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(volume.sets.len(), 2);
}

#[test]
fn failed_enable_attempt_still_restores_without_running_action() {
    let mut volume = MockVolume {
        fail_enable: true,
        ..Default::default()
    };
    let error = with_writable_snapshot(&mut volume, || panic!("must not run")).unwrap_err();
    assert!(format!("{error:#}").contains("partial enable failure"));
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(volume.sets.len(), 2);
}

#[test]
fn preserves_both_operation_and_restoration_errors() {
    let mut volume = MockVolume {
        fail_restore: true,
        ..Default::default()
    };
    let error =
        with_writable_snapshot(&mut volume, || Err(anyhow::anyhow!("file failure"))).unwrap_err();
    assert!(format!("{error:#}").contains("file failure"));
    assert!(format!("{error:#}").contains("RESTORATION FAILED"));
}

#[test]
fn restoration_failure_prevents_success() {
    let mut volume = MockVolume {
        fail_restore: true,
        ..Default::default()
    };
    assert!(
        with_writable_snapshot(&mut volume, || Ok(()))
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("RESTORATION FAILED"))
    );
}

#[test]
fn rejects_unobserved_attribute_transition() {
    let mut volume = MockVolume {
        ignore_enable: true,
        ..Default::default()
    };
    assert!(
        with_writable_snapshot(&mut volume, || panic!("must not run"))
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("transition not observed"))
    );
    assert_eq!(volume.sets.len(), 2);
}

#[test]
fn refuses_write_probe_when_driver_still_reports_write_protection() {
    let mut volume = MockVolume {
        deny_enabled_write: true,
        ..Default::default()
    };
    assert!(
        with_writable_snapshot(&mut volume, || panic!("must not run"))
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("still reports write protection"))
    );
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(volume.sets.len(), 2);
}

#[test]
fn refuses_already_writable_or_unexpected_attributes() {
    for attributes in [
        KNOWN_ATTRIBUTES & !READ_ONLY,
        READ_ONLY,
        KNOWN_ATTRIBUTES | 1,
    ] {
        let mut volume = MockVolume {
            attributes,
            ..Default::default()
        };
        assert!(with_writable_snapshot(&mut volume, || panic!("must not run")).is_err());
        assert_eq!(volume.sets, Vec::<u64>::new());
    }
}

#[test]
fn refuses_inconsistent_initial_writable_status() {
    let mut volume = MockVolume {
        writable_override: Some(true),
        ..Default::default()
    };
    assert!(with_writable_snapshot(&mut volume, || panic!("must not run")).is_err());
    assert_eq!(volume.sets, Vec::<u64>::new());
}

#[test]
fn unwinding_restores_attributes() {
    let mut volume = MockVolume::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = with_writable_snapshot(&mut volume, || panic!("simulated panic"));
    }));
    assert!(result.is_err());
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(volume.sets.len(), 2);
}

#[test]
fn remount_probe_runs_both_transitions_in_order() {
    let mut volume = MockVolume::default();
    with_writable_snapshot(&mut volume, || Ok(())).unwrap();
    assert_eq!(
        volume.events,
        [
            "lock", "dismount", "set", "unlock", "reopen", "lock", "dismount", "set", "unlock",
            "reopen",
        ]
    );
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert!(!volume.locked);
}

#[test]
fn lock_failure_never_forces_a_dismount_or_attribute_change() {
    let mut volume = MockVolume {
        failures: vec![("lock", 1)],
        ..Default::default()
    };
    assert!(change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY).is_err());
    assert_eq!(volume.events, ["lock"]);
    assert_eq!(volume.sets, Vec::<u64>::new());
    assert!(!volume.locked);
}

#[test]
fn dismount_failure_unlocks_and_reopens_without_changing_attributes() {
    let mut volume = MockVolume {
        failures: vec![("dismount", 1)],
        ..Default::default()
    };
    assert!(change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY).is_err());
    assert_eq!(volume.events, ["lock", "dismount", "unlock", "reopen"]);
    assert_eq!(volume.sets, Vec::<u64>::new());
    assert!(!volume.locked);
}

#[test]
fn each_failed_enable_stage_prevents_file_action_and_attempts_restoration() {
    for stage in ["lock", "dismount", "set", "unlock", "reopen"] {
        let mut volume = MockVolume {
            failures: vec![(stage, 1)],
            ..Default::default()
        };
        let error = with_writable_snapshot(&mut volume, || panic!("must not run")).unwrap_err();
        assert!(format!("{error:#}").contains(stage), "{error:#}");
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(!volume.locked);
    }
}

#[test]
fn partial_attribute_change_still_unlocks_reopens_and_restores() {
    let mut volume = MockVolume {
        fail_enable: true,
        ..Default::default()
    };
    assert!(with_writable_snapshot(&mut volume, || panic!("must not run")).is_err());
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(
        volume
            .events
            .iter()
            .filter(|name| **name == "reopen")
            .count(),
        2
    );
    assert!(!volume.locked);
}

#[test]
fn restoration_remount_failure_is_not_success() {
    for stage in ["lock", "dismount", "set", "unlock", "reopen"] {
        let mut volume = MockVolume {
            failures: vec![(stage, 2)],
            ..Default::default()
        };
        let error = with_writable_snapshot(&mut volume, || Ok(())).unwrap_err();
        assert!(
            format!("{error:#}").contains("RESTORATION FAILED"),
            "{stage}: {error:#}"
        );
        assert!(!volume.locked);
        if stage == "lock" {
            assert_eq!(volume.sets, [KNOWN_ATTRIBUTES & !READ_ONLY]);
            assert_eq!(
                volume.events,
                ["lock", "dismount", "set", "unlock", "reopen", "lock"]
            );
            assert_eq!(volume.attributes, KNOWN_ATTRIBUTES & !READ_ONLY);
        }
    }
}

#[test]
fn remount_reports_primary_unlock_and_reopen_errors_together() {
    let mut volume = MockVolume {
        failures: vec![("dismount", 1), ("unlock", 1), ("reopen", 1)],
        ..Default::default()
    };
    let error = change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY).unwrap_err();
    for expected in ["dismount", "unlock", "REOPEN FAILED"] {
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
    assert!(!volume.locked);
}

#[test]
fn remount_restoration_also_runs_on_file_failure_and_unwinding() {
    let mut volume = MockVolume::default();
    assert!(
        with_writable_snapshot(&mut volume, || Err(anyhow::anyhow!(
            "file is still read-only"
        )))
        .is_err()
    );
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert_eq!(
        volume
            .events
            .iter()
            .filter(|name| **name == "reopen")
            .count(),
        2
    );
    let mut volume = MockVolume::default();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _ = with_writable_snapshot(&mut volume, || panic!("simulated file failure"));
        }))
        .is_err()
    );
    assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    assert!(!volume.locked);
}

#[test]
fn remount_recovery_preserves_other_attributes() {
    for original in [
        KNOWN_ATTRIBUTES & !READ_ONLY,
        KNOWN_ATTRIBUTES & !READ_ONLY & !HIDDEN,
    ] {
        let mut volume = MockVolume {
            attributes: original,
            ..Default::default()
        };
        restore_readonly(&mut volume).unwrap();
        assert_eq!(
            volume.events,
            ["lock", "dismount", "set", "unlock", "reopen"]
        );
        assert_eq!(volume.attributes, original | READ_ONLY);
    }
}

#[test]
fn unwinding_while_locked_releases_lock_before_restoring_attributes() {
    for stage in ["dismount", "set"] {
        let mut volume = MockVolume {
            panic_on: Some(stage),
            ..Default::default()
        };
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _ = with_writable_snapshot(&mut volume, || panic!("must not run"));
            }))
            .is_err()
        );
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(!volume.locked);
        assert_eq!(
            volume
                .events
                .iter()
                .filter(|name| **name == "reopen")
                .count(),
            2
        );
    }
}

#[test]
fn attribute_changes_always_lock_dismount_and_reopen() {
    for attributes in [KNOWN_ATTRIBUTES, KNOWN_ATTRIBUTES & !READ_ONLY] {
        let mut volume = MockVolume::default();
        change_attributes(&mut volume, attributes).unwrap();
        assert_eq!(
            volume.events,
            ["lock", "dismount", "set", "unlock", "reopen"]
        );
        assert_eq!(volume.attributes, attributes);
        assert_eq!(volume.sets, [attributes]);
        assert!(!volume.locked);
    }
}
