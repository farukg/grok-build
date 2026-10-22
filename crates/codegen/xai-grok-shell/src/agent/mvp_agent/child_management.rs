//! Bounded, prompt-free access to a saved child actor.

use std::time::Duration;

use agent_client_protocol as acp;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{ChildReach, ChildResidence, MvpAgent, SessionHost};
use crate::session::compaction_config::CompactCancelGate;
use crate::session::helpers::session_compact::{
    CompactErrorKind, CompactFailure, compact_error_data,
};
use crate::session::{SessionCommand, SessionHandle, ShutdownKind};

#[derive(Default)]
pub(crate) struct ChildManagement {
    pub(super) lock: std::sync::Arc<Mutex<()>>,
    pub(crate) compact: std::sync::Arc<Mutex<()>>,
    pub(crate) cancel: CompactCancelGate,
}

pub(crate) enum ManagementResolution {
    Active(SessionHandle),
    Restorable(Box<crate::session::persistence::Summary>),
    Unknown,
    ChannelClosed,
}

pub(crate) struct ManagementSession<'a> {
    pub(crate) handle: SessionHandle,
    agent: &'a MvpAgent,
    temporary: Option<OwnedMutexGuard<()>>,
}

impl Drop for ManagementSession<'_> {
    fn drop(&mut self) {
        if self.temporary.is_some() {
            let _ = self
                .handle
                .cmd_tx
                .send(SessionCommand::Shutdown(ShutdownKind::Graceful));
            self.agent
                .session_registry
                .take_resident(&self.handle.info.id);
            self.handle
                .workspace_ops
                .end_local_session(self.handle.info.id.0.as_ref());
        }
    }
}

pub(crate) fn failed(message: &str) -> acp::Error {
    acp::Error::internal_error().data(compact_error_data(CompactErrorKind::Failed, message))
}

impl MvpAgent {
    pub(crate) async fn management_resolution(
        &self,
        id: &acp::SessionId,
    ) -> Result<ManagementResolution, acp::Error> {
        if let Some(handle) = self.resident_handle(id) {
            return Ok(if handle.cmd_tx.is_closed() {
                ManagementResolution::ChannelClosed
            } else {
                ManagementResolution::Active(handle)
            });
        }
        match self.session_host(id) {
            Some(SessionHost::Child(child)) => match child.reach {
                ChildReach::Addressed {
                    residence: ChildResidence::Finished,
                    ..
                } => {
                    let cwd = self
                        .get_session_cwd(&child.parent_session_id)
                        .ok_or_else(|| failed("the saved child's parent is unavailable"))?;
                    let source = crate::agent::subagent::durable_resume_source(
                        id.0.as_ref(),
                        child.parent_session_id.0.as_ref(),
                        &cwd,
                    )
                    .map_err(|error| failed(&error.to_string()))?;
                    let info = crate::session::info::Info {
                        id: id.clone(),
                        cwd: source.child_cwd,
                    };
                    use crate::session::storage::StorageAdapter as _;
                    let storage = crate::session::storage::JsonlStorageAdapter::with_root(
                        crate::util::grok_home::grok_home(),
                    );
                    let summary = storage
                        .load_summary(&info)
                        .await
                        .map_err(|error| failed(&error.to_string()))?;
                    if summary.info.id != *id {
                        return Err(failed("saved child identity does not match its address"));
                    }
                    Ok(ManagementResolution::Restorable(Box::new(summary)))
                }
                ChildReach::Addressed {
                    residence: ChildResidence::Running(_),
                    ..
                }
                | ChildReach::Unaddressed => Ok(ManagementResolution::ChannelClosed),
            },
            Some(SessionHost::Root) | None => Ok(ManagementResolution::Unknown),
        }
    }

    pub(crate) fn compact_operation(&self, id: &acp::SessionId) -> std::rc::Rc<ChildManagement> {
        self.session_registry.begin_management(id)
    }

    pub(crate) fn cancel_management(&self, id: &acp::SessionId) {
        if let Some(operation) = self.session_registry.management(id) {
            operation.cancel.request_cancel();
        }
    }

    pub(crate) fn child_writer_is_active(&self, id: &acp::SessionId) -> bool {
        self.session_registry.thread_is_finished(id) == Some(false)
    }

    pub(crate) async fn claim_child_management(&self, id: &acp::SessionId) -> OwnedMutexGuard<()> {
        self.session_registry
            .begin_management(id)
            .lock
            .clone()
            .lock_owned()
            .await
    }

    pub(crate) async fn wait_for_child_management(&self, id: &acp::SessionId) {
        if let Some(operation) = self.session_registry.management(id) {
            let _guard = operation.lock.lock().await;
        }
    }

    pub(crate) async fn management_session(
        &self,
        id: &acp::SessionId,
    ) -> Result<ManagementSession<'_>, acp::Error> {
        let operation = self.session_registry.begin_management(id);
        let (cancel, _scope) = operation.cancel.enter();
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(CompactFailure::cancelled_error()),
            result = tokio::time::timeout(Duration::from_secs(60), self.prepare_management_session(id)) => {
                result.map_err(|_| failed("saved child preparation exceeded 60 seconds"))?
            }
        }
    }

    async fn prepare_management_session(
        &self,
        id: &acp::SessionId,
    ) -> Result<ManagementSession<'_>, acp::Error> {
        let operation = self.session_registry.begin_management(id);
        let guard = operation.lock.clone().lock_owned().await;
        match self.management_resolution(id).await? {
            ManagementResolution::Active(handle) => Ok(ManagementSession {
                handle,
                agent: self,
                temporary: None,
            }),
            ManagementResolution::Unknown => Err(failed("session not found")),
            ManagementResolution::ChannelClosed => Err(failed("session command channel is closed")),
            ManagementResolution::Restorable(summary) => {
                let mut cleanup = PreparationCleanup {
                    agent: self,
                    id: id.clone(),
                    armed: true,
                };
                self.drain_old_session_thread(id).await;
                if self.session_registry.thread_is_finished(id) == Some(false) {
                    return Err(failed("the previous child actor still owns persistence"));
                }
                self.load_child_for_management(&summary.info).await?;
                let handle = self
                    .resident_handle(id)
                    .ok_or_else(|| failed("saved child did not install an actor"))?;
                if !matches!(self.session_host(id), Some(SessionHost::Child(_))) {
                    return Err(failed("saved child was released during preparation"));
                }
                cleanup.armed = false;
                Ok(ManagementSession {
                    handle,
                    agent: self,
                    temporary: Some(guard),
                })
            }
        }
    }
}

struct PreparationCleanup<'a> {
    agent: &'a MvpAgent,
    id: acp::SessionId,
    armed: bool,
}

impl Drop for PreparationCleanup<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Some(handle) = self.agent.session_registry.take_resident(&self.id)
        {
            let _ = handle
                .cmd_tx
                .send(SessionCommand::Shutdown(ShutdownKind::Graceful));
            handle.workspace_ops.end_local_session(self.id.0.as_ref());
        }
    }
}
