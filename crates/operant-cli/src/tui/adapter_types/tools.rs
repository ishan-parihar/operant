/// Task status enum used by the tasks overlay. (TaskStore and
/// UserQuestionEvent were deleted in iter-115 — TaskStore was dead
/// code, UserQuestionEvent was replaced by operant_core::user_question::
/// UserQuestionRequest in iter-97.)
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    #[allow(dead_code)] // Prepared for task overlay integration
    Running,
    Completed,
    #[allow(dead_code)] // Prepared for task overlay integration
    Failed,
    InProgress,
    #[allow(dead_code)] // Prepared for task overlay integration
    Deleted,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "Pending"),
            TaskStatus::Running => write!(f, "Running"),
            TaskStatus::Completed => write!(f, "Completed"),
            TaskStatus::Failed => write!(f, "Failed"),
            TaskStatus::InProgress => write!(f, "In Progress"),
            TaskStatus::Deleted => write!(f, "Deleted"),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Background task tracking
pub struct Task {
    pub id: String,
    pub status: TaskStatus,
    pub description: String,
    pub subject: String,
}
