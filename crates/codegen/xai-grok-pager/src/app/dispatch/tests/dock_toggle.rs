//! `/dock`: the runtime gate flips at once and an explicit `[features].dock`
//! override is persisted; a requirements pin outranks the command.

use super::*;

/// Precedence of the pure planner: a pin refuses, an env override only widens
/// the report, otherwise the runtime gate flips.
#[test]
fn plan_refuses_a_pin_and_flips_otherwise() {
    use crate::app::dispatch::settings::ui::{DockTogglePlan, plan_dock_toggle};
    assert_eq!(
        plan_dock_toggle(Some(true), None, false),
        DockTogglePlan::Pinned(true)
    );
    assert_eq!(
        plan_dock_toggle(Some(false), Some(true), true),
        DockTogglePlan::Pinned(false)
    );
    assert_eq!(
        plan_dock_toggle(None, None, false),
        DockTogglePlan::Flip {
            next: true,
            env_wins: false
        }
    );
    assert_eq!(
        plan_dock_toggle(None, Some(false), true),
        DockTogglePlan::Flip {
            next: false,
            env_wins: true
        }
    );
}

/// Dispatch flips the process gate immediately and persists exactly one
/// explicit dock override per toggle, in both directions.
#[test]
fn dock_toggle_round_trips_the_runtime_gate_and_the_override() {
    assert!(
        crate::app::dock_requirement_pin().is_none(),
        "this test needs no dock pin in the merged requirements.toml"
    );
    crate::views::dock::set_enabled(false);
    let mut app = test_app_with_agent();

    let effects = dispatch(Action::ToggleDock, &mut app);
    assert!(
        crate::views::dock::enabled(),
        "runtime gate must flip on immediately"
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::PersistFeatureOverride {
                feature: xai_grok_shell::agent::config::Feature::Dock,
                saved: Some(true),
            }]
        ),
        "toggle must persist exactly one explicit dock override, got {effects:?}"
    );

    let effects = dispatch(Action::ToggleDock, &mut app);
    assert!(
        !crate::views::dock::enabled(),
        "second toggle flips back off"
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::PersistFeatureOverride {
                feature: xai_grok_shell::agent::config::Feature::Dock,
                saved: Some(false),
            }]
        ),
        "second toggle must persist the off override, got {effects:?}"
    );
}
