use super::state::{PersistedDashboard, PersistedRowId};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ChildLayoutUpdate {
    pub scope: BTreeSet<PersistedRowId>,
    pub pinned: BTreeSet<PersistedRowId>,
    pub reorder: Vec<PersistedRowId>,
}

impl ChildLayoutUpdate {
    pub(crate) fn apply(&self, saved: &mut PersistedDashboard) {
        saved.pinned.retain(|id| !self.scope.contains(id));
        saved
            .pinned
            .extend(self.pinned.intersection(&self.scope).cloned());
        let mut order = self.reorder.iter().filter(|id| self.scope.contains(id));
        saved.reorder = saved
            .reorder
            .iter()
            .filter_map(|id| {
                if self.scope.contains(id) {
                    order.next().cloned()
                } else {
                    Some(id.clone())
                }
            })
            .collect();
        saved.reorder.extend(order.cloned());
    }

    pub fn write(&self) -> std::io::Result<()> {
        let path = super::state::config_path()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no grok home"))?;
        self.write_to_path(&path)
    }

    pub fn write_to_path(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let Some(mut doc) = crate::config_toml_edit::read_config_document_for_edit(path)? else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid dashboard config",
            ));
        };
        let dashboard = doc.entry("dashboard").or_insert(toml_edit::table());
        let Some(table) = dashboard.as_table_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid dashboard table",
            ));
        };
        for key in ["pinned", "reorder"] {
            if table.get(key).is_some_and(|item| item.as_array().is_none()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid dashboard layout",
                ));
            }
        }
        let pins: Vec<_> = self.pinned.intersection(&self.scope).cloned().collect();
        let order: Vec<_> = self
            .reorder
            .iter()
            .filter(|id| self.scope.contains(id))
            .cloned()
            .collect();
        for (key, replacement) in [("pinned", pins), ("reorder", order)] {
            let mut replacement = replacement.iter();
            let mut values = toml_edit::Array::new();
            if let Some(existing) = table.get(key).and_then(|item| item.as_array()) {
                for value in existing {
                    let scoped = value
                        .as_str()
                        .and_then(PersistedRowId::from_key)
                        .is_some_and(|id| self.scope.contains(&id));
                    if scoped {
                        if let Some(id) = replacement.next() {
                            values.push(id.to_key());
                        }
                    } else {
                        values.push_formatted(value.clone());
                    }
                }
            }
            for id in replacement {
                values.push(id.to_key());
            }
            table.insert(key, toml_edit::value(values));
        }
        super::state::atomic_write(path, &doc.to_string())
    }
}

#[cfg(test)]
#[path = "child_layout_tests.rs"]
mod tests;
