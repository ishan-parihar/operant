//! Context engine CLI subcommand
//!
//! Provides `operant context status`, `operant context recall <query>`, and
//! `operant context sessions` — CLI-level diagnostics for the lossless DAG
//! context engine (`agent.context_engine = "lcm"`). The agent-facing tools
//! (`lcm_recall`, `lcm_stats`) remain the primary surface; these commands
//! give the operator a read-only view without launching the agent.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::client::{Message, OpenAIClient};
use operant_core::config::AppConfig;
use operant_core::context::rollup::{self, RollupPeriod, RollupSummary};
use operant_core::context::{ContextEngine, LcmConfig, LcmContextEngine, RecallHit};

/// Inspect the context engine (lossless DAG)
#[derive(Debug, Clone, Subcommand)]
pub enum ContextSubcommand {
    /// Show DAG engine status: db path, tail budget, node counts
    Status,
    /// Recall nodes from the DAG by FTS query
    Recall {
        /// Search query (phrase semantics)
        query: String,

        /// Max hits to show (default: 5)
        #[arg(long, default_value_t = 5)]
        limit: usize,

        /// Scope recall to one session id (default: all sessions)
        #[arg(short, long)]
        session: Option<String>,
    },
    /// List sessions present in the DAG with node counts and last activity
    Sessions,

    /// Build an LLM rollup summary of DAG content for a session period
    Rollup {
        /// Session id to roll up
        session: String,

        /// Period to summarize: day | week | month (default: day)
        #[arg(long, default_value = "day")]
        period: String,

        /// UTC date anchor YYYY-MM-DD (default: today)
        #[arg(long)]
        date: Option<String>,
    },

    /// Show stored rollups for a session
    Rollups {
        /// Session id
        session: String,
    },
}

/// Dispatch a context subcommand.
pub async fn handle_context_command(config: &AppConfig, cmd: ContextSubcommand) -> Result<()> {
    match cmd {
        ContextSubcommand::Status => {
            let engine = engine_from_config(config)?;
            print!("{}", status_report(&engine)?);
        }
        ContextSubcommand::Recall {
            query,
            limit,
            session,
        } => {
            let engine = engine_from_config(config)?;
            let hits = engine
                .recall(session.as_deref(), &query, limit.clamp(1, 50))
                .await
                .context("recall failed")?;
            print!("{}", render_hits(&query, &hits));
        }
        ContextSubcommand::Sessions => {
            let engine = engine_from_config(config)?;
            print!("{}", sessions_report(&engine)?);
        }
        ContextSubcommand::Rollup {
            session,
            period,
            date,
        } => {
            let engine = engine_from_config(config)?;
            let period = period.parse::<RollupPeriod>().map_err(anyhow::Error::msg)?;
            let anchor = date
                .map(|d| {
                    chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                        .with_context(|| format!("invalid date '{d}' (expected YYYY-MM-DD)"))
                })
                .transpose()?;
            let client = OpenAIClient::new(crate::client_config(config));
            let model = config.agent.model.clone();
            if model.is_empty() {
                anyhow::bail!(
                    "agent.model not configured — set model = \"...\" in your config to use rollup"
                );
            }
            let summarizer = move |transcript: String| {
                let client = client.clone();
                let model = model.clone();
                async move {
                    let msgs = vec![
                        Message::system(
                            "You are a lossless temporal summarizer. Summarize the \
                             following conversation excerpt into concise key facts \
                             and decisions. Preserve names, numbers, and dates \
                             exactly. Output only the summary.",
                        ),
                        Message::user(transcript),
                    ];
                    let resp = client
                        .chat(&model, &msgs, None, Some(1024), Some(0.2))
                        .await
                        .map_err(|e| {
                            operant_core::error::Error::Agent(format!(
                                "rollup LLM call failed: {e}"
                            ))
                        })?;
                    let content = resp
                        .choices
                        .first()
                        .and_then(|c| c.message.content.clone())
                        .unwrap_or_default();
                    Ok(content.trim().to_string())
                }
            };
            match rollup::build_rollup(&engine, &session, period, anchor, summarizer).await? {
                Some(summary) => {
                    print!("{}", render_rollup(&summary));
                }
                None => println!("No DAG content in that period for session '{session}'."),
            }
        }
        ContextSubcommand::Rollups { session } => {
            let engine = engine_from_config(config)?;
            let all = engine.list_rollups(&session)?;
            if all.is_empty() {
                println!(
                    "No rollups for session '{session}'. Run `operant context rollup {} --period day|week|month` first.",
                    session
                );
            } else {
                for r in all {
                    print!("{}", render_rollup(&r));
                }
            }
        }
    }
    Ok(())
}

fn render_rollup(r: &RollupSummary) -> String {
    let when = chrono::DateTime::from_timestamp_millis(r.created_at)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "?".into());
    format!(
        "[{} {}] {} nodes · {}\n{}\n\n",
        r.period_kind, r.period_start, r.source_count, when, r.summary
    )
}

/// Build the LCM engine from config; errors when the engine isn't LCM.
fn engine_from_config(config: &AppConfig) -> Result<LcmContextEngine> {
    if config.agent.context_engine.as_str() != "lcm" {
        anyhow::bail!(
            "context engine is '{}' (expected \"lcm\") — set \
             agent.context_engine = \"lcm\" in your config to use these commands",
            config.agent.context_engine
        );
    }
    let lcm: LcmConfig = crate::lcm_config(config);
    LcmContextEngine::new(lcm).context("failed to open LCM DAG")
}

