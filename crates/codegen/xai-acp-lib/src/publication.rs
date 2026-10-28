use std::sync::{Arc, Mutex};

use futures::{FutureExt, StreamExt};
use futures::future::{BoxFuture, Shared};
use futures::stream::FuturesUnordered;
use tokio::sync::oneshot;

use crate::AcpResult;

type Receipt = Shared<BoxFuture<'static, Result<(), PublicationFailure>>>;

pub type PublicationCompletion<T> = BoxFuture<'static, Result<AcpResult<T>, oneshot::error::RecvError>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationFailure {
    ProducerLost,
    ReceiverFailed,
    IncompletePublication,
}

#[derive(Debug)]
pub struct ReceiverSettlementSelectionError;

impl std::fmt::Display for ReceiverSettlementSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("receiver settlement must be selected before sharing or using the gateway sender")
    }
}

impl std::error::Error for ReceiverSettlementSelectionError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReceiverSettlement {
    #[default]
    TransportOnly,
    AppliedOrDiscarded,
}

struct Receipts {
    pending: FuturesUnordered<Receipt>,
    outcome: Result<(), PublicationFailure>,
}

impl Default for Receipts {
    fn default() -> Self {
        Self { pending: FuturesUnordered::new(), outcome: Ok(()) }
    }
}

impl Receipts {
    fn push(&mut self, receipt: Receipt) {
        self.reap();
        self.pending.push(receipt);
    }

    fn reap(&mut self) {
        while let Some(Some(settled)) = self.pending.next().now_or_never() {
            self.outcome = self.outcome.and(settled);
        }
    }

    fn prefix(&mut self) -> PublicationPrefix {
        self.reap();
        PublicationPrefix { pending: self.pending.iter().cloned().collect(), outcome: self.outcome }
    }
}

#[derive(Default)]
enum GatewayUse {
    #[default]
    Unused,
    Started,
}

#[derive(Default)]
struct PublicationState {
    producers: Receipts,
    deliveries: Receipts,
    gateway_use: GatewayUse,
}

#[derive(Clone, Default)]
pub(crate) struct Publications(Arc<Mutex<PublicationState>>);

impl Publications {
    pub(crate) fn can_select_receiver(&self) -> bool {
        Arc::strong_count(&self.0) == 1
            && matches!(self.0.lock().unwrap_or_else(|error| error.into_inner()).gateway_use, GatewayUse::Unused)
    }

    pub(crate) fn start(&self) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).gateway_use = GatewayUse::Started;
    }

    pub(crate) fn begin(&self) -> PublicationGuard {
        let (tx, rx) = oneshot::channel();
        self.0.lock().unwrap_or_else(|error| error.into_inner()).producers
            .push(async move { rx.await.map_err(|_| PublicationFailure::ProducerLost) }.boxed().shared());
        PublicationGuard { settled: Some(tx), custody: PublicationCustody::Preparing }
    }

    pub(crate) fn track<T: Send + 'static>(&self, rx: oneshot::Receiver<AcpResult<T>>) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).deliveries
            .push(async move {
                match rx.await {
                    Ok(Ok(_)) => Ok(()),
                    Ok(Err(_)) | Err(_) => Err(PublicationFailure::ReceiverFailed),
                }
            }.boxed().shared());
    }

    pub(crate) fn completion<T: Send + 'static>(&self, rx: oneshot::Receiver<AcpResult<T>>) -> PublicationCompletion<T> {
        let (tx, response) = oneshot::channel();
        let receipt = async move {
            let result = rx.await;
            let outcome = match &result {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(_)) | Err(_) => Err(PublicationFailure::ReceiverFailed),
            };
            let _ = tx.send(result);
            outcome
        }.boxed().shared();
        self.0.lock().unwrap_or_else(|error| error.into_inner()).deliveries.push(receipt.clone());
        async move {
            let _ = receipt.await;
            response.await?
        }.boxed()
    }

    pub(crate) fn fail(&self) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).deliveries.outcome = Err(PublicationFailure::IncompletePublication);
    }

    pub(crate) fn producers(&self) -> PublicationPrefix {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).producers.prefix()
    }

    pub(crate) fn deliveries(&self) -> PublicationPrefix {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).deliveries.prefix()
    }
}

enum PublicationCustody {
    Preparing,
    Owed,
    Enqueued,
    Rejected,
}

pub struct PublicationGuard {
    settled: Option<oneshot::Sender<()>>,
    custody: PublicationCustody,
}

impl PublicationGuard {
    pub(crate) fn untracked() -> Self {
        Self { settled: None, custody: PublicationCustody::Preparing }
    }

    pub fn require_delivery(&mut self) {
        self.custody = PublicationCustody::Owed;
    }

    pub fn enqueued(mut self) {
        self.custody = PublicationCustody::Enqueued;
    }

    pub fn rejected(mut self) {
        self.custody = PublicationCustody::Rejected;
    }
}

impl Drop for PublicationGuard {
    fn drop(&mut self) {
        if let Some(settled) = self.settled.take() {
            match self.custody {
                PublicationCustody::Preparing | PublicationCustody::Enqueued | PublicationCustody::Rejected if !std::thread::panicking() => {
                    let _ = settled.send(());
                }
                PublicationCustody::Preparing | PublicationCustody::Enqueued | PublicationCustody::Rejected | PublicationCustody::Owed => {}
            }
        }
    }
}

pub struct PublicationPrefix {
    pending: Vec<Receipt>,
    outcome: Result<(), PublicationFailure>,
}

impl PublicationPrefix {
    pub async fn settle(self) -> Result<(), PublicationFailure> {
        let mut outcome = self.outcome;
        for receipt in self.pending {
            outcome = outcome.and(receipt.await);
        }
        outcome
    }
}
