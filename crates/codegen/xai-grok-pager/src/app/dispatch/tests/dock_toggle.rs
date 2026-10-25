//! `/dock` cycles durable placement while preserving the legacy feature gate.
use super::*;
use crate::views::dock::DockPlacement;

#[test]
fn plan_refuses_a_pin_and_cycles_placements_otherwise() {
    use crate::app::dispatch::settings::ui::{DockTogglePlan, plan_dock_toggle};
    assert_eq!(plan_dock_toggle(Some(true), None, DockPlacement::Off), DockTogglePlan::Pinned(true));
    assert_eq!(plan_dock_toggle(Some(false), Some(true), DockPlacement::Top), DockTogglePlan::Pinned(false));
    for (current, next) in [
        (DockPlacement::Off, DockPlacement::Top),
        (DockPlacement::Top, DockPlacement::Bottom),
        (DockPlacement::Bottom, DockPlacement::Off),
    ] {
        assert_eq!(plan_dock_toggle(None, None, current), DockTogglePlan::Flip { next, env_wins: false });
    }
}

#[test]
fn dock_toggle_round_trips_placement_and_legacy_override() {
    assert!(crate::app::dock_requirement_pin().is_none());
    crate::views::dock::set_enabled_for_test(false);
    let root = tempfile::tempdir().unwrap();
    let mut app = test_app_with_agent();
    app.agents.get_mut(&crate::app::agent::AgentId(0)).unwrap().open_panel_settings_in(root.path());
    for next in [DockPlacement::Top, DockPlacement::Bottom, DockPlacement::Off] {
        let effects = dispatch(Action::ToggleDock, &mut app);
        assert_eq!(crate::views::dock::placement(), next);
        assert!(matches!(effects.as_slice(), [Effect::PersistFeatureOverride {
            feature: xai_grok_shell::agent::config::Feature::Dock, saved: Some(value),
        }] if *value == next.is_visible()));
        let reopened = crate::app::panel_settings::PanelSettingsStore::open(root.path(), None).store;
        assert_eq!(reopened.global().dock_placement(), Some(next));
    }
}
