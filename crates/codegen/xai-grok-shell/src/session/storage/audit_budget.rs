use serde::Serialize;

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ISSUES: usize = 4096;
pub const MAX_INVENTORY_ENTRIES: usize = 100_000;
const MAX_INVENTORY_DEPTH: usize = 128;
const MAX_INVENTORY_PATH_BYTES: usize = 16 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy)]
pub struct AuditLimits {
    pub issues: usize,
    pub inventory_entries: usize,
    pub response_bytes: usize,
    pub history_items: usize,
    pub inventory_path_bytes: usize,
    pub inventory_depth: usize,
    pub decode_bytes: u64,
}

impl Default for AuditLimits {
    fn default() -> Self {
        Self { issues: MAX_ISSUES, inventory_entries: MAX_INVENTORY_ENTRIES, response_bytes: MAX_RESPONSE_BYTES, history_items: 100_000, inventory_path_bytes: MAX_INVENTORY_PATH_BYTES, inventory_depth: MAX_INVENTORY_DEPTH, decode_bytes: MAX_DECODE_BYTES }
    }
}

impl AuditLimits {
    pub(super) fn bounded(self) -> Self {
        let maximum = Self::default();
        Self {
            issues: self.issues.min(maximum.issues),
            inventory_entries: self.inventory_entries.min(maximum.inventory_entries),
            response_bytes: self.response_bytes.min(maximum.response_bytes),
            history_items: self.history_items.min(maximum.history_items),
            inventory_path_bytes: self.inventory_path_bytes.min(maximum.inventory_path_bytes),
            inventory_depth: self.inventory_depth.min(maximum.inventory_depth),
            decode_bytes: self.decode_bytes.min(maximum.decode_bytes),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    InventoryEntries,
    InventoryPaths,
    InventoryDepth,
    Issues,
    HistoryItems,
    ResponseBytes,
    DecodeBytes,
}

#[derive(Debug, thiserror::Error)]
#[error("session_audit_budget_exceeded")]
pub struct BudgetExceeded {
    pub budget: BudgetKind,
    pub observed_files: usize,
    pub observed_issues: usize,
}

pub(super) struct Budget {
    pub limits: AuditLimits,
    pub observed_files: usize,
    pub observed_issues: usize,
    response_bytes: usize,
}

impl Budget {
    pub fn new(limits: AuditLimits, build: &str) -> Self {
        Self { limits: limits.bounded(), observed_files: 0, observed_issues: 0, response_bytes: 2048_usize.saturating_add(build.len().saturating_mul(6)) }
    }

    pub fn exceeded(&self, budget: BudgetKind) -> BudgetExceeded {
        BudgetExceeded { budget, observed_files: self.observed_files, observed_issues: self.observed_issues }
    }

    pub fn response(&mut self, bytes: usize) -> Result<(), BudgetExceeded> {
        self.response_bytes = self.response_bytes.saturating_add(bytes);
        if self.response_bytes > self.limits.response_bytes { return Err(self.exceeded(BudgetKind::ResponseBytes)); }
        Ok(())
    }

    pub fn issue_capacity(&self, path: &str) -> usize {
        self.limits.response_bytes.saturating_sub(self.response_bytes) / issue_bytes(path)
    }

    pub fn issue(&mut self, path: &str) -> Result<(), BudgetExceeded> {
        self.observed_issues += 1;
        if self.observed_issues > self.limits.issues { return Err(self.exceeded(BudgetKind::Issues)); }
        self.response(issue_bytes(path))
    }
}

pub(super) fn file_bytes(path: &str) -> usize {
    512_usize.saturating_add(path.len().saturating_mul(6))
}

pub(super) fn issue_bytes(path: &str) -> usize {
    384_usize.saturating_add(path.len().saturating_mul(6))
}

pub(super) struct InventoryBudget {
    entries: usize,
    path_bytes: usize,
    limits: AuditLimits,
}

impl InventoryBudget {
    pub fn new(limits: AuditLimits) -> Self { Self { entries: 0, path_bytes: 0, limits } }

    pub fn entry(&mut self, bytes: usize) -> Result<(), BudgetKind> {
        if self.entries >= self.limits.inventory_entries { return Err(BudgetKind::InventoryEntries); }
        self.path_bytes = self.path_bytes.saturating_add(bytes);
        if self.path_bytes > self.limits.inventory_path_bytes { return Err(BudgetKind::InventoryPaths); }
        self.entries += 1;
        Ok(())
    }
}
