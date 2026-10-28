use super::{AuditLimits, AuditResponse, AuditResult, BudgetKind, audit, audit_with_limits, render};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    let session = temp.path().join("g1/session");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(session.join("summary.json"), r#"{"info":{"id":"session","cwd":"/workspace"},"session_summary":"kept","created_at":"2026-10-05T00:00:00Z","updated_at":"2026-10-05T00:00:00Z","num_messages":2,"current_model_id":"grok-4","chat_format_version":1}"#).unwrap();
    std::fs::write(session.join("chat_history.jsonl"), b"{\"type\":\"user\",\"content\":[]}\n{\"type\":\"user\",\"content\":[]}\n").unwrap();
    temp
}

#[test]
fn lower_inventory_and_decode_budgets_produce_only_incomplete_observations() {
    let temp = fixture();
    let defaults = AuditLimits::default();
    for (limits, expected, hashed) in [
        (AuditLimits { inventory_path_bytes: 1, ..defaults }, BudgetKind::InventoryPaths, 0),
        (AuditLimits { inventory_depth: 1, ..defaults }, BudgetKind::InventoryDepth, 0),
        (AuditLimits { decode_bytes: 10, ..defaults }, BudgetKind::DecodeBytes, 1),
        (AuditLimits { history_items: 1, ..defaults }, BudgetKind::HistoryItems, 2),
    ] {
        let response = audit_with_limits(temp.path(), "fixture (abcdef012345)", limits);
        assert!(matches!(response, AuditResponse::Incomplete { budget, observed_files, observed_issues: 0, .. } if budget == expected && observed_files == hashed));
        let text = render(&response).unwrap();
        assert!(!text.contains("[[files]]"));
        assert!(!text.contains("records = "));
        assert!(!text.contains("supported_no_transform"));
    }
    let complete = audit(temp.path(), "fixture (abcdef012345)");
    assert!(matches!(complete, AuditResponse::Audit { result: AuditResult::SupportedNoTransform, ref files, ref issues, .. } if files.len() == 2 && issues.is_empty()));
}

#[test]
fn response_budget_reaches_the_history_diagnostic_allocation_owner() {
    let temp = fixture();
    std::fs::write(temp.path().join("g1/session/chat_history.jsonl"), "{private-sentinel\n".repeat(100)).unwrap();
    let response = audit_with_limits(temp.path(), "fixture (abcdef012345)", AuditLimits { response_bytes: 4096, ..AuditLimits::default() });
    let AuditResponse::Incomplete { budget, observed_issues, .. } = response else { panic!("expected incomplete coverage"); };
    assert_eq!(budget, BudgetKind::ResponseBytes);
    assert!(observed_issues > 0 && observed_issues < 100);
}
