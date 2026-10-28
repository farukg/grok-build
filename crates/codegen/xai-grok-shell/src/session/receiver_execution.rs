use std::fs::File;
use std::io;

#[path = "parent_message_attempt.rs"]
mod parent_message_attempt;
pub(crate) use parent_message_attempt::{ParentMessageAdoption, ParentMessagePublicationContract};

pub use xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership;

pub(crate) enum ParentAdoptionTurn {
    Unreserved(ParentMessageAdoption),
    Reserved {
        turn_id: String,
        parent_task: xai_chat_state::ParentTask,
        publication: ParentMessageAdoption,
    },
}

impl ParentAdoptionTurn {
    fn publication(&self) -> &ParentMessageAdoption {
        match self {
            Self::Unreserved(publication) | Self::Reserved { publication, .. } => publication,
        }
    }
}

pub struct ReceiverExecutionLease {
    pub(crate) _file: std::sync::Arc<File>,
    pub(crate) session_id: String,
}

impl ReceiverExecutionLease {
    pub(crate) fn retain(&self) -> Self {
        Self { _file: self._file.clone(), session_id: self.session_id.clone() }
    }
}

pub enum ReceiverAccess<'a> {
    Observe,
    Owner(&'a ReceiverExecutionLease),
}

#[derive(Clone, Copy)]
pub(crate) enum ReceiverPresence {
    Live,
    Detached,
}

#[derive(Default)]
pub(crate) enum ReceiverOwner {
    #[default]
    Unclaimed,
    Ready(ReceiverExecutionLease),
    Adopting {
        message_id: String,
        turn: ParentAdoptionTurn,
        lease: ReceiverExecutionLease,
    },
    Running {
        turn_id: String,
        parent_task: xai_chat_state::ParentTask,
        lease: ReceiverExecutionLease,
    },
}
impl ReceiverOwner {
    pub(crate) fn begin_adoption(&mut self, message_id: String) -> io::Result<()> {
        match std::mem::take(self) {
            Self::Ready(lease) => {
                *self = Self::Adopting { message_id, turn: ParentAdoptionTurn::Unreserved(ParentMessageAdoption::new()), lease };
                Ok(())
            }
            owner @ (Self::Unclaimed | Self::Running { .. } | Self::Adopting { .. }) => {
                *self = owner;
                Err(io::Error::new(io::ErrorKind::WouldBlock, "receiver already has an execution attempt"))
            }
        }
    }

    pub(crate) fn publication_contract(&self) -> ParentMessagePublicationContract {
        match self {
            Self::Adopting { turn, .. } => ParentMessagePublicationContract::Adoption(turn.publication().clone()),
            Self::Unclaimed | Self::Ready(_) | Self::Running { .. } => ParentMessagePublicationContract::Admission,
        }
    }

    pub(crate) fn retain_parent_message_at_end(&self, message_id: &str) -> bool {
        match self {
            Self::Adopting { message_id: owned, turn, .. } => owned == message_id && matches!(turn.publication().finish_attempt(), InitialMessageTerminalOwnership::AdoptionAttempt),
            Self::Unclaimed | Self::Ready(_) | Self::Running { .. } => false,
        }
    }

    pub(crate) fn terminal_decision(&self, turn_id: &str, parent_task: &xai_chat_state::ParentTask, origin: &super::delivery_receiver::ReceiverTurnOrigin) -> InitialMessageTerminalOwnership {
        if self.retain_turn_at_end(turn_id, parent_task, origin) {
            InitialMessageTerminalOwnership::AdoptionAttempt
        } else {
            InitialMessageTerminalOwnership::Message
        }
    }

    pub(crate) fn retain_turn_at_end(&self, turn_id: &str, parent_task: &xai_chat_state::ParentTask, origin: &super::delivery_receiver::ReceiverTurnOrigin) -> bool {
        match (self, origin) {
            (Self::Adopting { message_id, turn, .. }, super::delivery_receiver::ReceiverTurnOrigin::ParentMessage { message_id: requested }) if message_id == requested => match turn {
                ParentAdoptionTurn::Unreserved(publication) => matches!(publication.finish_attempt(), InitialMessageTerminalOwnership::AdoptionAttempt),
                ParentAdoptionTurn::Reserved { turn_id: owned, parent_task: task, publication } => owned != turn_id || task != parent_task || matches!(publication.finish_attempt(), InitialMessageTerminalOwnership::AdoptionAttempt),
            },
            (Self::Unclaimed | Self::Ready(_) | Self::Running { .. } | Self::Adopting { .. }, super::delivery_receiver::ReceiverTurnOrigin::Other | super::delivery_receiver::ReceiverTurnOrigin::Delivery { .. } | super::delivery_receiver::ReceiverTurnOrigin::ParentMessage { .. }) => false,
        }
    }

