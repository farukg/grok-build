//! `/dock`: toggle the experimental dock at runtime.
//!
//! Flips the process gate immediately and persists an explicit `[features].dock`
//! override so the choice survives a restart. A `requirements.toml` pin outranks
//! the command; dispatch reports that instead of changing anything.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct DockCommand;

impl SlashCommand for DockCommand {
    slash_meta! {
        name: "dock",
        description: "Toggle the dock (experimental; tasks consolidate above the prompt and in the F7 column)",
        usage: "/dock",
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::ToggleDock)
    }
}
