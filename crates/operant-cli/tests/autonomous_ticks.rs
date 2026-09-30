//! Supervision-loop battery for `autonomous.rs` (plan A1-A5).
//!
//! The unit tests in `autonomous.rs` drive `publish_changes` and
//! `record_failure` directly, and drive `tick()` only against a real git repo.
//! Three tick-level paths are therefore only half-proved:
//!   - validation failure (autonomous.rs:416-431) — reached only after the agent
//!     both succeeds AND changes the worktree, which the real-git test never
//!     does, and `run_validation` has no test at all;
//!   - `record_failure` keyed on the POST-run snapshot (autonomous.rs:421)
//!     instead of the pre-run one, which is what the whole existing pause test
//!     (`persisted_pause_state_survives_restart_until_workspace_changes`)
//!     never exercises — it drives the agent-error branch, which records the
//!     PRE-run snapshot at :385;
//!   - "no workspace changes" (autonomous.rs:399-410) and
//!     `looks_like_nothing_to_commit` (autonomous.rs:698), both unreached.
//!
//! Every command goes through `CommandExecutor` and every turn through
//! `AutonomousAgentExecutor`, so no git and no model is touched: the only real
//! I/O is TODO.md and autonomous-status.toml in a temp dir.

// `operant-cli` is a binary-only crate (`[[bin]] src/main.rs`, no `src/lib.rs`),
// so `tests/` has no library target to import. `include!` splices the module
// in as a nested module, which puts its *private* seams (`AutonomousRunner`,
// `CommandExecutor`, `load_status_report`) within reach of a child module
// without adding a `[lib]` target or editing the source file.
// Integration test binaries are also not covered by the `#![cfg_attr(test,
// ...)]` exemption in main.rs, so the gate's -D flags reach here.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fakes and assertions; the spliced module keeps its own private helpers"
)]

use std::path::Path;

use anyhow::Result;
use operant_core::config::AppConfig;
use operant_core::mcp::McpManager;

/// Stand-in for `main.rs`'s agent factory. `autonomous.rs` only needs it to
/// type-check `RealAutonomousAgentExecutor`, which these tests never build.
struct StubAgent;

impl StubAgent {
    async fn run(self, _query: String) -> Result<()> {
        Ok(())
    }
}

async fn create_agent_without_events(
    _config: &AppConfig,
    _system_prompt: Option<&str>,
    _mcp_manager: &McpManager,
    _skills_dir: &Path,
) -> Result<StubAgent> {
    Ok(StubAgent)
}

mod autonomous {
    include!("../src/autonomous.rs");

    /// Plan A1-A5. Lives inside the spliced module so it can reach the
    /// private supervisor seams; `use super::*` also brings in the std/serde
    /// imports the module already declares.
    mod battery {
        use super::*;
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        const TODO: &str =
            "## Implemented\n- bootstrap repo\n\n## Pending\n- automate sample task\n";
        const TODO_EXTRA: &str = "## Implemented\n- bootstrap repo\n\n## Pending\n- automate sample task\n- retry after workspace change\n";
        const TODO_MORE: &str = "## Implemented\n- bootstrap repo\n\n## Pending\n- automate sample task\n- retry after workspace change\n- and again\n";