    pub(crate) fn bind_parent_message(&mut self, message_id: &str, turn_id: &str, parent_task: &xai_chat_state::ParentTask) -> io::Result<()> {
        if let Self::Adopting { message_id: owned, .. } = self
            && owned != message_id
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "adoption reservation names another message"));
        }
        self.bind_turn(turn_id, parent_task)
    }

    pub(crate) fn bind_projection(&mut self, turn_id: &str, parent_task: &xai_chat_state::ParentTask) -> io::Result<()> {
        if let Self::Adopting { turn, .. } = self
            && !turn.publication().can_project()
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "adoption publication has not authorized projection"));
        }
        self.bind_turn(turn_id, parent_task)
    }

    pub(crate) fn observe_projection(
        &self,
        binding: &xai_chat_state::DeliveryBinding,
        result: &Result<xai_chat_state::CompletionProjectionAck, xai_chat_state::CompletionProjectionError>,
    ) {
        if let Self::Adopting { message_id, turn: ParentAdoptionTurn::Reserved { turn_id, parent_task, publication }, .. } = self
            && binding.delivery_id == super::delivery_receiver::parent_message_delivery_id(message_id)
            && &binding.turn_id == turn_id && &binding.parent_task == parent_task
        {
            match result {
                Ok(acknowledgement) | Err(xai_chat_state::StrictAppendError::Committed { acknowledgement, .. }) => match acknowledgement {
                    xai_chat_state::CompletionProjectionAck::Appended | xai_chat_state::CompletionProjectionAck::AlreadyPresent { .. } => publication.projected(),
                    xai_chat_state::CompletionProjectionAck::Continue { .. } | xai_chat_state::CompletionProjectionAck::TurnClosed => {}
                },
                Err(xai_chat_state::StrictAppendError::NotCommitted(_) | xai_chat_state::StrictAppendError::Indeterminate(_)) => {}
            }
        }
    }

    pub(crate) fn access(&self) -> ReceiverAccess<'_> {
        match self {
            Self::Unclaimed => ReceiverAccess::Observe,
            Self::Ready(lease) | Self::Running { lease, .. } | Self::Adopting { lease, .. } => ReceiverAccess::Owner(lease),
        }
    }
    pub(crate) fn bind_turn(
        &mut self,
        turn_id: &str,
        parent_task: &xai_chat_state::ParentTask,
    ) -> io::Result<()> {
        match std::mem::take(self) {
            Self::Ready(lease) => {
                *self = Self::Running {
                    turn_id: turn_id.to_owned(),
                    parent_task: parent_task.clone(),
                    lease,
                };
                Ok(())
            }
            Self::Adopting { message_id, turn: ParentAdoptionTurn::Unreserved(publication), lease } => {
                *self = Self::Adopting {
                    message_id,
                    turn: ParentAdoptionTurn::Reserved {
                        turn_id: turn_id.to_owned(), parent_task: parent_task.clone(), publication,
                    },
                    lease,
                };
                Ok(())
            }
            owner @ (Self::Running { .. } | Self::Adopting { turn: ParentAdoptionTurn::Reserved { .. }, .. }) => {
                let matches = match &owner {
                    Self::Running { turn_id: current, parent_task: task, .. }
                    | Self::Adopting { turn: ParentAdoptionTurn::Reserved { turn_id: current, parent_task: task, .. }, .. } => current == turn_id && task == parent_task,
                    Self::Unclaimed | Self::Ready(_) | Self::Adopting { turn: ParentAdoptionTurn::Unreserved(_), .. } => false,
                };
                *self = owner;
                if matches {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "another receiver turn still owns execution",
                    ))
                }
            }
            Self::Unclaimed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "receiver execution is unclaimed",
            )),
        }
    }
    pub(crate) fn finish_turn(&mut self, turn_id: &str, parent_task: &xai_chat_state::ParentTask) {
        match std::mem::take(self) {
            Self::Running {
                turn_id: current,
                parent_task: task,
                lease,
            } if current == turn_id && &task == parent_task => *self = Self::Ready(lease),
            Self::Adopting { turn: ParentAdoptionTurn::Reserved { turn_id: current, parent_task: task, publication }, lease, message_id: _ }
                if current == turn_id && &task == parent_task && !publication.retains_admission() => *self = Self::Ready(lease),
            owner @ (Self::Unclaimed | Self::Ready(_) | Self::Running { .. } | Self::Adopting { .. }) => *self = owner,
        }
    }
}
