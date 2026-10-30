use super::*;
use super::agent_quotas::TargetIncarnationKey;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct ChildSubmissionAuthority {
    parent: String,
    child: String,
    incarnation: TargetIncarnationKey,
    prompt: String,
    expires_at: tokio::time::Instant,
    revocation: CancellationToken,
    claim: Arc<parking_lot::Mutex<SubmissionClaim>>,
}

#[derive(Debug)]
enum SubmissionClaim {
    Available,
    Consumed,
}

impl PartialEq for ChildSubmissionAuthority {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.claim, &other.claim)
    }
}
impl Eq for ChildSubmissionAuthority {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChildSubmission {
    #[default]
    Ordinary,
    New(ChildSubmissionAuthority),
    Admitted(ChildSubmissionLease),
}

#[derive(Clone, Debug)]
pub struct ChildSubmissionLease {
    parent: String,
    child: String,
    revocation: CancellationToken,
    claim: Arc<parking_lot::Mutex<SubmissionClaim>>,
}

impl PartialEq for ChildSubmissionLease {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.claim, &other.claim)
    }
}
impl Eq for ChildSubmissionLease {}

impl ChildSubmission {
    pub fn permits(&self, parent: &str, child: &str) -> bool {
        match self {
            Self::Ordinary | Self::New(_) => false,
            Self::Admitted(lease) => lease.parent == parent && lease.child == child
                && !lease.revocation.is_cancelled(),
        }
    }

    pub(crate) fn cancellation(&self) -> Option<CancellationToken> {
        match self {
            Self::Ordinary | Self::New(_) => None,
            Self::Admitted(lease) => Some(lease.revocation.child_token()),
        }
    }

