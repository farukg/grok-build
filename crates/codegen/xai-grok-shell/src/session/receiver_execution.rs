use std::fs::File;
use std::io;

pub struct ReceiverExecutionLease {
    pub(crate) _file: File,
    pub(crate) session_id: String,
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
    Running {
        turn_id: String,
        parent_task: xai_chat_state::ParentTask,
        lease: ReceiverExecutionLease,
    },
}
impl ReceiverOwner {
    pub(crate) fn access(&self) -> ReceiverAccess<'_> {
        match self {
            Self::Unclaimed => ReceiverAccess::Observe,
            Self::Ready(lease) | Self::Running { lease, .. } => ReceiverAccess::Owner(lease),
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
            owner @ Self::Running { .. } => {
                let matches = matches!(&owner, Self::Running { turn_id: current, parent_task: task, .. } if current == turn_id && task == parent_task);
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
            owner @ (Self::Unclaimed | Self::Ready(_) | Self::Running { .. }) => *self = owner,
        }
    }
}
