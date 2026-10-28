use serde::{Deserialize, Serialize};
pub use xai_grok_sampler::RequestId;
pub use xai_grok_sampler::request_capture::RequestAttemptId;
pub use xai_grok_sampler::retry_deadline::{RetryClockMapping, RetryDeadlineProjection};

pub const SAMPLING_OWNER_META_KEY: &str = "samplingOwner";
pub const SAMPLING_LIFECYCLE_META_KEY: &str = "samplingLifecycle";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingOwner {
    pub prompt_id: String,
    pub request_id: RequestId,
    pub attempt: RequestAttemptId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingLifecycle {
    pub owner: SamplingOwner,
    #[serde(flatten)]
    pub phase: SamplingPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SamplingPhase {
    RetryScheduled {
        deadline: RetryDeadlineProjection,
    },
    DispatchStarted {
        #[serde(default)]
        started_at: xai_monotonic_clock::MonotonicPoint,
    },
    Finished {
        outcome: SamplingOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingOutcome {
    Completed,
    Failed,
    Cancelled,
}
