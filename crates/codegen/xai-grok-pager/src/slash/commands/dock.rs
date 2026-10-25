//! `/dock`: cycle the durable Off / Top / Bottom placement.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct DockCommand;

impl SlashCommand for DockCommand {
    slash_meta! {
        name: "dock",
        description: "Cycle dock placement: Off / Top / Bottom (also in F7)",
        usage: "/dock",
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::ToggleDock)
    }
}
