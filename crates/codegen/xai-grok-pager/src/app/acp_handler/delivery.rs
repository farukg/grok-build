use super::*;
use crate::app::agent_view::load_continuation::{LoadDelivery, with_load_delivery};

pub(super) enum AcpDelivery {
    Work,
    History,
    Observation,
}

impl AcpDelivery {
    pub(super) fn classify(update: &acp::SessionUpdate, meta: &NotificationMeta) -> Self {
        if meta.is_replay {
            Self::History
        } else if crate::acp::tracker::is_agent_output_update(update)
            || matches!(update, acp::SessionUpdate::UserMessageChunk(_))
        {
            Self::Work
        } else {
            Self::Observation
        }
    }

    pub(super) fn apply<T>(self, app: &mut AppView, id: AgentId, apply: impl FnOnce(&mut AppView) -> T) -> T {
        match self {
            Self::Work => with_load_delivery(app, id, LoadDelivery::Live, apply),
            Self::History => with_load_delivery(app, id, LoadDelivery::Historical, apply),
            Self::Observation => apply(app),
        }
    }
}
