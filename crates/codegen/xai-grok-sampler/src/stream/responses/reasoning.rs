use xai_grok_sampling_types::rs::{OutputItem, ResponseStreamEvent};

#[derive(Debug, PartialEq, Eq)]
enum SectionKind {
    Summary(u32),
    Text(u32),
}

#[derive(Debug)]
pub(super) struct SectionKey {
    item_id: String,
    output_index: u32,
    kind: SectionKind,
}

impl SectionKey {
    fn matches(&self, item_id: &str, output_index: u32, kind: &SectionKind) -> bool {
        self.item_id == item_id && self.output_index == output_index && self.kind == *kind
    }
}

#[derive(Debug, Default)]
pub(super) enum ReasoningSections {
    #[default]
    Empty,
    Active(SectionKey),
    BetweenSections,
}

impl ReasoningSections {
    pub(super) fn observe(&mut self, event: &ResponseStreamEvent) -> bool {
        match event {
            ResponseStreamEvent::ResponseReasoningSummaryTextDelta(event) => {
                if event.delta.is_empty() {
                    return false;
                }
                self.start(
                    &event.item_id,
                    event.output_index,
                    SectionKind::Summary(event.summary_index),
                )
            }
            ResponseStreamEvent::ResponseReasoningTextDelta(event) => {
                if event.delta.is_empty() {
                    return false;
                }
                self.start(
                    &event.item_id,
                    event.output_index,
                    SectionKind::Text(event.content_index),
                )
            }
            ResponseStreamEvent::ResponseReasoningSummaryPartAdded(event) => {
                if let Self::Active(current) = self {
                    if !current.matches(
                        &event.item_id,
                        event.output_index,
                        &SectionKind::Summary(event.summary_index),
                    ) {
                        *self = Self::BetweenSections;
                    }
                }
                false
            }
            ResponseStreamEvent::ResponseReasoningSummaryPartDone(event) => {
                self.finish(
                    &event.item_id,
                    event.output_index,
                    SectionKind::Summary(event.summary_index),
                );
                false
            }
            ResponseStreamEvent::ResponseReasoningSummaryTextDone(event) => {
                self.finish(
                    &event.item_id,
                    event.output_index,
                    SectionKind::Summary(event.summary_index),
                );
                false
            }
            ResponseStreamEvent::ResponseReasoningTextDone(event) => {
                self.finish(
                    &event.item_id,
                    event.output_index,
                    SectionKind::Text(event.content_index),
                );
                false
            }
            ResponseStreamEvent::ResponseOutputItemDone(event) => {
                if let Self::Active(current) = self {
                    if current.output_index == event.output_index {
                        *self = Self::BetweenSections;
                    }
                }
                false
            }
            ResponseStreamEvent::ResponseOutputItemAdded(event) => {
                if !matches!(&event.item, OutputItem::Reasoning(_)) {
                    self.interrupt();
                }
                false
            }
            _ => false,
        }
    }

    fn start(&mut self, item_id: &str, output_index: u32, kind: SectionKind) -> bool {
        let separator = match self {
            Self::Empty => false,
            Self::Active(current) => {
                if current.matches(item_id, output_index, &kind) {
                    return false;
                }
                true
            }
            Self::BetweenSections => true,
        };
        *self = Self::Active(SectionKey {
            item_id: item_id.to_owned(),
            output_index,
            kind,
        });
        separator
    }

    fn finish(&mut self, item_id: &str, output_index: u32, kind: SectionKind) {
        if let Self::Active(current) = self {
            if current.matches(item_id, output_index, &kind) {
                *self = Self::BetweenSections;
            }
        }
    }

    fn interrupt(&mut self) {
        match self {
            Self::Active(_) => *self = Self::BetweenSections,
            Self::Empty | Self::BetweenSections => {}
        }
    }
}
