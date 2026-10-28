use super::*;
use std::sync::Mutex;
use std::time::Duration;

use crate::{ListToolsContext, ToolDyn, ToolError, ToolErrorKind, TypedToolOutput};
use serde::Serialize;
use xai_tool_protocol::ToolId;
use xai_tool_types::ToolDescription;

#[derive(Serialize)]
struct Output;
impl crate::ToolOutput for Output {}

struct TimedTool;
impl Tool for TimedTool {
    type Args = ();
    type Output = Output;

    fn id(&self) -> ToolId {
        ToolId::new("timed_fixture").unwrap()
    }

    fn description(&self, _: &ListToolsContext) -> ToolDescription {
        ToolDescription::new("timed_fixture", "Timing fixture")
    }

    async fn run(&self, _: ToolCallContext, _: ()) -> Result<Output, ToolError> {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(Output)
    }
}

fn observed_context() -> (ToolCallContext, Arc<Mutex<Vec<ToolExecutionTiming>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let mut ctx = ToolCallContext::default();
    ctx.insert(ToolExecutionObservation(Arc::new(move |timing| {
        sink.lock().unwrap().push(timing);
    })));
    (ctx, events)
}

#[tokio::test(start_paused = true)]
async fn typed_dispatch_times_only_execution_and_settles_before_postprocessing() {
    let (ctx, events) = observed_context();
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(events.lock().unwrap().is_empty());
    let mut dispatch = Box::pin(ToolDyn::execute(&TimedTool, ctx, serde_json::Value::Null));
    assert!(futures::poll!(dispatch.as_mut()).is_pending());
    let started = events.lock().unwrap().clone();
    let [
        ToolExecutionTiming::Started {
            execution_id,
            started_at,
        },
    ] = started.as_slice()
    else {
        panic!("exactly one actual dispatch start is required");
    };
    tokio::time::advance(Duration::from_secs(2)).await;
    let mut stream = dispatch.await;
    assert!(matches!(
        stream.next().await,
        Some(ToolStreamItem::Terminal(Ok(_)))
    ));
    tokio::time::advance(Duration::from_secs(60)).await;
    drop(stream);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            ToolExecutionTiming::Started {
                execution_id: *execution_id,
                started_at: *started_at
            },
            ToolExecutionTiming::Finished {
                execution_id: *execution_id,
                started_at: *started_at,
                duration_ms: 2_000,
                termination: ToolExecutionTermination::Returned,
            },
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn cancelling_actual_dispatch_uses_its_anchor_and_invalid_arguments_never_start() {
    let (ctx, events) = observed_context();
    let mut invalid = ToolDyn::execute(&TimedTool, ctx.clone(), serde_json::json!({})).await;
    assert!(matches!(
        invalid.next().await,
        Some(ToolStreamItem::Terminal(Err(_)))
    ));
    assert!(events.lock().unwrap().is_empty());
    let mut dispatch = Box::pin(ToolDyn::execute(&TimedTool, ctx, serde_json::Value::Null));
    assert!(futures::poll!(dispatch.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(750)).await;
    drop(dispatch);
    let events = events.lock().unwrap();
    let [
        ToolExecutionTiming::Started {
            execution_id: start,
            started_at: start_point,
        },
        ToolExecutionTiming::Finished {
            execution_id: end,
            duration_ms: 750,
            termination: ToolExecutionTermination::DispatchCancelled,
            started_at: finish_point,
        },
    ] = events.as_slice()
    else {
        panic!("cancel must settle its actual dispatch only");
    };
    assert_eq!(start, end);
    assert_eq!(start_point, finish_point);
}

#[tokio::test(start_paused = true)]
async fn actual_point_is_captured_once_and_survives_delayed_delivery() {
    let (ctx, events) = observed_context();
    #[cfg(target_os = "linux")]
    let anchor = Instant::now();
    let mut stream = ToolDyn::execute(&TimedTool, ctx, serde_json::Value::Null).await;
    assert!(matches!(
        stream.next().await,
        Some(ToolStreamItem::Terminal(Ok(_)))
    ));
    tokio::time::advance(Duration::from_secs(90)).await;
    let wire = serde_json::to_string(&*events.lock().unwrap()).unwrap();
    let events: Vec<ToolExecutionTiming> = serde_json::from_str(&wire).unwrap();
    let [
        ToolExecutionTiming::Started {
            execution_id: start_id,
            started_at: start,
        },
        ToolExecutionTiming::Finished {
            execution_id: end_id,
            started_at: end,
            duration_ms: 2_000,
            termination: ToolExecutionTermination::Returned,
        },
    ] = events.as_slice()
    else {
        panic!("missing actual execution point pair");
    };
    assert_eq!(start_id, end_id);
    assert_eq!(start, end);
    #[cfg(target_os = "linux")]
    {
        let xai_monotonic_clock::LocalClockMapping::Correlated {
            earliest_at,
            latest_at,
        } = start.map_local()
        else {
            panic!("same-domain Linux point");
        };
        assert!(earliest_at <= anchor && anchor <= latest_at);
        assert!(latest_at < Instant::now() - Duration::from_secs(90));
    }
}

struct StreamingTool;
impl Tool for StreamingTool {
    type Args = ();
    type Output = Output;

    fn id(&self) -> ToolId {
        ToolId::new("streaming_fixture").unwrap()
    }

    fn description(&self, _: &ListToolsContext) -> ToolDescription {
        ToolDescription::new("streaming_fixture", "Streaming timing fixture")
    }

    async fn execute(&self, _: ToolCallContext, _: ()) -> ToolStream<Output> {
        crate::deferred_terminal(async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Err(ToolError::new(
                ToolErrorKind::Cancelled,
                "typed cancellation",
            ))
        })
    }
}

#[tokio::test(start_paused = true)]
async fn lazy_stream_drop_and_typed_cancellation_are_distinct_and_publish_once() {
    let (ctx, events) = observed_context();
    let mut stream: ToolStream<TypedToolOutput> =
        ToolDyn::execute(&StreamingTool, ctx.clone(), serde_json::Value::Null).await;
    assert!(futures::poll!(stream.next()).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    drop(stream);
    assert!(matches!(
        events.lock().unwrap().last(),
        Some(ToolExecutionTiming::Finished {
            duration_ms: 1_000,
            termination: ToolExecutionTermination::DispatchCancelled,
            ..
        })
    ));
    let mut stream = ToolDyn::execute(&StreamingTool, ctx, serde_json::Value::Null).await;
    assert!(
        matches!(stream.next().await, Some(ToolStreamItem::Terminal(Err(error)))
        if error.kind == ToolErrorKind::Cancelled)
    );
    drop(stream);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 4);
    assert!(matches!(
        events.last(),
        Some(ToolExecutionTiming::Finished {
            duration_ms: 2_000,
            termination: ToolExecutionTermination::Returned,
            ..
        })
    ));
    let ids: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ToolExecutionTiming::Started { execution_id, .. } => Some(execution_id),
            ToolExecutionTiming::Unknown | ToolExecutionTiming::Finished { .. } => None,
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
}
