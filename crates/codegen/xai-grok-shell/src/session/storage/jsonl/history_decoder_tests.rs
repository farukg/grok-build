use super::history_decoder::{HistoryImageIssue, HistoryIssue, decode_history};
use super::{CHAT_FORMAT_VERSION, JsonlStorageAdapter};
use crate::sampling::types::ChatRequestMessage;
use crate::sampling::{ContentPart, ConversationItem};
use tempfile::TempDir;

const LEGACY_USER: &str = r#"{"role":"user","content":"legacy"}"#;
const CURRENT_USER: &str = r#"{"type":"user","content":[{"type":"text","text":"current"}]}"#;

fn assert_user(item: &ConversationItem, expected: &str) {
    let ConversationItem::User(user) = item else {
        panic!("expected user");
    };
    assert!(matches!(user.content.as_slice(), [ContentPart::Text { text }] if text.as_ref() == expected));
}

#[test]
fn both_genuine_fallback_directions_match_normal_loading() {
    assert!(serde_json::from_str::<ConversationItem>(LEGACY_USER).is_err());
    assert!(serde_json::from_str::<ChatRequestMessage>(CURRENT_USER).is_err());
    for (version, fixture, text) in [
        (CHAT_FORMAT_VERSION, LEGACY_USER, "legacy"),
        (0, CURRENT_USER, "current"),
    ] {
        let decoded = decode_history(fixture.as_bytes(), version, usize::MAX).unwrap();
        assert_eq!(decoded.issues().count(), 0);
        assert_eq!(decoded.items().len(), 1);
        assert_user(&decoded.items()[0], text);
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("chat_history.jsonl");
        std::fs::write(&path, fixture).unwrap();
        let loaded = JsonlStorageAdapter::with_root(temp.path().to_path_buf())
            .read_chat_history_sync(path.clone(), version).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_user(&loaded[0], text);
        assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
        assert!(!path.with_extension("jsonl.corrupt").exists());
    }
}

#[test]
fn version_selects_preferred_schema_when_both_are_valid() {
    let fixture = br#"{"type":"system","role":"user","content":"ambiguous"}"#;
    let current = decode_history(fixture, CHAT_FORMAT_VERSION, usize::MAX).unwrap();
    let legacy = decode_history(fixture, 0, usize::MAX).unwrap();
    assert!(matches!(current.items(), [ConversationItem::System(system)] if system.content.as_ref() == "ambiguous"));
    assert_user(&legacy.items()[0], "ambiguous");
}

#[test]
fn streamwide_backend_ids_preserve_sibling_order() {
    let fixture = concat!(
        "{\"type\":\"backend_tool_call\",\"kind\":{\"tool_type\":\"web_search\",\"id\":\"ws_prior\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}}}\n",
        "{\"type\":\"assistant\",\"content\":\"one\",\"raw_output\":[{\"type\":\"reasoning\",\"id\":\"r1\",\"summary\":[]},{\"type\":\"web_search_call\",\"id\":\"ws_prior\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}},{\"type\":\"web_search_call\",\"id\":\"ws_new\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}}]}\n",
        "{\"type\":\"assistant\",\"content\":\"two\",\"raw_output\":[{\"type\":\"web_search_call\",\"id\":\"ws_new\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}}]}\n",
        "{\"role\":\"assistant\",\"content\":\"three\",\"reasoning_content\":\"legacy thought\"}\n",
    );
    for version in [0, CHAT_FORMAT_VERSION] {
        let decoded = decode_history(fixture.as_bytes(), version, usize::MAX).unwrap();
        assert_eq!(decoded.issues().count(), 0);
        assert!(matches!(decoded.items(), [
            ConversationItem::BackendToolCall(prior),
            ConversationItem::Reasoning(reasoning),
            ConversationItem::BackendToolCall(new),
            ConversationItem::Assistant(one),
            ConversationItem::Assistant(two),
            ConversationItem::Reasoning(_),
            ConversationItem::Assistant(three),
        ] if prior.id() == "ws_prior" && reasoning.id == "r1" && new.id() == "ws_new"
            && one.content.as_ref() == "one" && two.content.as_ref() == "two" && three.content.as_ref() == "three"));
        assert_eq!(decode_history(fixture.as_bytes(), version, 6).err().unwrap().line, 4);
    }
}

