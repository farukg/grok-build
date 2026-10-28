use crate::{AcpAgentGatewaySender, AcpClientMessage, ReceiverSettlement};
use agent_client_protocol as acp;

#[test]
fn receiver_settlement_cannot_be_selected_after_sharing_using_or_exporting_the_sender() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let sender = AcpAgentGatewaySender::new(tx);
    let shared = sender.clone();
    assert!(sender.with_local_receiver_settlement().is_err());
    assert_eq!(
        shared.receiver_settlement(),
        ReceiverSettlement::TransportOnly
    );

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sender = AcpAgentGatewaySender::new(tx);
    assert!(
        sender.forward_fire_and_forget(acp::SessionNotification::new(
            acp::SessionId::new("already-published"),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::from(
                "original record"
            ))),
        ))
    );
    assert!(matches!(
        rx.try_recv().unwrap(),
        AcpClientMessage::SessionNotification(_)
    ));
    assert!(
        sender.with_local_receiver_settlement().is_err(),
        "unobserved earlier completions cannot be recovered by upgrading a flag"
    );

    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let sender = AcpAgentGatewaySender::new(tx);
    let _raw_channel = sender.tx();
    assert!(sender.with_local_receiver_settlement().is_err());
}
