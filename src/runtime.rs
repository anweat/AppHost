//! Runtime observations only: no retry engine and no implicit process actions.
use serde::Serialize;

mod launch;
pub use launch::{LaunchPlan, LaunchSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceStatus {
    state: State,
    pub log_incomplete: bool,
    pub failure: Option<String>,
}

impl Default for InstanceStatus {
    fn default() -> Self {
        Self {
            state: State::Starting,
            log_incomplete: false,
            failure: None,
        }
    }
}

impl InstanceStatus {
    pub fn state(&self) -> State {
        self.state
    }

    /// Only call after every service has supplied its required readiness evidence.
    pub fn all_ready(&mut self) {
        if self.state == State::Starting {
            self.state = State::Running;
        }
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.failure.get_or_insert_with(|| reason.into());
        if !matches!(self.state, State::Stopping | State::Stopped) {
            self.state = State::Failed;
        }
    }

    pub fn collector_exited(&mut self, success: bool) {
        if !success {
            self.log_incomplete = true;
            // Failure is sticky for this instance. Surviving services are diagnostic
            // evidence, not permission to recreate the collector or report recovery.
            self.fail("LOG_COLLECTION_FAILED");
        }
    }

    pub fn request_stop(&mut self) {
        if self.state != State::Stopped {
            self.state = State::Stopping;
        }
    }

    /// The caller supplies process-group evidence, not just leader exit status.
    pub fn finish_stop(&mut self, all_groups_empty: bool) -> bool {
        if self.state == State::Stopping && all_groups_empty {
            self.state = State::Stopped;
        }
        self.state == State::Stopped
    }
}

pub use crate::platform::collector;
pub mod lifecycle;

mod retention;