#[test]
fn corrupt_lines_and_images_are_reported_without_repairing_pure_items() {
    let fixture = concat!(
        "\n{\"type\":\"user\",\"content\":[{\"type\":\"image\",\"url\":\"data:image/png;base64,!!!\"}]}\n",
        "{torn\n",
        "{\"type\":\"unknown\",\"private\":\"not a diagnostic\"}\n",
        "{\"type\":\"tool_result\",\"tool_call_id\":\"t\",\"content\":\"ok\",\"images\":[{\"type\":\"image\",\"url\":\"data:image/png;base64,bm90LWFuLWltYWdl\"},{\"type\":\"image\",\"url\":\"https://example.invalid/image\"}]}\n",
    );
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("chat_history.jsonl");
    std::fs::write(&path, fixture).unwrap();
    let decoded = decode_history(&std::fs::read(&path).unwrap(), CHAT_FORMAT_VERSION, usize::MAX).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    let issues: Vec<_> = decoded.issues().collect();
    assert_eq!(issues, [
        HistoryIssue::InvalidJson { line: 3 },
        HistoryIssue::InvalidRecord { line: 4 },
        HistoryIssue::InvalidImage { line: 2, part: 0, cause: HistoryImageIssue::Base64 },
        HistoryIssue::InvalidImage { line: 5, part: 0, cause: HistoryImageIssue::Content },
    ]);
    assert!(!format!("{issues:?}").contains("not a diagnostic"));
    assert!(matches!(&decoded.items()[0], ConversationItem::User(user)
        if matches!(user.content.as_slice(), [ContentPart::Image { url }] if url.as_ref() == "data:image/png;base64,!!!")));
    assert!(matches!(&decoded.items()[1], ConversationItem::ToolResult(tool) if tool.images.len() == 2));
    let loaded = JsonlStorageAdapter::with_root(temp.path().to_path_buf())
        .read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_user(&loaded[0], "[image removed — invalid data]");
    assert!(matches!(&loaded[1], ConversationItem::ToolResult(tool)
        if matches!(tool.images.as_slice(), [ContentPart::Image { url }] if url.as_ref() == "https://example.invalid/image")));
    assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
    assert_eq!(std::fs::read(path.with_extension("jsonl.corrupt")).unwrap(), fixture.as_bytes());
}