fn status_report(engine: &LcmContextEngine) -> Result<String> {
    let nodes = engine.node_count_global().context("node count failed")?;
    let sessions = engine.list_sessions().context("session list failed")?;
    let rollups = engine
        .rollup_count_global()
        .context("rollup count failed")?;
    let mut out = String::new();
    out.push_str("LCM lossless DAG engine\n");
    out.push_str(&format!("  db path     : {}\n", engine.db_path().display()));
    out.push_str(&format!(
        "  tail budget : {} tokens (D0 verbatim)\n",
        engine.tail_tokens()
    ));
    out.push_str(&format!("  sessions    : {}\n", sessions.len()));
    out.push_str(&format!("  total nodes : {}\n", nodes));
    out.push_str(&format!("  rollups     : {}\n", rollups));
    if sessions.is_empty() {
        out.push_str("  (no DAG content yet — run the agent once to populate)\n");
    } else {
        let newest = &sessions[0];
        out.push_str(&format!(
            "  newest      : {} ({}/{} nodes)\n",
            newest.0, newest.1, nodes
        ));
    }
    Ok(out)
}

fn sessions_report(engine: &LcmContextEngine) -> Result<String> {
    let sessions = engine.list_sessions().context("session list failed")?;
    let mut out = String::new();
    if sessions.is_empty() {
        out.push_str("(no sessions in the DAG yet)\n");
        return Ok(out);
    }
    out.push_str(&format!(
        "{:<20} {:>8}  {}\n",
        "SESSION", "NODES", "LAST ACTIVITY"
    ));
    for (sid, count, last) in sessions {
        let when = if last > 0 {
            chrono::DateTime::from_timestamp_millis(last)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "?".into())
        } else {
            "?".into()
        };
        out.push_str(&format!("{:<20} {:>8}  {}\n", sid, count, when));
    }
    Ok(out)
}

fn render_hits(query: &str, hits: &[RecallHit]) -> String {
    let mut out = String::new();
    if hits.is_empty() {
        out.push_str(&format!("No DAG hits for query: {query}\n"));
        return out;
    }
    out.push_str(&format!("{} hit(s) for query: {query}\n", hits.len()));
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!(
            "--- hit {} (node {} · {} · score {:.2}) ---\n",
            i + 1,
            h.node_id,
            h.role,
            h.score
        ));
        let snippet: String = h.content.chars().take(600).collect();
        out.push_str(&snippet);
        if h.content.chars().count() > 600 {
            out.push('…');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::client::Message;

    fn test_engine() -> (LcmContextEngine, String) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("operant_ctx_cli_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("ctx-test.db");
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
        })
        .unwrap();
        (engine, format!("{}", db_path.display()))
    }

    #[tokio::test]
    async fn status_report_shows_dag_counts() {
        let (engine, path) = test_engine();
        let turn = vec![
            Message::user("what is the capital of France"),
            Message::assistant("Paris is the capital of France"),
        ];
        engine.ingest_turn("sess_a", &turn).await.unwrap();

        let report = status_report(&engine).unwrap();
        assert!(report.contains("LCM lossless DAG engine"));
        assert!(report.contains(&path));
        assert!(report.contains("sessions    : 1"));
        assert!(report.contains("total nodes : 2"));
        assert!(report.contains("sess_a"));
    }

    #[tokio::test]
    async fn recall_renders_hits_scoped_and_unscoped() {
        let (engine, _) = test_engine();
        let turn = vec![
            Message::user("deploy window policy"),
            Message::assistant("The release cadence is biweekly"),
        ];
        engine.ingest_turn("sess_x", &turn).await.unwrap();
        // A second session must not leak into scoped recall.
        let other = vec![
            Message::user("unrelated"),
            Message::assistant("other content"),
        ];
        engine.ingest_turn("sess_y", &other).await.unwrap();

        let hits = engine
            .recall(Some("sess_x"), "release cadence", 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("biweekly"));

        let all = engine.recall(None, "release", 10).await.unwrap();
        assert!(!all.is_empty());

        let out = render_hits("release cadence", &hits);
        assert!(out.contains("1 hit(s)"));
        assert!(out.contains("biweekly"));
    }

    #[tokio::test]
    async fn sessions_report_lists_multiple_sessions() {
        let (engine, _) = test_engine();
        engine
            .ingest_turn(
                "sess_one",
                &[Message::user("hello"), Message::assistant("hi there")],
            )
            .await
            .unwrap();
        engine
            .ingest_turn(
                "sess_two",
                &[Message::user("second session"), Message::assistant("ok")],
            )
            .await
            .unwrap();

        let report = sessions_report(&engine).unwrap();
        assert!(report.contains("sess_one"));
        assert!(report.contains("sess_two"));
        assert!(report.contains("NODES"));
    }

    #[test]
    fn empty_render_reports_no_hits() {
        let out = render_hits("nothing", &[]);
        assert!(out.contains("No DAG hits for query: nothing"));
    }

    #[test]
    fn render_rollup_formats_summary() {
        let r = RollupSummary {
            period_kind: "day".into(),
            period_start: "2026-08-13".into(),
            summary: "Deploys happen biweekly.".into(),
            source_count: 5,
            created_at: 0,
        };
        let out = render_rollup(&r);
        assert!(out.contains("[day 2026-08-13] 5 nodes"));
        assert!(out.contains("Deploys happen biweekly."));
    }
}
