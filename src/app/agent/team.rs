//! Group membership and one task's lifecycle, independent of tabs and RPC I/O.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Independent,
    Coordinator,
    Worker,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Independent => t!("Independent"),
            Self::Coordinator => t!("Coordinator"),
            Self::Worker => t!("Worker"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub text: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub delivered: bool,
    #[serde(skip)]
    pub start_item: usize,
}

impl Task {
    pub fn new(text: String, start_item: usize) -> Self {
        Self { text, start_item, status: TaskStatus::Pending, result: None, delivered: false }
    }

    pub fn active(&self) -> bool {
        matches!(self.status, TaskStatus::Pending | TaskStatus::Running)
    }

    pub fn finish(&mut self, status: TaskStatus, result: String) {
        if self.active() {
            self.status = status;
            self.result = Some(result);
        }
    }

    /// Only terminal tasks can be delivered. Repeated settled/exit events are harmless.
    pub fn take_delivery(&mut self) -> Option<String> {
        if self.active() || self.delivered {
            return None;
        }
        let result = self.result.clone()?;
        self.delivered = true;
        Some(format!("Task: {}\nStatus: {:?}\nResult:\n{result}", self.text, self.status))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedAgent {
    pub path: String,
    #[serde(default)]
    pub persisted: bool,
    pub title: String,
    #[serde(default)]
    pub role: Role,
    pub group: Option<String>,
    pub docked: bool,
    #[serde(default)]
    pub paused: bool,
    pub task: Option<Task>,
    #[serde(default)]
    pub draft: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_deliver_once_and_not_before_settled() {
        let mut task = Task::new("Review parser".into(), 12);
        assert!(task.take_delivery().is_none());
        task.status = TaskStatus::Running;
        assert!(task.take_delivery().is_none());
        task.finish(TaskStatus::Completed, "No regressions".into());
        assert!(task.take_delivery().unwrap_or_else(|| panic!("expected delivery")).contains("No regressions"));
        assert!(task.take_delivery().is_none());
        task.finish(TaskStatus::Failed, "late exit".into());
        assert_eq!(task.status, TaskStatus::Completed);
    }

    #[test]
    fn saved_pending_work_is_not_replayed() {
        let saved = SavedAgent {
            path: "/session.jsonl".into(),
            persisted: true,
            title: "Worker".into(),
            role: Role::Worker,
            group: Some("team".into()),
            docked: false,
            paused: false,
            task: Some(Task::new("Edit code".into(), 2)),
            draft: "Continue here".into(),
        };
        let roundtrip: SavedAgent =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap_or_else(|e| panic!("serialize failed: {e}")))
                .unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(roundtrip.task.unwrap_or_else(|| panic!("expected a task")).status, TaskStatus::Pending);
        assert_eq!(roundtrip.draft, "Continue here");
    }
}
