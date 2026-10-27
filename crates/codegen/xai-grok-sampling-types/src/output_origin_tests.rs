use super::*;
use crate::{ConversationItem, ConversationRequest};

#[test]
fn output_references_are_opaque_and_absent_from_canonical_and_provider_bodies() {
    let plain = ConversationItem::assistant("answer");
    let mut annotated = plain.clone();
    let ConversationItem::Assistant(item) = &mut annotated else { panic!("assistant fixture"); };
    item.output_origin = OutputOrigin::Aggregate { references: vec![
        ProviderOutputRef::Item("private-output-one".into()),
        ProviderOutputRef::Item("private-output-two".into()),
        ProviderOutputRef::ToolCall("private-call".into()),
    ] };
    assert!(!format!("{annotated:?}").contains("private-output"));
    assert!(!format!("{annotated:?}").contains("private-call"));
    assert_eq!(serde_json::to_vec(&plain).unwrap(), serde_json::to_vec(&annotated).unwrap());
    let plain = ConversationRequest::from_items(vec![ConversationItem::user("question"), plain]);
    let annotated = ConversationRequest::from_items(vec![ConversationItem::user("question"), annotated]);
    let plain_chat: crate::ChatCompletionRequest = plain.clone().into();
    let annotated_chat: crate::ChatCompletionRequest = annotated.clone().into();
    assert_eq!(serde_json::to_vec(&plain_chat).unwrap(), serde_json::to_vec(&annotated_chat).unwrap());
    let plain_responses: crate::rs::CreateResponse = (&plain).into();
    let annotated_responses: crate::rs::CreateResponse = (&annotated).into();
    assert_eq!(serde_json::to_vec(&plain_responses).unwrap(), serde_json::to_vec(&annotated_responses).unwrap());
    assert_eq!(serde_json::to_vec(&crate::build_messages_request(&plain)).unwrap(), serde_json::to_vec(&crate::build_messages_request(&annotated)).unwrap());
}
