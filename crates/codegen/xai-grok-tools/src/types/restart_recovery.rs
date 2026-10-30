use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartEpoch(pub uuid::Uuid);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RestartAdmission {
    #[default]
    Unaffected,
    Quiescent { epoch: RestartEpoch },
    Reactivated { epoch: RestartEpoch },
    NewSession { epoch: RestartEpoch },
}

impl RestartAdmission {
    pub fn permits_recovery(self) -> bool {
        match self {
            Self::Unaffected | Self::Reactivated { .. } | Self::NewSession { .. } => true,
            Self::Quiescent { .. } => false,
        }
    }

    pub fn after_user_message(self) -> Self {
        match self {
            Self::Unaffected => Self::Unaffected,
            Self::NewSession { epoch } => Self::NewSession { epoch },
            Self::Quiescent { epoch } | Self::Reactivated { epoch } => Self::Reactivated { epoch },
        }
    }
}

#[derive(Debug)]
pub enum ScheduledRestartCohort {
    Unselected,
    Selected(Vec<String>),
    Drained,
}

#[derive(Clone, Debug)]
pub struct SessionRestartRecovery {
    admission: Arc<Mutex<RestartAdmission>>,
    pub scheduled: Arc<tokio::sync::Mutex<ScheduledRestartCohort>>,
    pub activation: Arc<tokio::sync::Mutex<()>>,
}

impl Default for SessionRestartRecovery {
    fn default() -> Self {
        Self::new(RestartAdmission::Unaffected)
    }
}

impl SessionRestartRecovery {
    fn new(admission: RestartAdmission) -> Self {
        let scheduled = match admission {
            RestartAdmission::Unaffected | RestartAdmission::Reactivated { .. } | RestartAdmission::NewSession { .. } => ScheduledRestartCohort::Drained,
            RestartAdmission::Quiescent { .. } => ScheduledRestartCohort::Unselected,
        };
        Self {
            admission: Arc::new(Mutex::new(admission)),
            activation: Arc::default(),
            scheduled: Arc::new(tokio::sync::Mutex::new(scheduled)),
        }
    }

    pub fn admission(&self) -> RestartAdmission {
        *self.admission.lock()
    }

    /// Call only after the corresponding durable admission write is acknowledged.
    pub fn acknowledge(&self, admission: RestartAdmission) {
        *self.admission.lock() = admission;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum RestartStartup {
    #[default]
    Continue,
    Cancel { epoch: RestartEpoch, started_at: DateTime<Utc> },
}

#[derive(Clone, Debug, Default)]
pub struct LeaderRestartRecovery {
    policy: RestartStartup,
    sessions: Arc<Mutex<HashMap<String, SessionRestartRecovery>>>,
}

impl LeaderRestartRecovery {
    pub fn from_startup(policy: RestartStartup) -> Self {
        Self { policy, sessions: Arc::default() }
    }

    pub fn startup(&self) -> RestartStartup {
        self.policy
    }

    pub fn stale(started_at: DateTime<Utc>) -> Self {
        Self {
            policy: RestartStartup::Cancel { epoch: RestartEpoch(uuid::Uuid::now_v7()), started_at },
            sessions: Arc::default(),
        }
    }

    pub fn register(
        &self,
        session_id: &str,
        created_at: DateTime<Utc>,
        persisted: RestartAdmission,
    ) -> SessionRestartRecovery {
        self.sessions.lock().entry(session_id.to_owned()).or_insert_with(|| {
            let admission = match self.policy {
                RestartStartup::Continue => persisted,
                RestartStartup::Cancel { epoch, started_at } if created_at < started_at => match persisted {
                    RestartAdmission::Quiescent { epoch: consumed }
                    | RestartAdmission::Reactivated { epoch: consumed }
                    | RestartAdmission::NewSession { epoch: consumed } if consumed == epoch => persisted,
                    RestartAdmission::Unaffected
                    | RestartAdmission::Quiescent { .. }
                    | RestartAdmission::Reactivated { .. }
                    | RestartAdmission::NewSession { .. } => RestartAdmission::Quiescent { epoch },
                },
                RestartStartup::Cancel { epoch, .. } => match persisted {
                    RestartAdmission::Quiescent { epoch: consumed }
                    | RestartAdmission::Reactivated { epoch: consumed }
                    | RestartAdmission::NewSession { epoch: consumed } if consumed == epoch => persisted,
                    RestartAdmission::Unaffected | RestartAdmission::Quiescent { .. }
                    | RestartAdmission::Reactivated { .. } | RestartAdmission::NewSession { .. } => RestartAdmission::NewSession { epoch },
                },
            };
            SessionRestartRecovery::new(admission)
        }).clone()
    }

    pub fn session(&self, session_id: &str) -> SessionRestartRecovery {
        self.sessions.lock().entry(session_id.to_owned()).or_insert_with(|| {
            SessionRestartRecovery::new(match self.policy {
                RestartStartup::Continue => RestartAdmission::Unaffected,
                RestartStartup::Cancel { epoch, .. } => RestartAdmission::NewSession { epoch },
            })
        }).clone()
    }
}