    pub fn is_revoked(&self) -> bool {
        match self {
            Self::Ordinary => false,
            Self::New(authority) => authority.revocation.is_cancelled(),
            Self::Admitted(lease) => lease.revocation.is_cancelled(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum SpawnBlock {
    Stopped,
    StaleRestart,
}

impl SpawnBlock {
    pub(super) fn blocks_child_submission(self) -> bool {
        match self {
            Self::Stopped => true,
            Self::StaleRestart => false,
        }
    }
}

#[derive(Default)]
pub(super) struct ChildSubmissions {
    scopes: HashMap<String, CancellationToken>,
    children: HashMap<(String, String), CancellationToken>,
}

impl<R: ChildRunner> SubagentCoordinator<R> {
    pub(super) fn authorize_child_message(
        &mut self,
        request: &mut super::super::active_message::SubagentActiveMessageRequest,
    ) -> Result<(), ActiveAgentMessageOutcome> {
        use super::super::active_message::{ActiveMessageSenderContext, ActiveMessageTarget};
        let child = match &request.request.child_submission {
            ChildSubmission::Ordinary => return Ok(()),
            ChildSubmission::Admitted(_) => return Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing),
            ChildSubmission::New(authority) => authority.child.clone(),
        };
        let parent = match &request.sender_context {
            ActiveMessageSenderContext::HumanRoot { session_id } => session_id.to_string(),
            ActiveMessageSenderContext::RootSession { .. } | ActiveMessageSenderContext::GrantedChild { .. } => {
                return Err(ActiveAgentMessageOutcome::NotFoundOrNotOwned);
            }
        };
        let address = self.active.get(&child).and_then(|c| c.agent_address.as_ref())
            .or_else(|| self.pending.get(&child).and_then(|c| c.agent_address.as_ref()))
            .or_else(|| self.completed.get(&child).and_then(|c| c.agent_address.as_ref()))
            .or_else(|| self.queued.iter().find(|c| c.request.id == child).and_then(|c| c.agent_address.as_ref()));
        let exact = match request.request.target() {
            ActiveMessageTarget::ChildId(id) => id == &child,
            ActiveMessageTarget::Address(target) => address == Some(target),
            ActiveMessageTarget::Parent | ActiveMessageTarget::Agent { .. } => false,
        };
        if !exact { return Err(ActiveAgentMessageOutcome::NotFoundOrNotOwned); }
        request.request.child_submission = self.claim_child_submission(&request.request.child_submission, &parent, &child, request.request.text())?;
        Ok(())
    }

    pub(super) fn reserve_child_submission(
        &mut self,
        parent: String,
        child: String,
        prompt: String,
    ) -> Result<ChildSubmissionAuthority, ActiveAgentMessageOutcome> {
        if self.teardown_drains.contains_key(&parent)
            || self.spawn_blocked_sessions.get(&parent).is_some_and(|block| block.blocks_child_submission())
        {
            return Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing);
        }
        let scope = self.child_submissions.scopes.entry(parent.clone()).or_default();
        let revocation = self.child_submissions.children.entry((parent.clone(), child.clone()))
            .or_insert_with(|| scope.child_token()).clone();
        Ok(ChildSubmissionAuthority {
            parent,
            incarnation: self.current_target_incarnation(&child),
            child,
            prompt,
            expires_at: tokio::time::Instant::now() + std::time::Duration::from_secs(60),
            revocation,
            claim: Arc::new(parking_lot::Mutex::new(SubmissionClaim::Available)),
        })
    }

    pub(super) fn claim_child_submission(
        &mut self,
        submission: &ChildSubmission,
        parent: &str,
        child: &str,
        prompt: &str,
    ) -> Result<ChildSubmission, ActiveAgentMessageOutcome> {
        match submission {
            ChildSubmission::Ordinary => Ok(ChildSubmission::Ordinary),
            ChildSubmission::Admitted(_) => Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing),
            ChildSubmission::New(authority) => {
                if authority.parent != parent || authority.child != child || authority.prompt != prompt
                    || authority.revocation.is_cancelled()
                    || tokio::time::Instant::now() >= authority.expires_at
                    || authority.incarnation != self.current_target_incarnation(child)
                    || self.teardown_drains.contains_key(parent)
                    || self.spawn_blocked_sessions.get(parent).is_some_and(|block| block.blocks_child_submission())
                {
                    return Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing);
                }
                let mut claim = authority.claim.lock();
                match *claim {
                    SubmissionClaim::Consumed => Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing),
                    SubmissionClaim::Available => {
                        *claim = SubmissionClaim::Consumed;
                        Ok(ChildSubmission::Admitted(ChildSubmissionLease {
                            parent: parent.to_owned(), child: child.to_owned(),
                            revocation: authority.revocation.clone(), claim: authority.claim.clone(),
                        }))
                    }
                }
            }
        }
    }

    pub(super) fn revoke_unregistered_submission(&mut self, parent: &str, child: &str) {
        if let Some(token) = self.child_submissions.children.remove(&(parent.to_owned(), child.to_owned())) {
            token.cancel();
        }
    }

    pub(super) fn revoke_child_submissions(&mut self, child: &str) {
        self.child_submissions.children.retain(|(_, id), token| {
            if id == child { token.cancel(); return false; }
            true
        });
    }

    pub(super) fn revoke_parent_submissions(&mut self, parent: &str) {
        if let Some(block) = self.spawn_blocked_sessions.get_mut(parent) { *block = SpawnBlock::Stopped; }
        if let Some(scope) = self.child_submissions.scopes.remove(parent) { scope.cancel(); }
        self.child_submissions.children.retain(|(owner, _), _| owner != parent);
    }

    pub(super) fn quiesce_restart_children(&mut self, parent: &str) -> SubagentCancelOutcome {
        match self.spawn_blocked_sessions.get(parent).copied() {
            Some(SpawnBlock::StaleRestart) => SubagentCancelOutcome::Cancelled {
                cause: InterruptionCause::SessionStopped { session_id: parent.to_owned() },
            },
            Some(SpawnBlock::Stopped) => self.cancel_parent_session(Some(parent), ChildCancellation::Authorized),
            None => {
                let outcome = self.cancel_parent_session(Some(parent), ChildCancellation::Authorized);
                self.spawn_blocked_sessions.insert(parent.to_owned(), SpawnBlock::StaleRestart);
                outcome
            }
        }
    }
}
