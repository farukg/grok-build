use super::{claude_import_marked_with_env, is_claude_import_marked, user_config_file};
use crate::compat::CompatEnv;

#[test]
fn only_an_explicit_true_marker_reads_as_imported() {
    let cases = [
        (None, false),
        (Some("[other]\nkey = \"value\"\n"), false),
        (Some("[claude_compat]\nimported = false\n"), false),
        (Some("[claude_compat]\nimported = "), false),
        (Some("[claude_compat]\nimported = true\n"), true),
    ];

    for (config, expected) in cases {
        let home = tempfile::tempdir().expect("create temp grok home");
        if let Some(config) = config {
            std::fs::write(user_config_file(home.path()), config).expect("write config");
        }

        assert_eq!(expected, is_claude_import_marked(home.path()), "{config:?}");
    }
}

#[test]
fn a_disabled_claude_rules_or_mcps_cell_reads_as_imported() {
    let cases = [
        ("[compat.claude]\nrules = false\nmcps = true\n", None, true),
        ("[compat.claude]\nrules = true\nmcps = false\n", None, true),
        ("[compat.claude]\nrules = true\nmcps = true\n", None, false),
        ("", Some("GROK_CLAUDE_RULES_ENABLED"), true),
        ("", Some("GROK_CLAUDE_MCPS_ENABLED"), true),
        ("[compat.claude]\nrules = false\n", Some("GROK_CLAUDE_SKILLS_ENABLED"), true),
    ];

    for (config, disabled_env, expected) in cases {
        let home = tempfile::tempdir().expect("create temp grok home");
        std::fs::write(user_config_file(home.path()), config).expect("write config");
        let env = CompatEnv::read(|name| (Some(name) == disabled_env).then_some(false));

        assert_eq!(
            expected,
            claude_import_marked_with_env(home.path(), &env),
            "{config:?} {disabled_env:?}"
        );
    }
}
