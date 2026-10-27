use crate::{SamplerHandle, commands::SamplerCommand};
use crate::request_preview::NativePreviewFailure;
use xai_grok_sampling_types::ConversationRequest;

#[tokio::test]
async fn preview_mailbox_and_reply_loss_are_distinct_and_do_not_submit() {
    assert!(matches!(SamplerHandle::noop().preview_request(ConversationRequest::default()).await,
        Err(NativePreviewFailure::MailboxClosed)));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sampler = SamplerHandle::new(tx);
    let task = tokio::spawn(async move { sampler.preview_request(ConversationRequest::default()).await });
    let SamplerCommand::Preview { reply, .. } = rx.recv().await.unwrap() else { panic!("preview, not Submit"); };
    drop(reply);
    assert!(matches!(task.await.unwrap(), Err(NativePreviewFailure::ReplyLost)));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn a_model_switch_cannot_relabel_a_stale_candidate() {
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let sampler = crate::SamplerActor::spawn(crate::SamplerConfig {
        model: "new-model".into(), ..Default::default()
    }, crate::RetryPolicy::default(), tx);
    let request = ConversationRequest { model: Some("old-model".into()), ..Default::default() };
    assert!(matches!(sampler.preview_request(request).await, Err(NativePreviewFailure::ModelConfigurationChanged)));
    assert_eq!(sampler.active_count().await, 0);
    assert!(events.try_recv().is_err());
}
