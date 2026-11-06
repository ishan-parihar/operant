pub mod db;
pub mod schedule;
pub mod scheduler;
pub mod suggestions;

/// The cron database's path for a given main database path.
///
/// ONE rule, in the core, because every cron reader and writer must open the
/// SAME file. `operant cron`'s commands and the agent runtime have always
/// used `<database dir>/operant_cron.db`; the CURATOR opened
/// `<skills>/.curator/cron.db` instead — a file no cron job ever writes to —
/// so its "never archive a cron-referenced skill" guard protected nothing,
/// and the skill a cron job called was archived anyway (RT-267).
pub fn db_path(database_path: &std::path::Path) -> std::path::PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("operant_cron.db")
}

pub use db::{
    CreateJobParams, CronDb, CronJob, CronRewriteDrop, CronRewriteMapping, CronRewriteReport,
};
pub use schedule::normalize_schedule;
pub use scheduler::{CronDelivery, CronScheduler};
pub use suggestions::{CronSuggestion, SuggestionStore, curated_catalog};
