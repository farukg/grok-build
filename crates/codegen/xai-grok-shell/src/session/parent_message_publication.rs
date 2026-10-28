use std::io;
use serde::{Deserialize, Serialize};
use crate::session::persistence::{PersistedAgent, PersistedAgentSelection, Summary, WakeSummaryState};
use xai_grok_tools::implementations::grok_build::task::native_recovery::{InitialPublication, NativeWakeLaunch, payload_digest};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PublicationAgentRef { Absent, Named { name: String }, Inline { name: String, digest: String } }

impl PublicationAgentRef {
    fn inline(name: String, mut definition: serde_json::Value) -> Self {
        definition.sort_all_objects();
        Self::Inline { name, digest: payload_digest(&definition.to_string()) }
    }

    fn capture_selection(selection: &PersistedAgentSelection) -> io::Result<Self> {
        #[derive(Deserialize)]
        struct ReferenceFields { agent_name: Option<String>, agent_profile: Option<serde_json::Value> }
        let fields: ReferenceFields = serde_json::from_value(serde_json::to_value(selection).map_err(io::Error::other)?).map_err(io::Error::other)?;
        Ok(match (fields.agent_name, fields.agent_profile) {
            (None, None | Some(_)) => Self::Absent,
            (Some(name), None) => Self::Named { name },
            (Some(name), Some(definition)) => Self::inline(name, definition),
        })
    }

    fn capture(summary: &Summary) -> Self {
        match summary.persisted_agent() {
            None => Self::Absent,
            Some(PersistedAgent::Named(name)) => Self::Named { name: name.clone() },
            Some(PersistedAgent::Inline(definition)) => {
                Self::inline(definition.name.clone(), definition.to_json_value())
            }
        }
    }

    fn restore(&self, summary: &mut Summary) -> io::Result<()> {
        match self {
            Self::Absent => summary.agent = PersistedAgentSelection::default(),
            Self::Named { name } => {
                summary.agent = PersistedAgentSelection::default();
                summary.set_agent(PersistedAgent::Named(name.clone()));
            }
            Self::Inline { .. } if self == &Self::capture(summary) => {}
            Self::Inline { .. } => return Err(io::Error::new(io::ErrorKind::NotFound, "original inline agent capability unavailable")),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentMessagePublication {
    attempt_id: Option<String>,
    next_trace_turn: u64,
    current_model_id: agent_client_protocol::ModelId,
    agent: PublicationAgentRef,
    reasoning_effort: Option<xai_grok_sampling_types::ReasoningEffort>,
}

impl ParentMessagePublication {
    pub(crate) fn capture(prior: &WakeSummaryState) -> io::Result<Self> {
        let selection: PersistedAgentSelection = serde_json::from_slice(&prior.summary_bytes).map_err(io::Error::other)?;
        Ok(Self { attempt_id: prior.attempt_id.clone(), next_trace_turn: prior.next_trace_turn,
            current_model_id: prior.current_model_id.clone(), agent: PublicationAgentRef::capture_selection(&selection)?,
            reasoning_effort: prior.reasoning_effort })
    }

    fn restore_fields(&self, summary: &mut Summary) -> io::Result<()> {
        self.agent.restore(summary)?;
        summary.attempt_id = self.attempt_id.clone();
        summary.next_trace_turn = self.next_trace_turn;
        summary.current_model_id = self.current_model_id.clone();
        summary.reasoning_effort = self.reasoning_effort;
        Ok(())
    }

    pub(crate) fn prior_state(&self, current: &WakeSummaryState) -> io::Result<WakeSummaryState> {
        let mut summary: Summary = serde_json::from_slice(&current.summary_bytes).map_err(io::Error::other)?;
        self.restore_fields(&mut summary)?;
        Ok(WakeSummaryState { attempt_id: self.attempt_id.clone(), next_trace_turn: self.next_trace_turn,
            current_model_id: self.current_model_id.clone(), agent_name: summary.agent_name().map(str::to_owned),
            reasoning_effort: self.reasoning_effort,
            summary_bytes: serde_json::to_vec(&summary).map_err(io::Error::other)? })
    }

    pub(crate) fn matches_prior(&self, summary: &Summary) -> bool {
        summary.attempt_id == self.attempt_id && summary.next_trace_turn == self.next_trace_turn
            && summary.current_model_id == self.current_model_id && summary.reasoning_effort == self.reasoning_effort
            && self.agent == PublicationAgentRef::capture(summary)
    }

    pub(crate) fn matches_started(summary: &Summary, launch: &NativeWakeLaunch) -> bool {
        summary.attempt_id.as_deref() == Some(launch.attempt_id.as_str())
            && summary.next_trace_turn == launch.turn_number.saturating_add(1)
            && Some(summary.current_model_id.0.as_ref()) == launch.runtime_overrides.model.as_deref()
            && summary.agent_name() == Some(launch.publication_agent_name.as_str())
            && summary.reasoning_effort == launch.publication_reasoning_effort
    }

    pub(crate) fn reconcile(&self, summary: &mut Summary, launch: &NativeWakeLaunch, publication: InitialPublication) -> io::Result<()> {
        match publication {
            InitialPublication::Unpublished if self.matches_prior(summary) => Ok(()),
            InitialPublication::Unpublished if Self::matches_started(summary, launch) => self.restore_fields(summary),
            InitialPublication::Published if Self::matches_started(summary, launch) => Ok(()),
            InitialPublication::Unpublished | InitialPublication::Published => Err(io::Error::new(io::ErrorKind::InvalidData, "native wake publication context changed")),
        }
    }
}
