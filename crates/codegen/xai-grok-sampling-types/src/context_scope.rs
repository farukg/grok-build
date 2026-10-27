use serde::{Deserialize, Serialize};

use crate::{ContextCategory, ContextPolicy, ContextSwitch};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextScope {
    #[default]
    Global,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSetting {
    pub category: ContextCategory,
    pub value: ContextSwitch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContextPolicy {
    pub scope: ContextScope,
    #[serde(default)]
    pub overrides: Vec<ContextSetting>,
}

impl SessionContextPolicy {
    pub fn effective(&self, global: &ContextPolicy) -> ContextPolicy {
        let mut policy = global.clone();
        match self.scope {
            ContextScope::Global => {},
            ContextScope::Session => {
                for setting in &self.overrides {
                    policy.set(setting.category, setting.value);
                }
            }
        }
        policy
    }

    pub fn set(&mut self, category: ContextCategory, value: ContextSwitch) {
        match self.overrides.iter_mut().find(|setting| setting.category == category) {
            Some(setting) => setting.value = value,
            None => self.overrides.push(ContextSetting { category, value }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicyVersion {
    pub revision: u64,
    pub incarnation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicySelection {
    pub version: ContextPolicyVersion,
    pub scope: ContextScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<ContextCategory>", into = "Vec<ContextCategory>")]
pub struct ContextPolicyCategories {
    first: ContextCategory,
    rest: Vec<ContextCategory>,
}

impl ContextPolicyCategories {
    pub fn one(category: ContextCategory) -> Self {
        Self { first: category, rest: Vec::new() }
    }

    pub fn count(&self) -> usize {
        1 + self.rest.len()
    }

    pub fn single(&self) -> Option<ContextCategory> {
        self.rest.is_empty().then_some(self.first)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ContextCategory> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPolicyCategoriesError {
    Empty,
}

impl std::fmt::Display for ContextPolicyCategoriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Empty => f.write_str("Context edit requires at least one category") }
    }
}

impl std::error::Error for ContextPolicyCategoriesError {}

impl TryFrom<Vec<ContextCategory>> for ContextPolicyCategories {
    type Error = ContextPolicyCategoriesError;

    fn try_from(categories: Vec<ContextCategory>) -> Result<Self, Self::Error> {
        let mut categories = categories.into_iter();
        let first = categories.next().ok_or(ContextPolicyCategoriesError::Empty)?;
        let mut seen = std::collections::BTreeSet::from([first]);
        let rest = categories.filter(|category| seen.insert(*category)).collect();
        Ok(Self { first, rest })
    }
}

impl From<ContextPolicyCategories> for Vec<ContextCategory> {
    fn from(categories: ContextPolicyCategories) -> Self {
        std::iter::once(categories.first).chain(categories.rest).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum ContextPolicyMutation {
    SetCategories {
        categories: ContextPolicyCategories,
        value: ContextSwitch,
        target: ContextScope,
    },
    SelectScope { scope: ContextScope },
    ReplaceSession { policy: ContextPolicy },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicyEdit {
    pub expected: ContextPolicyVersion,
    pub mutation: ContextPolicyMutation,
}
