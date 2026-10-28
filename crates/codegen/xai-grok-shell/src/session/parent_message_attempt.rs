use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Debug)]
enum AdoptionPublication {
    Reserved,
    Released,
    Projected,
    CancelAuthorized,
    Abandoned,
}

#[derive(Clone, Debug)]
pub(crate) struct ParentMessageAdoption(Arc<Mutex<AdoptionPublication>>);

impl ParentMessageAdoption {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(AdoptionPublication::Reserved)))
    }

    pub(crate) fn publish(&self, release: oneshot::Sender<()>) -> Result<(), ()> {
        let mut state = self.0.lock();
        match &*state {
            AdoptionPublication::Reserved => {
                if release.send(()).is_ok() {
                    *state = AdoptionPublication::Released;
                    Ok(())
                } else {
                    *state = AdoptionPublication::Abandoned;
                    Err(())
                }
            }
            AdoptionPublication::Released
            | AdoptionPublication::Projected
            | AdoptionPublication::CancelAuthorized
            | AdoptionPublication::Abandoned => Err(()),
        }
    }

    pub(crate) fn abandon(&self) {
        let mut state = self.0.lock();
        match &*state {
            AdoptionPublication::Reserved => *state = AdoptionPublication::Abandoned,
            AdoptionPublication::Released
            | AdoptionPublication::Projected
            | AdoptionPublication::CancelAuthorized
            | AdoptionPublication::Abandoned => {}
        }
    }

    pub(crate) fn authorize_cancel(
        &self,
    ) -> xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome {
        use xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome;
        let mut state = self.0.lock();
        match &*state {
            AdoptionPublication::Reserved | AdoptionPublication::Released => {
                *state = AdoptionPublication::CancelAuthorized;
                ChildCancelOutcome::Accepted
            }
            AdoptionPublication::Projected | AdoptionPublication::CancelAuthorized => {
                ChildCancelOutcome::Accepted
            }
            AdoptionPublication::Abandoned => ChildCancelOutcome::AttemptEnded,
        }
    }

    pub(crate) fn can_project(&self) -> bool {
        match &*self.0.lock() {
            AdoptionPublication::Released | AdoptionPublication::Projected => true,
            AdoptionPublication::Reserved
            | AdoptionPublication::CancelAuthorized
            | AdoptionPublication::Abandoned => false,
        }
    }

    pub(crate) fn projected(&self) {
        let mut state = self.0.lock();
        match &*state {
            AdoptionPublication::Released => *state = AdoptionPublication::Projected,
            AdoptionPublication::Reserved
            | AdoptionPublication::Projected
            | AdoptionPublication::CancelAuthorized
            | AdoptionPublication::Abandoned => {}
        }
    }

    // Retention retires cancellation authority before the terminal owner leaves.
    pub(crate) fn finish_attempt(&self) -> xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership{
        use xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership;
        let mut state = self.0.lock();
        match &*state {
            AdoptionPublication::Reserved | AdoptionPublication::Released => {
                *state = AdoptionPublication::Abandoned;
                InitialMessageTerminalOwnership::AdoptionAttempt
            }
            AdoptionPublication::Abandoned => InitialMessageTerminalOwnership::AdoptionAttempt,
            AdoptionPublication::Projected | AdoptionPublication::CancelAuthorized => {
                InitialMessageTerminalOwnership::Message
            }
        }
    }

    pub(crate) fn retains_admission(&self) -> bool {
        match &*self.0.lock() {
            AdoptionPublication::Reserved
            | AdoptionPublication::Released
            | AdoptionPublication::Abandoned => true,
            AdoptionPublication::Projected | AdoptionPublication::CancelAuthorized => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ParentMessagePublicationContract {
    Admission,
    Adoption(ParentMessageAdoption),
}

impl ParentMessagePublicationContract {
    pub(crate) fn terminal_ownership(&self) -> xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership{
        use xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership;
        match self {
            Self::Admission => InitialMessageTerminalOwnership::Message,
            Self::Adoption(adoption) if adoption.retains_admission() => {
                InitialMessageTerminalOwnership::AdoptionAttempt
            }
            Self::Adoption(_) => InitialMessageTerminalOwnership::Message,
        }
    }

    pub(crate) fn finish_attempt(&self) -> xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership{
        use xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership;
        match self {
            Self::Admission => InitialMessageTerminalOwnership::Message,
            Self::Adoption(adoption) => adoption.finish_attempt(),
        }
    }

    pub(crate) fn publish(&self, release: oneshot::Sender<()>) -> Result<(), ()> {
        match self {
            Self::Admission => release.send(()),
            Self::Adoption(adoption) => adoption.publish(release),
        }
    }
}
