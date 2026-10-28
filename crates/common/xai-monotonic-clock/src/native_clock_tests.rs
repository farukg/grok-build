use super::*;

#[test]
fn kernel_time_namespace_offsets_are_parsed_as_signed_timespecs() {
    assert_eq!(monotonic_offset_ns("monotonic           0         0\nboottime             0         0\n"), Ok(0));
    assert_eq!(monotonic_offset_ns("monotonic -1 500000000\nboottime 300 0\n"), Ok(-500_000_000));
    assert_eq!(monotonic_offset_ns("monotonic 1200 1\nboottime 0 0\n"), Ok(1_200_000_000_001));
}

#[test]
fn ambiguous_or_invalid_native_domain_evidence_is_not_a_clock_identity() {
    for contents in [
        "", "boottime 0 0", "monotonic 0", "monotonic 0 0 extra", "monotonic 0 -1",
        "monotonic 0 1000000000", "monotonic 0 0\nmonotonic 0 0", "monotonic text 0",
    ] {
        assert_eq!(monotonic_offset_ns(contents), Err(ClockUnavailable::DomainUnavailable), "{contents}");
    }
    assert_eq!(monotonic_offset_ns("monotonic 9223372036854775807 0"), Err(ClockUnavailable::ClockRange));
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
