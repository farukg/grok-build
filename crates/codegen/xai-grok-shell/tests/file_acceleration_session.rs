//! One process: the session gate, a missing factory, then the preserving factory.

use xai_grok_shell::session::{
    client_owned_files_skip_acceleration, decide_gate, install_preserving_file_accelerator,
    preserving_factory_start_notice, read_local_acceleration, record_off_sandbox_profile,
    GateDecision, GateInputs,
};

#[tokio::test]
async fn local_session_acceleration_follows_the_gate_and_the_factory() {
    record_off_sandbox_profile();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("note.txt");
    let disk = b"same bytes";
    std::fs::write(&path, disk).expect("write");

    let (wrapped_before_install, bytes_before) = read_local_acceleration(true, "before", &path)
        .await
        .expect("read before install");
    assert!(!wrapped_before_install);
    assert_eq!(disk.as_slice(), bytes_before);

    let (feature_off, _) = read_local_acceleration(false, "off", &path)
        .await
        .expect("feature off");
    assert!(!feature_off);

    assert_eq!(
        GateDecision::SandboxRequested,
        decide_gate(GateInputs {
            is_feature_enabled: true,
            configured_profile: Some("workspace"),
            requested_confinement: Some("workspace"),
            is_inside_bwrap: false,
        })
    );
    assert_eq!(
        GateDecision::InsideBwrap,
        decide_gate(GateInputs {
            is_feature_enabled: true,
            configured_profile: Some("off"),
            requested_confinement: None,
            is_inside_bwrap: true,
        })
    );
    assert!(client_owned_files_skip_acceleration());

    let noticed = preserving_factory_start_notice(&path)
        .await
        .expect("fresh slot");
    assert_eq!(disk.as_slice(), noticed);

    install_preserving_file_accelerator();
    let (wrapped, bytes) = read_local_acceleration(true, "after", &path)
        .await
        .expect("read after install");
    assert!(wrapped);
    assert_eq!(disk.as_slice(), bytes);
    assert_eq!(std::fs::read(&path).expect("disk"), bytes);
}
