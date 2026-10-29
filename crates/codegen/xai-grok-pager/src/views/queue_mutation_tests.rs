use super::{QueueMutation, ServerRowCapabilities};

#[test]
fn capabilities_truth_table() {
    let cases = [
        (
            "parent_agent_message",
            QueueMutation::PerRowKind,
            ServerRowCapabilities::PROTECTED,
        ),
        (
            "delivery",
            QueueMutation::PerRowKind,
            ServerRowCapabilities::PROTECTED,
        ),
        (
            "prompt",
            QueueMutation::PerRowKind,
            ServerRowCapabilities::EDITABLE,
        ),
        (
            "future",
            QueueMutation::PerRowKind,
            ServerRowCapabilities::EDITABLE,
        ),
        (
            "prompt",
            QueueMutation::ReadOnly,
            ServerRowCapabilities::PROTECTED,
        ),
        (
            "future",
            QueueMutation::ReadOnly,
            ServerRowCapabilities::PROTECTED,
        ),
    ];
    for (kind, mutation, expected) in cases {
        assert_eq!(
            expected,
            ServerRowCapabilities::for_pane(kind, mutation),
            "{kind} under {mutation:?}"
        );
    }
    assert_eq!(
        ServerRowCapabilities::EDITABLE,
        ServerRowCapabilities::for_local(QueueMutation::PerRowKind)
    );
    assert_eq!(
        ServerRowCapabilities::PROTECTED,
        ServerRowCapabilities::for_local(QueueMutation::ReadOnly)
    );
}
