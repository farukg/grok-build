use super::*;

#[test]
fn kernel_time_namespace_offsets_are_parsed_as_signed_timespecs() {
    assert_eq!(
        monotonic_offset_ns("monotonic           0         0\nboottime             0         0\n"),
        Ok(0)
    );
    assert_eq!(
        monotonic_offset_ns("monotonic -1 500000000\nboottime 300 0\n"),
        Ok(-500_000_000)
    );
    assert_eq!(
        monotonic_offset_ns("monotonic 1200 1\nboottime 0 0\n"),
        Ok(1_200_000_000_001)
    );
}

#[test]
fn ambiguous_or_invalid_native_domain_evidence_is_not_a_clock_identity() {
    for contents in [
        "",
        "boottime 0 0",
        "monotonic 0",
        "monotonic 0 0 extra",
        "monotonic 0 -1",
        "monotonic 0 1000000000",
        "monotonic 0 0\nmonotonic 0 0",
        "monotonic text 0",
    ] {
        assert_eq!(
            monotonic_offset_ns(contents),
            Err(ClockUnavailable::DomainUnavailable),
            "{contents}"
        );
    }
    assert_eq!(
        monotonic_offset_ns("monotonic 9223372036854775807 0"),
        Err(ClockUnavailable::ClockRange)
    );
}

fn namespace_observation(device: u64, inode: u64) -> NamespaceObservation {
    let identity = NamespaceIdentity { device, inode };
    NamespaceObservation {
        active_thread: identity,
        thread_children: identity,
        offset_owner: identity,
    }
}

#[test]
fn matching_process_offset_owner_and_active_thread_prove_one_namespace() {
    let observed = namespace_observation(7, 101);
    assert!(
        observed.namespace()
            == Ok(NamespaceIdentity {
                device: 7,
                inode: 101,
            })
    );
    assert_eq!(observed.validate_against(observed), Ok(()));
}

#[test]
fn contradictory_process_or_thread_namespace_evidence_is_unavailable() {
    let valid = namespace_observation(7, 101);
    let other = NamespaceIdentity {
        device: 7,
        inode: 102,
    };
    let other_device = NamespaceIdentity {
        device: 8,
        inode: 101,
    };
    for contradictory in [
        NamespaceObservation {
            active_thread: other,
            ..valid
        },
        NamespaceObservation {
            thread_children: other,
            ..valid
        },
        NamespaceObservation {
            offset_owner: other,
            ..valid
        },
        NamespaceObservation {
            offset_owner: other_device,
            ..valid
        },
    ] {
        assert!(contradictory.namespace() == Err(ClockUnavailable::DomainUnavailable));
        assert_eq!(
            valid.validate_against(contradictory),
            Err(ClockUnavailable::DomainUnavailable)
        );
        assert_eq!(
            contradictory.validate_against(valid),
            Err(ClockUnavailable::DomainUnavailable)
        );
    }
}

#[test]
fn coherent_namespace_switches_during_sampling_are_unavailable() {
    let before = namespace_observation(7, 101);
    for after in [namespace_observation(7, 102), namespace_observation(8, 101)] {
        assert!(after.namespace().is_ok());
        assert_eq!(
            before.validate_against(after),
            Err(ClockUnavailable::DomainUnavailable)
        );
    }
}

#[test]
fn actual_linux_worker_thread_uses_the_process_offset_owner() {
    let process = ClockSample::read().expect("process native clock evidence");
    let worker = std::thread::spawn(ClockSample::read)
        .join()
        .expect("worker completed")
        .expect("worker native clock evidence");
    assert!(process.domain == worker.domain);
    assert!(process.after_ns <= worker.before_ns);
}

#[test]
fn actual_linux_samples_have_one_stable_kernel_domain_and_ordered_bounds() {
    let first = ClockSample::read().expect("Linux boot/time namespace evidence and native clock");
    let second = ClockSample::read().expect("Linux boot/time namespace evidence and native clock");
    assert_eq!(first.domain, second.domain);
    assert!(first.before_ns <= first.after_ns);
    assert!(first.after_ns <= second.before_ns);
    assert!(first.local <= second.local);
}