#[test]
fn valid_current_and_legacy_images_remain_unmodified() {
    use base64::Engine as _;
    let image = image::ImageBuffer::from_pixel(64, 64, image::Rgba([10u8, 20, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let payload = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let url = format!("data:image/png;BASE64,{payload}");
    for fixture in [
        format!(r#"{{"type":"user","content":[{{"type":"image","url":"{url}"}}]}}"#),
        format!(r#"{{"role":"user","content":[{{"type":"image_url","image_url":{{"url":"{url}"}}}}]}}"#),
    ] {
        for version in [0, CHAT_FORMAT_VERSION] {
            let decoded = decode_history(fixture.as_bytes(), version, usize::MAX).unwrap();
            assert_eq!(decoded.issues().count(), 0);
            assert!(matches!(decoded.items(), [ConversationItem::User(user)]
                if matches!(user.content.as_slice(), [ContentPart::Image { url: loaded }] if loaded.as_ref() == url)));
            let temp = TempDir::new().unwrap();
            let path = temp.path().join("chat_history.jsonl");
            std::fs::write(&path, &fixture).unwrap();
            let loaded = JsonlStorageAdapter::with_root(temp.path().to_path_buf())
                .read_chat_history_sync(path.clone(), version).unwrap();
            assert!(matches!(loaded.as_slice(), [ConversationItem::User(user)]
                if matches!(user.content.as_slice(), [ContentPart::Image { url: loaded }] if loaded.as_ref() == url)));
            assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
            assert!(!path.with_extension("jsonl.corrupt").exists());
        }
    }
}

#[test]
fn whitespace_and_invalid_utf8_keep_physical_line_numbers() {
    let fixture = b"\r\n\t\n\xff\n{\"role\":\"user\",\"content\":\"legacy\"}\r\n";
    let decoded = decode_history(fixture, CHAT_FORMAT_VERSION, usize::MAX).unwrap();
    assert_eq!(decoded.issues().collect::<Vec<_>>(), [HistoryIssue::InvalidJson { line: 3 }]);
    assert_user(&decoded.items()[0], "legacy");
}

#[test]
fn rejected_records_do_not_emit_siblings_or_reserve_backend_ids() {
    let fixture = concat!(
        "{\"type\":\"assistant\",\"content\":42,\"raw_output\":[{\"type\":\"web_search_call\",\"id\":\"ws_kept\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}}]}\n",
        "{\"type\":\"assistant\",\"content\":\"kept\",\"raw_output\":[{\"type\":\"web_search_call\",\"id\":\"ws_kept\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"q\",\"sources\":[]}}]}\n",
    );
    for version in [0, CHAT_FORMAT_VERSION] {
        let decoded = decode_history(fixture.as_bytes(), version, usize::MAX).unwrap();
        assert_eq!(decoded.issues().collect::<Vec<_>>(), [HistoryIssue::InvalidRecord { line: 1 }]);
        assert!(matches!(decoded.items(), [ConversationItem::BackendToolCall(tool), ConversationItem::Assistant(assistant)]
            if tool.id() == "ws_kept" && assistant.content.as_ref() == "kept"));
    }
}

#[test]
fn image_rejection_causes_preserve_legacy_loading_semantics() {
    use base64::Engine as _;
    let oversized = format!("data:image/png;base64,{}", "A".repeat((super::MAX_LOADED_IMAGE_BYTES + 1) * 4 / 3 + 4));
    let image = image::ImageBuffer::from_pixel(8, 8, image::Rgba([10u8, 20, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let small = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()));
    for (url, cause) in [
        ("data:image/png;base64".to_owned(), HistoryImageIssue::Header),
        ("data:image/png,AAAA".to_owned(), HistoryImageIssue::Header),
        (oversized, HistoryImageIssue::Size),
        ("data:image/png;base64,!!!".to_owned(), HistoryImageIssue::Base64),
        ("data:image/png;base64,".to_owned(), HistoryImageIssue::Content),
        ("data:image/png;base64,iVBORw0KGgo=".to_owned(), HistoryImageIssue::Content),
        (small, HistoryImageIssue::Content),
    ] {
        for fixture in [
            format!(r#"{{"type":"user","content":[{{"type":"image","url":"{url}"}}]}}"#),
            format!(r#"{{"role":"user","content":[{{"type":"image_url","image_url":{{"url":"{url}"}}}}]}}"#),
        ] {
            for version in [0, CHAT_FORMAT_VERSION] {
                let decoded = decode_history(fixture.as_bytes(), version, usize::MAX).unwrap();
                assert_eq!(decoded.issues().collect::<Vec<_>>(), [HistoryIssue::InvalidImage { line: 1, part: 0, cause }]);
                assert!(matches!(decoded.items(), [ConversationItem::User(user)]
                    if matches!(user.content.as_slice(), [ContentPart::Image { url: original }] if original.as_ref() == url)));
            }
        }
    }
}

#[test]
fn multiple_image_rejections_keep_source_offsets_and_survivor_order() {
    let fixture = concat!(
        r#"{"type":"tool_result","tool_call_id":"t","content":"ok","images":[{"type":"image","url":"https://example.invalid/first"},{"type":"image","url":"data:image/png;base64,!!!"},{"type":"image","url":"data:image/png,AAAA"},{"type":"text","text":"retained"},{"type":"image","url":"https://example.invalid/last"}]}"#,
        "\n",
        r#"{"type":"user","content":[{"type":"text","text":"before"},{"type":"image","url":"data:image/png;base64,!!!"},{"type":"image","url":"https://example.invalid/user"}]}"#,
    );
    let decoded = decode_history(fixture.as_bytes(), CHAT_FORMAT_VERSION, usize::MAX).unwrap();
    assert_eq!(decoded.issues().collect::<Vec<_>>(), [
        HistoryIssue::InvalidImage { line: 1, part: 1, cause: HistoryImageIssue::Base64 },
        HistoryIssue::InvalidImage { line: 1, part: 2, cause: HistoryImageIssue::Header },
        HistoryIssue::InvalidImage { line: 2, part: 1, cause: HistoryImageIssue::Base64 },
    ]);
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("chat_history.jsonl");
    std::fs::write(&path, fixture).unwrap();
    let quarantine = path.with_extension("jsonl.corrupt");
    std::fs::write(&quarantine, b"earlier corruption evidence").unwrap();
    let loaded = JsonlStorageAdapter::with_root(temp.path().to_path_buf())
        .read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION).unwrap();
    assert!(matches!(loaded.as_slice(), [ConversationItem::ToolResult(tool), ConversationItem::User(user)]
        if matches!(tool.images.as_slice(), [ContentPart::Image { url: first }, ContentPart::Text { text }, ContentPart::Image { url: last }]
            if first.as_ref() == "https://example.invalid/first" && text.as_ref() == "retained" && last.as_ref() == "https://example.invalid/last")
        && matches!(user.content.as_slice(), [ContentPart::Text { text: before }, ContentPart::Text { text: placeholder }, ContentPart::Image { url }]
            if before.as_ref() == "before" && placeholder.as_ref() == "[image removed — invalid data]" && url.as_ref() == "https://example.invalid/user")));
    assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
    assert_eq!(std::fs::read(&quarantine).unwrap(), b"earlier corruption evidence");
}

#[test]
fn nested_legacy_output_damage_is_reported_without_changing_tolerant_loading() {
    use super::history_decoder::LegacyReasoningIssue;
    let fixture = concat!(
        "\n{\"type\":\"assistant\",\"content\":\"kept\",\"raw_output\":[{\"type\":\"future_private\"},{\"type\":\"reasoning\",\"id\":\"r\",\"summary\":[]},{\"type\":\"reasoning\",\"summary\":42}]}\n",
        "{\"type\":\"assistant\",\"content\":\"next\",\"raw_output\":42}\n",
    );
    let decoded = decode_history(fixture.as_bytes(), CHAT_FORMAT_VERSION, usize::MAX).unwrap();
    assert_eq!(decoded.issues().collect::<Vec<_>>(), [
        HistoryIssue::InvalidLegacyReasoning { line: 2, cause: LegacyReasoningIssue::InvalidOutputItem { index: 0 } },
        HistoryIssue::InvalidLegacyReasoning { line: 2, cause: LegacyReasoningIssue::InvalidOutputItem { index: 2 } },
        HistoryIssue::InvalidLegacyReasoning { line: 3, cause: LegacyReasoningIssue::InvalidRawOutput },
    ]);
    assert!(matches!(decoded.items(), [ConversationItem::Reasoning(_), ConversationItem::Assistant(_), ConversationItem::Assistant(_)]));
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("chat_history.jsonl");
    std::fs::write(&path, fixture).unwrap();
    let loaded = JsonlStorageAdapter::with_root(temp.path().to_path_buf()).read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION).unwrap();
    assert_eq!(loaded.len(), decoded.items().len());
    assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
    assert!(!path.with_extension("jsonl.corrupt").exists());
}

#[test]
fn bounded_loader_keeps_limit_errors_without_quarantine() {
    let fixture = format!("{CURRENT_USER}\n{LEGACY_USER}\n");
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("chat_history.jsonl");
    std::fs::write(&path, &fixture).unwrap();
    let limit = decode_history(fixture.as_bytes(), CHAT_FORMAT_VERSION, 1).err().unwrap();
    assert_eq!(limit.line, 2);
    assert_eq!(limit.max_items, 1);
    let adapter = JsonlStorageAdapter::with_root(temp.path().to_path_buf());
    for (max_bytes, max_items) in [(fixture.len(), 1), (fixture.len() - 1, usize::MAX)] {
        let error = adapter.read_chat_history_sync_bounded(path.clone(), CHAT_FORMAT_VERSION, max_bytes, max_items).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), fixture.as_bytes());
        assert!(!path.with_extension("jsonl.corrupt").exists());
    }
}

#[test]
fn bounded_history_accepts_an_exact_item_budget_without_changing_native_fallbacks() {
    use super::history_decoder::{HistoryLimits, decode_history_bounded};
    for (version, fixture, text) in [(0, CURRENT_USER, "current"), (CHAT_FORMAT_VERSION, LEGACY_USER, "legacy")] {
        let decoded = decode_history_bounded(fixture.as_bytes(), version, HistoryLimits { items: 1, issues: 0 }).unwrap();
        assert_eq!(decoded.items().len(), 1);
        assert_eq!(decoded.issues().count(), 0);
        assert_user(&decoded.items()[0], text);
    }
}

#[test]
fn dropped_json_diagnostics_accept_two_and_reject_the_third() {
    use super::history_decoder::{HistoryDecodeLimit, HistoryLimits, decode_history_bounded};
    for version in [0, CHAT_FORMAT_VERSION] {
        let decoded = decode_history_bounded(b"{torn\n{torn\n", version, HistoryLimits { items: 0, issues: 2 }).unwrap();
        assert!(decoded.items().is_empty());
        assert_eq!(decoded.issues().collect::<Vec<_>>(), [
            HistoryIssue::InvalidJson { line: 1 },
            HistoryIssue::InvalidJson { line: 2 },
        ]);
        let limit = decode_history_bounded(b"{torn\n{torn\n{torn\n", version, HistoryLimits { items: 0, issues: 2 }).err().unwrap();
        assert!(matches!(limit, HistoryDecodeLimit::Issues { line: 3, observed: 3 }));
    }
}

#[test]
fn dropped_record_diagnostics_accept_two_and_reject_the_third() {
    use super::history_decoder::{HistoryDecodeLimit, HistoryLimits, decode_history_bounded};
    let record = "{\"type\":\"future\",\"private\":\"not a diagnostic\"}\n";
    for version in [0, CHAT_FORMAT_VERSION] {
        let decoded = decode_history_bounded(record.repeat(2).as_bytes(), version, HistoryLimits { items: 0, issues: 2 }).unwrap();
        assert!(decoded.items().is_empty());
        assert_eq!(decoded.issues().collect::<Vec<_>>(), [
            HistoryIssue::InvalidRecord { line: 1 },
            HistoryIssue::InvalidRecord { line: 2 },
        ]);
        let limit = decode_history_bounded(record.repeat(3).as_bytes(), version, HistoryLimits { items: 0, issues: 2 }).err().unwrap();
        assert!(matches!(limit, HistoryDecodeLimit::Issues { line: 3, observed: 3 }));
    }
}

#[test]
fn legacy_reporting_diagnostics_accept_two_and_reject_the_third() {
    use super::history_decoder::{HistoryDecodeLimit, HistoryLimits, LegacyReasoningIssue, decode_history_bounded};
    for version in [0, CHAT_FORMAT_VERSION] {
        let fixture = r#"{"type":"assistant","content":"kept","raw_output":[{"type":"future"},{"type":"reasoning","summary":42}]}"#;
        let decoded = decode_history_bounded(fixture.as_bytes(), version, HistoryLimits { items: 4, issues: 2 }).unwrap();
        assert!(matches!(decoded.items(), [ConversationItem::Assistant(assistant)] if assistant.content.as_ref() == "kept"));
        assert_eq!(decoded.issues().collect::<Vec<_>>(), [
            HistoryIssue::InvalidLegacyReasoning { line: 1, cause: LegacyReasoningIssue::InvalidOutputItem { index: 0 } },
            HistoryIssue::InvalidLegacyReasoning { line: 1, cause: LegacyReasoningIssue::InvalidOutputItem { index: 1 } },
        ]);
        let overflowing = r#"{"type":"assistant","content":"kept","raw_output":[{"type":"future"},{"type":"reasoning","summary":42},{"type":"future"}]}"#;
        let limit = decode_history_bounded(overflowing.as_bytes(), version, HistoryLimits { items: 4, issues: 2 }).err().unwrap();
        assert!(matches!(limit, HistoryDecodeLimit::Issues { line: 1, observed: 3 }));
    }
}

#[test]
fn image_diagnostics_accept_two_unmodified_parts_and_reject_the_third() {
    use super::history_decoder::{HistoryDecodeLimit, HistoryLimits, decode_history_bounded};
    for version in [0, CHAT_FORMAT_VERSION] {
        for count in [2, 3] {
            let parts = vec![r#"{"type":"image","url":"data:image/png;base64,!!!"}"#; count].join(",");
            let fixture = format!(r#"{{"type":"user","content":[{parts}]}}"#);
            let result = decode_history_bounded(fixture.as_bytes(), version, HistoryLimits { items: 1, issues: 2 });
            if count == 2 {
                let decoded = result.unwrap();
                assert_eq!(decoded.issues().collect::<Vec<_>>(), [
                    HistoryIssue::InvalidImage { line: 1, part: 0, cause: HistoryImageIssue::Base64 },
                    HistoryIssue::InvalidImage { line: 1, part: 1, cause: HistoryImageIssue::Base64 },
                ]);
                assert!(matches!(decoded.items(), [ConversationItem::User(user)]
                    if matches!(user.content.as_slice(), [ContentPart::Image { url: first }, ContentPart::Image { url: second }]
                        if first.as_ref() == "data:image/png;base64,!!!" && second.as_ref() == "data:image/png;base64,!!!")));
            } else {
                assert!(matches!(result.err().unwrap(), HistoryDecodeLimit::Issues { line: 1, observed: 3 }));
            }
        }
    }
}

#[test]
fn raw_output_slots_are_bounded_before_the_shared_upgrader() {
    use super::history_decoder::{HistoryDecodeLimit, HistoryItemLimitExceeded, HistoryLimits, decode_history_bounded};
    let fixture = r#"{"type":"assistant","content":"kept","raw_output":[{"type":"reasoning","id":"r1","summary":[]},{"type":"reasoning","id":"r2","summary":[]}]}"#;
    for version in [0, CHAT_FORMAT_VERSION] {
        let decoded = decode_history_bounded(fixture.as_bytes(), version, HistoryLimits { items: 3, issues: 2 }).unwrap();
        assert_eq!(decoded.issues().count(), 0);
        assert!(matches!(decoded.items(), [ConversationItem::Reasoning(first), ConversationItem::Reasoning(second), ConversationItem::Assistant(assistant)]
            if first.id == "r1" && second.id == "r2" && assistant.content.as_ref() == "kept"));
        let limit = decode_history_bounded(fixture.as_bytes(), version, HistoryLimits { items: 2, issues: 2 }).err().unwrap();
        assert!(matches!(limit, HistoryDecodeLimit::Items {
            limit: HistoryItemLimitExceeded { line: 1, max_items: 2 }, observed_issues: 0,
        }));
        let invalid_slots = r#"{"type":"assistant","content":"kept","raw_output":[{"type":"future"},{"type":"future"}]}"#;
        let limit = decode_history_bounded(invalid_slots.as_bytes(), version, HistoryLimits { items: 2, issues: 0 }).err().unwrap();
        assert!(matches!(limit, HistoryDecodeLimit::Items {
            limit: HistoryItemLimitExceeded { line: 1, max_items: 2 }, observed_issues: 0,
        }));
    }
}
