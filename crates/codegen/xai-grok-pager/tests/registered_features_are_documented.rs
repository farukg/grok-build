//! `FEATURES` is the source of truth. The configuration reference is the
//! current operator-facing mirror and must expose every feature key and its
//! environment override.

use xai_grok_shell::agent::config::FEATURES;

const CONFIG_REFERENCE: &str = include_str!("../docs/user-guide/26-config-reference.md");

#[test]
fn every_registered_feature_reaches_the_operator() {
    for spec in FEATURES {
        assert!(
            CONFIG_REFERENCE.contains(&format!("features.{}", spec.key)),
            "{} has no row in the configuration reference",
            spec.key,
        );
        assert!(
            CONFIG_REFERENCE.contains(spec.env),
            "{} is undocumented in the configuration reference",
            spec.env,
        );
    }
}
