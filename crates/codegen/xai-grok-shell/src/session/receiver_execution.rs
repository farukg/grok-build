use std::fs::File;
use std::io;

pub struct ReceiverExecutionLease {
    pub(crate) _file: File,
    pub(crate) session_id: String,
}

pub enum ReceiverAccess<'a> { Observe, Owner(&'a ReceiverExecutionLease) }

#[derive(Clone, Copy)]
pub(crate) enum ReceiverPresence { Live, Detached }

#[derive(Default)]
pub(crate) enum ReceiverOwner {
    #[default]
    Unclaimed,
    Ready(ReceiverExecutionLease),
    Running { turn_id: String, lease: ReceiverExecutionLease },
}
impl ReceiverOwner {
    pub(crate) fn access(&self) -> ReceiverAccess<'_> {
        match self {
            Self::Unclaimed => ReceiverAccess::Observe,
            Self::Ready(lease) | Self::Running { lease, .. } => ReceiverAccess::Owner(lease),
        }
    }
    pub(crate) fn bind_turn(&mut self, turn_id: &str) -> io::Result<()> {
        match std::mem::take(self) {
            Self::Ready(lease) => { *self = Self::Running { turn_id: turn_id.to_owned(), lease }; Ok(()) }
            owner @ Self::Running { .. } => {
                let matches = matches!(&owner, Self::Running { turn_id: current, .. } if current == turn_id);
                *self = owner;
                if matches { Ok(()) } else { Err(io::Error::new(io::ErrorKind::WouldBlock, "another receiver turn still owns execution")) }
            }
            Self::Unclaimed => Err(io::Error::new(io::ErrorKind::NotConnected, "receiver execution is unclaimed")),
        }
    }
    pub(crate) fn finish_turn(&mut self, turn_id: &str) {
        match std::mem::take(self) {
            Self::Running { turn_id: current, lease } if current == turn_id => *self = Self::Ready(lease),
            owner @ (Self::Unclaimed | Self::Ready(_) | Self::Running { .. }) => *self = owner,
        }
    }
}
