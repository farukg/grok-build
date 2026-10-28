//! Permissions, `ask_user_question`, and plan approval are **blocking ACP reverse-requests**.
//! The agent parks a tool-loop future on an in-memory oneshot and waits for the driver to answer.
//! While such a request is open we record it here, keyed by `tool_call_id` (stable, lives in the transcript, so it survives reconnect).
//! This registry is the single source of truth for "what is pending right now".
//! The roster reads it to report [`crate::agent::roster::RosterActivity::NeedsInput`].
//!
//! Pending interactions are **requests, not notifications**: they are never persisted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_client_protocol as acp;
use xai_acp_lib::AcpAgentGatewaySender as GatewaySender;

use crate::extensions::notification::{SessionNotification, SessionUpdate as XaiSessionUpdate};

/// Shared per-session map of open reverse-requests, keyed by `tool_call_id`.
/// Mirrors the `current_prompt_id` signal on [`crate::session::handle::SessionHandle`].
/// The same `Arc` is shared between the session actor (which mutates it) and the handle (which the roster reads synchronously).
pub(crate) type PendingInteractions = Arc<Mutex<HashMap<String, PendingKind>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    /// `request_permission` for a tool action.
    Permission,
    /// `x.ai/ask_user_question`.
    Question,
    /// `x.ai/exit_plan_mode` plan approval.
    PlanApproval,
    McpElicitation,
}

pub(crate) fn has_parked_plan_approval(pending: &PendingInteractions) -> bool {
    pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .any(|k| *k == PendingKind::PlanApproval)
}

/// Fire-and-forget broadcast of a session notification carrying a `sessionId` (so the routing layer fans it out to every subscriber).
/// Never persisted.
fn broadcast(
    gateway: &GatewaySender,
    session_id: &acp::SessionId,
    update: XaiSessionUpdate,
    version: Option<super::current_actions::SourceVersion>,
) {
    let notification = SessionNotification {
        session_id: session_id.clone(),
        update,
        meta: version
            .and_then(|version| serde_json::to_value(version).ok())
            .map(|version| {
                let mut meta = acp::Meta::new();
                meta.insert(super::current_actions::VERSION_META_KEY.into(), version);
                serde_json::Value::Object(meta)
            }),
    };
    if let Ok(params) = serde_json::value::to_raw_value(&notification) {
        gateway.forward_fire_and_forget(acp::ExtNotification::new(
            "x.ai/session_notification",
            params.into(),
        ));
    }
}

/// RAII guard registering an open reverse-request for the lifetime of the parked oneshot.
///
/// Drop runs whether the await returns normally, is cancelled, or errors.
pub(crate) struct PendingInteractionGuard {
    pending: PendingInteractions,
    gateway: GatewaySender,
    session_id: acp::SessionId,
    tool_call_id: String,
    observation: InteractionObservation,
}

enum InteractionObservation {
    Unavailable,
    Registered {
        source: super::current_actions::Source,
        registration_id: uuid::Uuid,
    },
}

impl PendingInteractionGuard {
    /// Register a pending interaction and broadcast `pending_interaction`.
    pub(crate) fn new(
        pending: PendingInteractions,
        gateway: GatewaySender,
        session_id: acp::SessionId,
        tool_call_id: String,
        kind: PendingKind,
        source: Option<(
            super::current_actions::Source,
            super::current_actions::FrozenOwner,
            xai_monotonic_clock::MonotonicPoint,
        )>,
    ) -> Self {
        let (observation, version) = match source {
            Some((source, owner, started_at)) => {
                let registration_id =
                    source.start_interaction(&pending, &tool_call_id, kind, owner, started_at);
                let version = source.version();
                (
                    InteractionObservation::Registered {
                        source,
                        registration_id,
                    },
                    version,
                )
            }
            None => {
                pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(tool_call_id.clone(), kind);
                (InteractionObservation::Unavailable, None)
            }
        };
        broadcast(
            &gateway,
            &session_id,
            XaiSessionUpdate::PendingInteraction {
                tool_call_id: tool_call_id.clone(),
                kind,
            },
            version,
        );
        Self {
            pending,
            gateway,
            session_id,
            tool_call_id,
            observation,
        }
    }
}

impl Drop for PendingInteractionGuard {
    fn drop(&mut self) {
        let (removed, version) = match &self.observation {
            InteractionObservation::Registered {
                source,
                registration_id,
            } => (
                source.finish_interaction(&self.pending, &self.tool_call_id, *registration_id),
                source.version(),
            ),
            InteractionObservation::Unavailable => (
                self.pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&self.tool_call_id)
                    .is_some(),
                None,
            ),
        };
        // First-answer-wins: only announce resolution if this guard actually owned the live entry
        if removed {
            broadcast(
                &self.gateway,
                &self.session_id,
                XaiSessionUpdate::InteractionResolved {
                    tool_call_id: self.tool_call_id.clone(),
                },
                version,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_registry() -> PendingInteractions {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[test]
    fn guard_inserts_then_removes() {
        let reg = new_registry();
        // No gateway round-trip is exercised here (broadcast is best-effort and a dead sender drops)
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let gateway = GatewaySender::new(tx);
        {
            let _g = PendingInteractionGuard::new(
                reg.clone(),
                gateway,
                acp::SessionId::new("sess-1"),
                "call-1".to_string(),
                PendingKind::Permission,
                None,
            );
            assert_eq!(reg.lock().unwrap().len(), 1);
            assert_eq!(
                reg.lock().unwrap().get("call-1").copied(),
                Some(PendingKind::Permission)
            );
        }
        assert!(reg.lock().unwrap().is_empty());
    }

    #[test]
    fn has_parked_plan_approval_only_counts_plan_approval() {
        let reg = new_registry();
        assert!(!has_parked_plan_approval(&reg));

        reg.lock()
            .unwrap()
            .insert("perm".to_string(), PendingKind::Permission);
        reg.lock()
            .unwrap()
            .insert("q".to_string(), PendingKind::Question);
        assert!(
            !has_parked_plan_approval(&reg),
            "permission/question parks must not count as a parked approval"
        );

        reg.lock()
            .unwrap()
            .insert("plan".to_string(), PendingKind::PlanApproval);
        assert!(has_parked_plan_approval(&reg));

        reg.lock().unwrap().remove("plan");
        assert!(!has_parked_plan_approval(&reg));
    }

    #[test]
    fn has_parked_plan_approval_recovers_poisoned_lock() {
        let reg = new_registry();
        reg.lock()
            .unwrap()
            .insert("plan".to_string(), PendingKind::PlanApproval);

        let reg_poison = reg.clone();
        let _ = std::thread::spawn(move || {
            let _g = reg_poison.lock().unwrap();
            panic!("poison pending_interactions");
        })
        .join();
        assert!(
            reg.lock().is_err(),
            "precondition: the lock must be poisoned"
        );

        assert!(
            has_parked_plan_approval(&reg),
            "a poisoned lock must still surface the parked approval, not panic"
        );
    }
}