        static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone, Copy)]
        enum AgentStep {
            /// Leaves the worktree exactly as the tick found it.
            NoOp,
            /// Makes the worktree dirty with the given `git status --short` line.
            Touch(&'static str),
        }

        #[derive(Clone, Copy)]
        enum ValidationPlan {
            Pass,
            Fail(&'static str),
        }

        #[derive(Clone, Copy)]
        enum CommitPlan {
            Record,
            NothingToCommit,
        }

        struct TempRepo {
            path: PathBuf,
        }

        impl TempRepo {
            fn new(todo: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "operant-autonomous-ticks-{}-{}",
                    std::process::id(),
                    NEXT_TEMP_DIR.fetch_add(1, Ordering::SeqCst)
                ));
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("TODO.md"), todo).unwrap();
                Self { path }
            }

            fn edit_todo(&self, todo: &str) {
                std::fs::write(self.path.join("TODO.md"), todo).unwrap();
            }

            fn status(&self) -> AutonomousStatusReport {
                load_status_report(&self.path.join("autonomous-status.toml"))
                    .unwrap()
                    .expect("tick must write a status report")
            }
        }

        impl Drop for TempRepo {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        fn success_outcome(stdout: &str) -> CommandOutcome {
            CommandOutcome {
                success: true,
                exit_code: Some(0),
                stdout: stdout.to_string(),
                stderr: String::new(),
                timed_out: false,
            }
        }

        fn failure_outcome(stderr: &str) -> CommandOutcome {
            CommandOutcome {
                success: false,
                exit_code: Some(1),
                stdout: String::new(),
                stderr: stderr.to_string(),
                timed_out: false,
            }
        }

        fn args_of(spec: &CommandSpec) -> Vec<String> {
            match &spec.kind {
                CommandKind::Exec { args, .. } => args.clone(),
                CommandKind::Shell { command } => vec![command.clone()],
            }
        }

        #[derive(Clone)]
        struct FakeCommandExecutor {
            status: Arc<Mutex<String>>,
            validations: Arc<Mutex<VecDeque<ValidationPlan>>>,
            commit: Arc<Mutex<CommitPlan>>,
            calls: Arc<Mutex<Vec<String>>>,
            add_args: Arc<Mutex<Vec<String>>>,
            push_args: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl CommandExecutor for FakeCommandExecutor {
            async fn run(&self, spec: CommandSpec) -> Result<CommandOutcome> {
                self.calls.lock().unwrap().push(spec.description.clone());
                Ok(match spec.description.as_str() {
                    "read git branch" => success_outcome("agent-dev\n"),
                    "read git head" => success_outcome("abc123\n"),
                    "read git status" => success_outcome(&self.status.lock().unwrap().clone()),
                    "run validation" => match self.validations.lock().unwrap().pop_front() {
                        None | Some(ValidationPlan::Pass) => success_outcome(""),
                        Some(ValidationPlan::Fail(stderr)) => failure_outcome(stderr),
                    },
                    "git add" => {
                        *self.add_args.lock().unwrap() = args_of(&spec);
                        success_outcome("")
                    }
                    "git commit" => match *self.commit.lock().unwrap() {
                        CommitPlan::Record => success_outcome("[agent-dev abc123] Auto-commit"),
                        CommitPlan::NothingToCommit => {
                            failure_outcome("nothing to commit, working tree clean")
                        }
                    },
                    "git push" => {
                        *self.push_args.lock().unwrap() = args_of(&spec);
                        success_outcome("pushed")
                    }
                    other => panic!("unplanned command: {}", other),
                })
            }
        }

        #[derive(Clone)]
        struct FakeAgent {
            steps: Vec<AgentStep>,
            status: Arc<Mutex<String>>,
            queries: Arc<Mutex<Vec<String>>>,
            calls: Arc<AtomicUsize>,
        }

        #[async_trait(?Send)]
        impl AutonomousAgentExecutor for FakeAgent {
            async fn run(&self, _snapshot: &WorkspaceSnapshot, query: String) -> Result<()> {
                let index = self.calls.fetch_add(1, Ordering::SeqCst);
                self.queries.lock().unwrap().push(query);
                let step = self
                    .steps
                    .get(index)
                    .copied()
                    .or_else(|| self.steps.last().copied())
                    .expect("agent step script must not be empty");
                match step {
                    AgentStep::NoOp => {}
                    AgentStep::Touch(status) => {
                        *self.status.lock().unwrap() = status.to_string();
                    }
                }
                Ok(())
            }
        }

        struct Fakes {
            executor: FakeCommandExecutor,
            agent: FakeAgent,
            calls: Arc<Mutex<Vec<String>>>,
            add_args: Arc<Mutex<Vec<String>>>,
            push_args: Arc<Mutex<Vec<String>>>,
            agent_calls: Arc<AtomicUsize>,
            queries: Arc<Mutex<Vec<String>>>,
            commit: Arc<Mutex<CommitPlan>>,
        }

        impl Fakes {
            fn calls(&self) -> Vec<String> {
                self.calls.lock().unwrap().clone()
            }

            fn agent_calls(&self) -> usize {
                self.agent_calls.load(Ordering::SeqCst)
            }
        }

        fn fakes(
            agent_steps: Vec<AgentStep>,
            validations: Vec<ValidationPlan>,
            commit: CommitPlan,
        ) -> Fakes {
            let status = Arc::new(Mutex::new(String::new()));
            let calls = Arc::new(Mutex::new(Vec::new()));
            let add_args = Arc::new(Mutex::new(Vec::new()));
            let push_args = Arc::new(Mutex::new(Vec::new()));
            let agent_calls = Arc::new(AtomicUsize::new(0));
            let queries = Arc::new(Mutex::new(Vec::new()));
            let commit_plan = Arc::new(Mutex::new(commit));

            Fakes {
                executor: FakeCommandExecutor {
                    status: status.clone(),
                    validations: Arc::new(Mutex::new(validations.into())),
                    commit: commit_plan.clone(),
                    calls: calls.clone(),
                    add_args: add_args.clone(),
                    push_args: push_args.clone(),
                },
                agent: FakeAgent {
                    steps: agent_steps,
                    status,
                    queries: queries.clone(),
                    calls: agent_calls.clone(),
                },
                calls,
                add_args,
                push_args,
                agent_calls,
                queries,
                commit: commit_plan,
            }
        }

        impl Fakes {
            fn set_commit(&self, plan: CommitPlan) {
                *self.commit.lock().unwrap() = plan;
            }
        }

        fn runner(
            repo: &TempRepo,
            fakes: &Fakes,
            max_failures_per_state: usize,
        ) -> AutonomousRunner<FakeCommandExecutor, FakeAgent> {
            let mut config = AppConfig::default();
            config.autonomous.max_failures_per_state = max_failures_per_state;
            AutonomousRunner::new(
                config,
                repo.path.clone(),
                fakes.executor.clone(),
                fakes.agent.clone(),
            )
        }

        /// A1: a validation failure is the first place the supervisor writes a
        /// *command-derived* signature. The agent has to succeed and dirty the
        /// worktree, otherwise the tick short-circuits earlier at
        /// "no workspace changes".
        #[tokio::test]
        async fn validation_failure_is_recorded_with_its_signature() {
            let repo = TempRepo::new(TODO);
            let fakes = fakes(
                vec![AgentStep::Touch(" M src/lib.rs\n")],
                vec![ValidationPlan::Fail("error: 3 tests failed")],
                CommitPlan::Record,
            );
            let mut runner = runner(&repo, &fakes, 2);

            runner.tick().await.unwrap();

            assert_eq!(fakes.agent_calls(), 1);
            let status = repo.status();
            assert_eq!(status.state, AutonomousState::Failed);
            assert_eq!(status.attempts, 1);
            assert!(!status.paused);
            assert_eq!(
                status.last_failure_signature.as_deref(),
                Some("error: 3 tests failed")
            );
            let error = status.last_error.as_deref().unwrap();
            assert!(
                error.starts_with("Validation failed: exit code 1"),
                "unexpected last_error: {}",
                error
            );
            assert!(error.contains("error: 3 tests failed"));
            let validation = status.last_validation.as_ref().unwrap();
            assert!(!validation.success);
            assert_eq!(validation.exit_code, Some(1));
            assert_eq!(validation.stderr, "error: 3 tests failed");
            assert!(status.state_fingerprint.is_some());
            // A failing tick must never reach the commit step.
            assert!(fakes.add_args.lock().unwrap().is_empty());
            assert!(!fakes.calls().contains(&"git commit".to_string()));
        }

        /// A2 + A3: the feedback property. The FIRST identical failure is
        /// recorded (Failed, attempts 1); the SECOND identical failure pauses
        /// the same workspace (Paused, attempts 2); the tick after that does
        /// not even invoke the agent; and a workspace edit resumes it.
        ///
        /// A validation failure can never drive this transition:
        /// `record_failure` is keyed on the POST-run snapshot there
        /// (autonomous.rs:421), and reaching validation requires the agent to
        /// change the worktree, so consecutive post-run keys always differ and
        /// `attempts` stays pinned at 1. The transition is only reachable from
        /// the two branches that record the PRE-run snapshot.
        #[tokio::test]
        async fn second_identical_failure_pauses_then_a_workspace_edit_resumes() {
            let repo = TempRepo::new(TODO);
            let fakes = fakes(
                vec![AgentStep::NoOp],
                vec![
                    ValidationPlan::Fail("error: never reached"),
                    ValidationPlan::Fail("error: never reached"),
                ],
                CommitPlan::Record,
            );
            let mut runner = runner(&repo, &fakes, 2);

            // Tick 1: agent succeeded but changed nothing.
            runner.tick().await.unwrap();
            assert_eq!(fakes.agent_calls(), 1);
            let first = repo.status();
            assert_eq!(first.state, AutonomousState::Failed);
            assert_eq!(first.attempts, 1);
            assert!(!first.paused);
            assert_eq!(
                first.last_failure_signature.as_deref(),
                Some("no workspace changes")
            );
            assert!(
                first.last_validation.is_none(),
                "a no-change tick must short-circuit before validation"
            );

            // Tick 2: SAME failure, same workspace, same signature. This is the
            // behaviour change — identical input, different outcome.
            runner.tick().await.unwrap();
            assert_eq!(fakes.agent_calls(), 2);
            let second = repo.status();
            assert_eq!(second.state, AutonomousState::Paused);
            assert_ne!(first.state, second.state);
            assert_eq!(second.attempts, 2);
            assert!(second.paused);
            assert_eq!(
                second.last_failure_signature.as_deref(),
                first.last_failure_signature.as_deref()
            );
            assert_eq!(second.state_fingerprint, first.state_fingerprint);

            // Tick 3: paused on the same fingerprint — the agent is not asked
            // to do anything and validation is never reached, so the only work
            // left is the snapshot capture that precedes the guard.
            let calls_before = fakes.calls();
            runner.tick().await.unwrap();
            assert_eq!(fakes.agent_calls(), 2, "paused tick re-invoked the agent");
            assert_eq!(
                fakes.calls()[calls_before.len()..],
                vec!["read git branch", "read git head", "read git status"]
            );
            let third = repo.status();
            assert_eq!(third.state, AutonomousState::Paused);
            assert_eq!(third.attempts, 2);

            // A workspace edit clears the pause and the next failure starts a
            // fresh attempt count.
            repo.edit_todo(TODO_EXTRA);
            runner.tick().await.unwrap();
            assert_eq!(fakes.agent_calls(), 3);
            let resumed = repo.status();
            assert_eq!(resumed.state, AutonomousState::Failed);
            assert_eq!(resumed.attempts, 1);
            assert!(!resumed.paused);
            assert_eq!(
                resumed.pending_items,
                vec!["automate sample task", "retry after workspace change"]
            );
        }

        /// A4: a clean tick diffs the worktree, then auto-commits and pushes —
        /// with the status file excluded from the commit.
        #[tokio::test]
        async fn success_tick_auto_commits_after_a_worktree_diff() {
            let repo = TempRepo::new(TODO);
            let fakes = fakes(
                vec![AgentStep::Touch(" M src/lib.rs\n")],
                vec![ValidationPlan::Pass],
                CommitPlan::Record,
            );
            let mut runner = runner(&repo, &fakes, 2);

            runner.tick().await.unwrap();

            assert_eq!(fakes.agent_calls(), 1);
            assert_eq!(
                fakes.calls(),
                vec![
                    "read git branch",
                    "read git head",
                    "read git status",
                    // the post-agent diff that proves real work happened
                    "read git status",
                    "read git branch",
                    "read git head",
                    "read git status",
                    "run validation",
                    "git add",
                    "git commit",
                    "read git branch",
                    "read git head",
                    "read git status",
                    "git push",
                    "read git branch",
                    "read git head",
                    "read git status",
                ]
            );
            assert_eq!(
                *fakes.add_args.lock().unwrap(),
                vec!["add", "--all", ".", ":(exclude)autonomous-status.toml"]
            );
            assert_eq!(
                *fakes.push_args.lock().unwrap(),
                vec!["push", "origin", "agent-dev"]
            );

            let status = repo.status();
            assert_eq!(status.state, AutonomousState::Succeeded);
            assert!(status.last_success_unix_secs.is_some());
            assert!(status.last_publish.as_ref().unwrap().branch == "agent-dev");
            assert!(
                status
                    .last_publish
                    .as_ref()
                    .unwrap()
                    .commit_message
                    .starts_with("Auto-commit")
            );
            assert!(status.last_validation.as_ref().unwrap().success);
            assert!(status.last_error.is_none());
            assert!(runner.failure.is_none());
            assert!(fakes.queries.lock().unwrap()[0].contains("Pending items"));
        }

        /// A5: ten ticks of mixed outcomes. Covers the pause skip (twice), the
        /// nothing-to-commit short-circuit, and the ledger round-trip.
        #[tokio::test]
        async fn ten_tick_mixed_run_skips_pauses_and_recovers() {
            let repo = TempRepo::new(TODO);
            // Validation only runs on the three ticks that dirty the worktree.
            let fakes = fakes(
                vec![
                    AgentStep::NoOp,                // t1 no-change failure (attempts 1)
                    AgentStep::NoOp,                // t2 no-change failure -> paused
                    AgentStep::NoOp,                // t4 new failure after the resume
                    AgentStep::Touch(" M a.txt\n"), // t5 -> validated, nothing to commit
                    AgentStep::NoOp,                // t6 no-change failure (attempts 1)
                    AgentStep::NoOp,                // t7 no-change failure -> paused
                    AgentStep::Touch(" M b.txt\n"), // t9 validation failure
                    AgentStep::Touch(" M c.txt\n"), // t10 -> validated, committed
                ],
                vec![
                    ValidationPlan::Pass,
                    ValidationPlan::Fail("error: suite flaked"),
                    ValidationPlan::Pass,
                ],
                CommitPlan::NothingToCommit,
            );
            let mut runner = runner(&repo, &fakes, 2);

            for tick in 1..=10 {
                match tick {
                    4 => repo.edit_todo(TODO_EXTRA),
                    9 => repo.edit_todo(TODO_MORE),
                    10 => fakes.set_commit(CommitPlan::Record),
                    _ => {}
                }
                runner.tick().await.unwrap();
                match tick {
                    5 => {
                        let status = repo.status();
                        assert_eq!(status.state, AutonomousState::Succeeded);
                        assert!(
                            status.last_publish.is_none(),
                            "nothing-to-commit must not report a publish"
                        );
                        assert!(status.last_validation.as_ref().unwrap().success);
                    }
                    7 => assert_eq!(repo.status().state, AutonomousState::Paused),
                    8 => assert_eq!(fakes.agent_calls(), 6, "paused tick re-invoked the agent"),
                    9 => {
                        let status = repo.status();
                        assert_eq!(status.state, AutonomousState::Failed);
                        assert_eq!(
                            status.last_failure_signature.as_deref(),
                            Some("error: suite flaked")
                        );
                        assert_eq!(status.attempts, 1);
                    }
                    _ => {}
                }
            }

            // t3 and t8 were skipped while paused, so the agent ran 8 of 10 ticks.
            assert_eq!(fakes.agent_calls(), 8);

            let status = repo.status();
            assert_eq!(status.state, AutonomousState::Succeeded);
            assert!(runner.failure.is_none());
            assert!(status.last_publish.as_ref().unwrap().branch == "agent-dev");

            // The ledger the tick published is the one TODO.md actually parses to.
            let ledger =
                parse_todo_ledger(&std::fs::read_to_string(repo.path.join("TODO.md")).unwrap());
            assert_eq!(status.implemented_items, ledger.implemented);
            assert_eq!(status.pending_items, ledger.pending);
            assert_eq!(ledger.pending.len(), 3);
        }

        /// The signature heuristic itself: first non-blank output line wins, so
        /// a recompiled rustc banner does not mask the real test failure.
        #[test]
        fn failure_signature_prefers_the_first_output_line() {
            let from_stderr = failure_outcome("error: 1 failed\nnote: run with --nocapture");
            assert_eq!(
                from_stderr.failure_signature("validation"),
                "error: 1 failed"
            );

            let mut stdout_only = success_outcome("warning: unused\n");
            stdout_only.success = false;
            stdout_only.exit_code = None;
            assert_eq!(
                stdout_only.failure_signature("git add failed"),
                "warning: unused"
            );

            let silent = CommandOutcome {
                success: false,
                exit_code: Some(2),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            };
            assert_eq!(
                silent.failure_signature("git push failed"),
                "git push failed failed with exit code 2"
            );

            let mut timed_out = silent.clone();
            timed_out.timed_out = true;
            assert_eq!(
                timed_out.failure_signature("run validation"),
                "run validation timed out"
            );
        }

        #[test]
        fn nothing_to_commit_detects_both_git_phrasings() {
            assert!(looks_like_nothing_to_commit(&failure_outcome(
                "nothing to commit, working tree clean"
            )));
            assert!(looks_like_nothing_to_commit(&failure_outcome(
                "nothing added to commit but untracked files present"
            )));
            // git also prints "no changes added to commit", which the helper
            // misses — such a commit is recorded as a publish failure.
            // Pinned until that gap is fixed.
            assert!(!looks_like_nothing_to_commit(&failure_outcome(
                "no changes added to commit"
            )));
            assert!(!looks_like_nothing_to_commit(&failure_outcome(
                "fatal: not a git repository"
            )));
        }
    }
}
