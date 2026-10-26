use super::*;
use futures_util::stream;

fn summary(item_id: &str, output_index: u32, summary_index: u32, delta: &str) -> rs::ResponseStreamEvent {
    rs::ResponseStreamEvent::ResponseReasoningSummaryTextDelta(rs::ResponseReasoningSummaryTextDeltaEvent {
        sequence_number: 0,
        item_id: item_id.to_owned(),
        output_index,
        summary_index,
        delta: delta.to_owned(),
    })
}

fn part_added(summary_index: u32) -> rs::ResponseStreamEvent {
    rs::ResponseStreamEvent::ResponseReasoningSummaryPartAdded(rs::ResponseReasoningSummaryPartAddedEvent {
        sequence_number: 0,
        item_id: "reasoning-1".to_owned(),
        output_index: 0,
        summary_index,
        part: rs::SummaryPart::SummaryText(rs::SummaryTextContent { text: String::new() }),
    })
}

fn part_done(summary_index: u32) -> rs::ResponseStreamEvent {
    rs::ResponseStreamEvent::ResponseReasoningSummaryPartDone(rs::ResponseReasoningSummaryPartDoneEvent {
        sequence_number: 0,
        item_id: "reasoning-1".to_owned(),
        output_index: 0,
        summary_index,
        part: rs::SummaryPart::SummaryText(rs::SummaryTextContent { text: "finished".to_owned() }),
    })
}

async fn reasoning_tokens(mut events: Vec<rs::ResponseStreamEvent>) -> Vec<String> {
    let mut terminal = super::tests::completed_event();
    let rs::ResponseStreamEvent::ResponseCompleted(completed) = &mut terminal else {
        panic!("expected completed event");
    };
    completed.response.output.push(serde_json::from_str(
        r#"{"type":"message","id":"msg-1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"answer","annotations":[],"logprobs":[]}]}"#,
    ).unwrap());
    events.push(terminal);
    let events = super::tests::collect(stream_responses(
        stream::iter(events.into_iter().map(Ok)).boxed(),
        None,
        RequestId::from("reasoning-sections-test"),
        Duration::from_secs(60),
        None,
    )).await;
    assert!(matches!(events.last(), Some(SamplingEvent::Completed { .. })));
    events.into_iter().filter_map(|event| match event {
        SamplingEvent::ChannelToken { channel: SamplingChannel::Reasoning, text, .. } => Some(text),
        _ => None,
    }).collect()
}

#[tokio::test]
async fn summary_part_lifecycle_separates_sections_without_rewriting_deltas() {
    let tokens = reasoning_tokens(vec![
        part_added(0),
        summary("reasoning-1", 0, 0, "**Preparing watcher script**"),
        summary("reasoning-1", 0, 0, " "),
        summary("reasoning-1", 0, 0, "details"),
        part_done(0),
        part_added(1),
        summary("reasoning-1", 0, 1, "**Evaluating helper write**"),
        part_done(1),
    ]).await;
    assert_eq!(tokens, ["**Preparing watcher script**", " ", "details", "\n\n", "**Evaluating helper write**"]);
}

#[tokio::test]
async fn summary_and_output_index_changes_work_without_part_events() {
    let tokens = reasoning_tokens(vec![
        summary("reasoning-1", 0, 0, "first"),
        summary("reasoning-1", 0, 1, "second"),
        summary("reasoning-2", 1, 0, "third"),
        summary("reasoning-3", 1, 0, "fourth"),
    ]).await;
    assert_eq!(tokens, ["first", "\n\n", "second", "\n\n", "third", "\n\n", "fourth"]);
}

#[tokio::test]
async fn empty_parts_and_repeated_done_events_add_no_extra_separator() {
    let tokens = reasoning_tokens(vec![
        part_added(0),
        part_done(0),
        part_added(1),
        summary("reasoning-1", 0, 1, "first"),
        rs::ResponseStreamEvent::ResponseReasoningSummaryTextDone(rs::ResponseReasoningSummaryTextDoneEvent {
            sequence_number: 0,
            item_id: "reasoning-1".to_owned(),
            output_index: 0,
            summary_index: 1,
            text: "first".to_owned(),
        }),
        part_done(1),
        part_done(1),
        part_added(2),
        part_done(2),
        summary("reasoning-1", 0, 3, "second"),
        summary("reasoning-1", 0, 3, "\n"),
        summary("reasoning-1", 0, 3, "body"),
    ]).await;
    assert_eq!(tokens, ["first", "\n\n", "second", "\n", "body"]);
}

#[tokio::test]
async fn raw_reasoning_content_index_changes_keep_section_boundaries() {
    let raw = |content_index, delta: &str| {
        rs::ResponseStreamEvent::ResponseReasoningTextDelta(rs::ResponseReasoningTextDeltaEvent {
            sequence_number: 0,
            item_id: "reasoning-1".to_owned(),
            output_index: 0,
            content_index,
            delta: delta.to_owned(),
        })
    };
    let tokens = reasoning_tokens(vec![raw(0, "one"), raw(0, " "), raw(0, "two"), raw(1, "three")]).await;
    assert_eq!(tokens, ["one", " ", "two", "\n\n", "three"]);
}
