use super::*;
use crate::types::InjectedRequestRecord;
use xai_grok_sampling_types::{ContextCategory, ContextPolicy, ContextSwitch};

#[derive(Clone, Copy, Debug)]
enum MemoryPersistence {
    RequestOnly,
    Canonical,
}

impl MemoryPersistence {
    fn enabled(self) -> bool {
        match self {
            Self::RequestOnly => false,
            Self::Canonical => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SystemHead {
    Prompt,
    Absent,
}

#[derive(Clone, Copy, Debug)]
enum Exclusion {
    Memory,
    System,
}

#[tokio::test]
async fn withheld_memory_records_follow_the_final_view_without_changing_history_policy() {
    for persistence in [MemoryPersistence::RequestOnly, MemoryPersistence::Canonical] {
        for head in [SystemHead::Prompt, SystemHead::Absent] {
            for exclusion in [Exclusion::Memory, Exclusion::System] {
                let initial = match head {
                    SystemHead::Prompt => vec![ConversationItem::system("Base prompt"), ConversationItem::user("human control")],
                    SystemHead::Absent => vec![ConversationItem::user("human control")],
                };
                let mut h = TestHarness::with_conversation(initial.clone());
                let mut policy = ContextPolicy::default();
                policy.set(match exclusion {
                    Exclusion::Memory => ContextCategory::Memory,
                    Exclusion::System => ContextCategory::CoreInstructions,
                }, ContextSwitch::Excluded);
                h.handle.set_context_policy(policy).await.unwrap();
                let reminder = format!("<memory-context>\n{}\nWITHHELD_UNICODE_TAIL\n</memory-context>", "ä界".repeat(5000));
                let built = h.handle.build_request(vec![], Some(reminder.clone()), persistence.enabled(), None,
                    "visibility".into(), "withheld".into()).await.unwrap();
                assert!(built.items.iter().all(|item| !item.text_content().contains("WITHHELD_UNICODE_TAIL")));
                assert!(!built.injected_records.iter().any(|record| matches!(record, InjectedRequestRecord::MemoryReminder { .. })),
                    "a withheld body cannot be recorded as sent: {persistence:?}, {head:?}, {exclusion:?}");
                assert!(built.injected_records.iter().all(|record| !record.text().contains("WITHHELD_UNICODE_TAIL")));
                let expected_request = match (head, exclusion) {
                    (SystemHead::Prompt, Exclusion::Memory) => vec![ConversationItem::system("Base prompt"), ConversationItem::user("human control")],
                    (SystemHead::Absent, Exclusion::Memory)
                    | (SystemHead::Prompt, Exclusion::System)
                    | (SystemHead::Absent, Exclusion::System) => vec![ConversationItem::user("human control")],
                };
                assert_eq!(serde_json::to_value(payload_items(&built.items)).unwrap(), serde_json::to_value(expected_request).unwrap());
                let expected_history = match persistence {
                    MemoryPersistence::RequestOnly => initial,
                    MemoryPersistence::Canonical => {
                        let system = match head {
                            SystemHead::Prompt => format!("Base prompt\n\n{reminder}"),
                            SystemHead::Absent => reminder.clone(),
                        };
                        vec![ConversationItem::system(system), ConversationItem::user("human control")]
                    }
                };
                let canonical = h.handle.get_conversation().await;
                assert!(canonical.iter().all(|item| !item.source().is_unadmitted()));
                assert_eq!(serde_json::to_value(payload_items(&canonical)).unwrap(), serde_json::to_value(&expected_history).unwrap());
                let records = h.drain_persistence();
                match persistence {
                    MemoryPersistence::Canonical => assert!(records.iter().any(|record| matches!(record,
                        PersistenceRecord::ReplaceHistory(items) if serde_json::to_value(payload_items(items)).unwrap() == serde_json::to_value(&expected_history).unwrap())),
                        "withholding must not suppress independent canonical memory persistence"),
                    MemoryPersistence::RequestOnly => assert!(!records.iter().any(|record|
                        matches!(record, PersistenceRecord::Message(item) if item.text_content().contains("WITHHELD_UNICODE_TAIL"))
                        || matches!(record, PersistenceRecord::ReplaceHistory(items) if items.iter().any(|item| item.text_content().contains("WITHHELD_UNICODE_TAIL")))),
                        "request-only memory cannot reach canonical persistence"),
                }
            }
        }
    }
}
