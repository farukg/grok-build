//! The session's context policy, kept beside its other per-session files.

use xai_grok_sampling_types::ContextPolicy;

const CONTEXT_POLICY_FILE: &str = "context_policy.json";

/// The saved policy, or the default when none was saved or the file is unreadable.
pub(crate) fn load(session_dir: &std::path::Path) -> ContextPolicy {
    let path = session_dir.join(CONTEXT_POLICY_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ContextPolicy::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(%error, path = %path.display(), "ignoring unreadable context policy");
        ContextPolicy::default()
    })
}

/// Best-effort: a policy that cannot be saved still applies to the running session.
pub(crate) fn save(session_dir: &std::path::Path, policy: &ContextPolicy) {
    if !session_dir.is_dir() {
        return;
    }
    let path = session_dir.join(CONTEXT_POLICY_FILE);
    let written = serde_json::to_vec(policy)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&path, bytes));
    if let Err(error) = written {
        tracing::warn!(%error, path = %path.display(), "failed to persist context policy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_grok_sampling_types::{ContextCategory, ContextSwitch};

    #[test]
    fn a_saved_policy_comes_back_and_a_missing_one_is_the_default() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(load(dir.path()).is_unrestricted());

        let mut policy = ContextPolicy::default();
        policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
        save(dir.path(), &policy);

        assert_eq!(load(dir.path()), policy);
    }
}
