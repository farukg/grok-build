use crate::app::actions::TaskResult;
use crate::app::agent::AgentId;
use crate::app::agent_view::{CurrentActionsContext, CurrentActionsReadContext};
use agent_client_protocol as acp;
use xai_acp_lib::{AcpAgentTx, acp_send};
use xai_grok_shell::session::current_actions::{CAPABILITY, CurrentActionsSnapshot};

pub(crate) fn load_result(
    agent_id: AgentId,
    context: CurrentActionsReadContext,
    meta: Option<&acp::Meta>,
    result: TaskResult,
) -> TaskResult {
    let snapshot = xai_grok_shell::extensions::current_actions::load_snapshot(meta)
        .unwrap_or(CurrentActionsSnapshot::Unavailable);
    TaskResult::WithCurrentActions {
        agent_id,
        context,
        snapshot,
        result: Box::new(result),
    }
}

pub(crate) fn prepare(
    effect: &crate::app::actions::Effect,
    app: &mut crate::app::app_view::AppView,
) -> Option<xai_grok_shell::session::current_actions::SnapshotBinding> {
    if let crate::app::actions::Effect::LoadSession {
        agent_id,
        session_id,
        ..
    } = effect
    {
        let view = app.agents.get_mut(agent_id)?;
        let binding = view.current_actions_binding()?;
        if binding.session_id != *session_id {
            return None;
        }
        return view.begin_publication_load();
    }
    None
}

pub(crate) fn load_succeeded(mut result: &TaskResult) -> bool {
    while let TaskResult::WithPinnedMemoryMode { result: inner, .. }
    | TaskResult::WithCurrentActions { result: inner, .. }
    | TaskResult::WithPublication { result: inner, .. } = result
    {
        result = inner;
    }
    matches!(result, TaskResult::SessionLoaded { .. })
}

#[derive(Debug)]
pub(crate) struct LoadPublication {
    pub binding: xai_grok_shell::session::current_actions::SnapshotBinding,
    pub snapshot: CurrentActionsSnapshot,
    pub cut: xai_grok_shell::extensions::current_actions::LoadCut,
}

impl LoadPublication {
    pub(crate) fn read(
        binding: xai_grok_shell::session::current_actions::SnapshotBinding,
        meta: Option<&acp::Meta>,
    ) -> Self {
        use xai_grok_shell::extensions::current_actions::{
            CutUnavailable, LoadCut, load_cut, load_snapshot,
        };
        Self {
            binding,
            snapshot: load_snapshot(meta).unwrap_or(CurrentActionsSnapshot::Unavailable),
            cut: load_cut(meta).unwrap_or(LoadCut::Unavailable {
                reason: CutUnavailable::PublicationIncomplete,
            }),
        }
    }

    pub(crate) fn context(&self) -> CurrentActionsReadContext {
        match &self.snapshot {
            CurrentActionsSnapshot::Current(current)
                if current.binding == self.binding && self.snapshot.validate().is_ok() =>
            {
                CurrentActionsReadContext::Attach(CurrentActionsContext {
                    binding: self.binding.clone(),
                    incarnation: current.version.incarnation,
                })
            }
            CurrentActionsSnapshot::Current(_) | CurrentActionsSnapshot::Unavailable => {
                CurrentActionsReadContext::Unavailable(self.binding.clone())
            }
        }
    }
}

pub(crate) fn publication_result(
    agent_id: AgentId,
    binding: Option<xai_grok_shell::session::current_actions::SnapshotBinding>,
    meta: Option<&acp::Meta>,
    result: TaskResult,
) -> TaskResult {
    let Some(binding) = binding else {
        return result;
    };
    let LoadPublication {
        binding,
        snapshot,
        cut,
    } = LoadPublication::read(binding, meta);
    TaskResult::WithPublication {
        agent_id,
        binding,
        snapshot,
        cut,
        result: Box::new(result),
    }
}

pub(crate) async fn observe(
    context: &CurrentActionsContext,
    tx: &AcpAgentTx,
) -> CurrentActionsSnapshot {
    let Ok(params) = serde_json::value::to_raw_value(&context.binding) else {
        return CurrentActionsSnapshot::Unavailable;
    };
    let request = acp::ExtRequest::new(CAPABILITY, params.into());
    match acp_send(request, tx).await {
        Ok(response) => {
            serde_json::from_str(response.0.get()).unwrap_or(CurrentActionsSnapshot::Unavailable)
        }
        Err(_) => CurrentActionsSnapshot::Unavailable,
    }
}
