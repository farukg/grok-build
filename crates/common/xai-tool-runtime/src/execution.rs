//! Actual typed dispatch timing, independent of call admission and post-processing.
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use tokio::time::Instant;

use crate::{Tool, ToolCallContext, ToolStream, ToolStreamItem};
use xai_tool_protocol::tool_execution::{ToolExecutionId, ToolExecutionTermination, ToolExecutionTiming};

/// Publications retain the projection captured at actual dispatch.
#[derive(Clone)]
pub struct ToolExecutionObservation(pub Arc<dyn Fn(ToolExecutionTiming) + Send + Sync>);

enum ExecutionState {
    Running {
        observation: Arc<ToolExecutionObservation>,
        execution_id: ToolExecutionId,
        started: Instant,
        started_at: xai_monotonic_clock::MonotonicPoint,
    },
    Settled,
}

struct ExecutionGuard(ExecutionState);

impl ExecutionGuard {
    fn start(observation: Arc<ToolExecutionObservation>) -> Self {
        let execution_id = ToolExecutionId::generate();
        let started = Instant::now();
        let started_at = xai_monotonic_clock::MonotonicPoint::at(started);
        (observation.0)(ToolExecutionTiming::Started { execution_id, started_at });
        Self(ExecutionState::Running { observation, execution_id, started, started_at })
    }

    fn finish(&mut self, termination: ToolExecutionTermination) {
        match std::mem::replace(&mut self.0, ExecutionState::Settled) {
            ExecutionState::Running { observation, execution_id, started, started_at } => {
                let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                (observation.0)(ToolExecutionTiming::Finished { execution_id, started_at, duration_ms, termination });
            }
            ExecutionState::Settled => {}
        }
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        self.finish(ToolExecutionTermination::DispatchCancelled);
    }
}

struct ObservedStream<T> {
    inner: ToolStream<T>,
    guard: ExecutionGuard,
}

impl<T> Stream for ObservedStream<T> {
    type Item = ToolStreamItem<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let next = this.inner.poll_next_unpin(cx);
        match &next {
            Poll::Ready(Some(ToolStreamItem::Terminal(_))) => {
                this.guard.finish(ToolExecutionTermination::Returned);
            }
            Poll::Ready(None) => this.guard.finish(ToolExecutionTermination::StreamEndedWithoutTerminal),
            Poll::Ready(Some(ToolStreamItem::Progress(_))) | Poll::Pending => {}
        }
        next
    }
}

/// Called after typed argument validation and the dispatcher's admission/resource guards.
/// Dropping this future or its returned stream ends dispatch, not a detached backend process.
pub async fn execute_observed<T: Tool>(
    tool: &T,
    ctx: ToolCallContext,
    args: T::Args,
) -> ToolStream<T::Output> {
    let guard = ctx.get::<ToolExecutionObservation>().map(ExecutionGuard::start);
    let inner = tool.execute(ctx, args).await;
    match guard {
        Some(guard) => Box::pin(ObservedStream { inner, guard }),
        None => inner,
    }
}

#[cfg(test)]
#[path = "execution_tests.rs"]
mod tests;
