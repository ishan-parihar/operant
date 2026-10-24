//! Personal-history word prediction — the in-process port of OMP's
//! text-prediction seam (plan 2026-10-24 M5).
//!
//! OMP runs `pi-predict` behind a JSONL/unix-socket daemon so every omp
//! process shares one engine. Operant runs ONE TUI process per user, so
//! the engine is a dedicated OS thread behind an mpsc — same contract
//! (`complete` never blocks the caller; `observe` learns from every
//! submitted prompt; `persist` writes the snapshot atomically), no daemon.
//!
//! ```text
//! composer (App) ──std mpsc──► engine thread (owns the Model)
//!     ▲                             ├─ bootstrap: history.jsonl ingest
//!     └──tokio mpsc── EngineReply ──├─ complete(before, prefix)
//!                                   ├─ observe(prompt)   [submit path]
//!                                   └─ persist()         [clean exit]
//! ```

pub mod model;
pub mod query;
pub mod snapshot;

use std::path::PathBuf;

use model::Model;
use query::Suggestion;

/// Snapshot file inside the predictor's state directory.
const SNAPSHOT_FILE: &str = "ngram.snapshot";

/// Prompts from the current session, newest last (the session cache).
const SESSION_RING: usize = 64;

/// Messages the engine thread accepts.
enum EngineMsg {
    Observe(String),
    Complete {
        generation: u64,
        before: String,
        prefix: String,
        threshold: Option<f64>,
        reply: tokio::sync::mpsc::Sender<EngineReply>,
    },
    Persist,
}

/// A completion answer back to the App (generation-tagged; stale answers
/// are dropped by the drainer).
pub struct EngineReply {
    pub generation: u64,
    pub suggestion: Option<Suggestion>,
}

/// Handle to the running engine. Cheap to clone; dropping the last handle
/// ends the thread after it drains its channel.
#[derive(Clone)]
pub struct EngineHandle {
    tx: std::sync::mpsc::Sender<EngineMsg>,
}

impl EngineHandle {
    /// Feed one submitted prompt into the model (fire-and-forget).
    pub fn observe(&self, prompt: &str) {
        let _ = self.tx.send(EngineMsg::Observe(prompt.to_string()));
    }

    /// Ask for the ghost suffix of `prefix` (fire-and-forget; the answer
    /// arrives on `reply`).
    pub fn complete(
        &self,
        generation: u64,
        before: String,
        prefix: String,
        threshold: Option<f64>,
        reply: tokio::sync::mpsc::Sender<EngineReply>,
    ) {
        let _ = self.tx.send(EngineMsg::Complete {
            generation,
            before,
            prefix,
            threshold,
            reply,
        });
    }

    /// Flush the snapshot (clean exit).
    pub fn persist(&self) {
        let _ = self.tx.send(EngineMsg::Persist);
    }
}

/// Where the engine keeps its snapshot: `<operant home>/predict/`.
fn state_dir() -> Option<PathBuf> {
    Some(operant_core::platform::operant_home().join("predict"))
}

/// Spawn the engine thread at the app's default state dir: restore the
/// snapshot if present, else bootstrap from `history`.
pub fn spawn(history: &[String]) -> Option<EngineHandle> {
    spawn_at(state_dir()?, history)
}

/// Spawn at an explicit state directory (tests pass a temp dir so they
/// never touch the operator's real predictor state).
pub fn spawn_at(dir: PathBuf, history: &[String]) -> Option<EngineHandle> {
    let model = match std::fs::read(dir.join(SNAPSHOT_FILE)) {
        Ok(bytes) => match snapshot::decode(&bytes) {
            Ok(m) => m,
            Err(e) => {
                // Corrupt/unknown snapshot: wipe and re-ingest (OMP's
                // recovery contract).
                tracing::warn!(error = %e, "word-predictor snapshot unreadable; re-ingesting history");
                let _ = std::fs::remove_dir_all(&dir);
                bootstrap(history)
            }
        },
        Err(_) => bootstrap(history),
    };
    let (tx, rx) = std::sync::mpsc::channel::<EngineMsg>();
    std::thread::Builder::new()
        .name("word-predictor".to_string())
        .spawn(move || engine_loop(model, dir, rx))
        .ok()?;
    Some(EngineHandle { tx })
}

/// Ingest history (oldest → newest) into a fresh model.
fn bootstrap(history: &[String]) -> Model {
    let mut model = Model::default();
    for prompt in history {
        model.learn(prompt);
    }
    model
}

fn engine_loop(mut model: Model, dir: PathBuf, rx: std::sync::mpsc::Receiver<EngineMsg>) {
    let mut session: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    while let Ok(msg) = rx.recv() {
        match msg {
            EngineMsg::Observe(prompt) => {
                model.learn(&prompt);
                session.push_back(prompt);
                while session.len() > SESSION_RING {
                    session.pop_front();
                }
            }
            EngineMsg::Complete {
                generation,
                before,
                prefix,
                threshold,
                reply,
            } => {
                let sess: Vec<String> = session.iter().rev().take(8).cloned().collect();
                let suggestion = query::complete(&model, &sess, &before, &prefix, threshold);
                // blocking_send: this is a sync OS thread, and a failed
                // send (App dropped the receiver mid-shutdown) just
                // abandons the answer.
                let _ = reply.blocking_send(EngineReply {
                    generation,
                    suggestion,
                });
            }
            EngineMsg::Persist => match snapshot::encode(&model) {
                Ok(bytes) => {
                    if let Err(e) = snapshot::write_atomic(&dir, SNAPSHOT_FILE, &bytes) {
                        tracing::warn!(error = %e, "word-predictor snapshot write failed");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "word-predictor snapshot encode failed"),
            },
        }
    }
    // Last handle gone: flush what was learned.
    if let Ok(bytes) = snapshot::encode(&model) {
        let _ = snapshot::write_atomic(&dir, SNAPSHOT_FILE, &bytes);
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    #[test]
    fn engine_round_trip_and_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "operant-pred-engine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let history: Vec<String> = vec![
            "can you refactor the parser".into(),
            "can you review the parser".into(),
            "please review the tests".into(),
        ];
        let engine = spawn_at(dir.clone(), &history).expect("engine spawns");
        engine.observe("review the parser output");
        engine.persist();

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        engine.complete(7, "please rev".into(), "rev".into(), None, tx);
        let reply = rx.blocking_recv().expect("reply");
        assert_eq!(reply.generation, 7);
        assert!(
            reply.suggestion.is_some(),
            "review completes: {:?}",
            reply.suggestion
        );

        drop(engine); // ends the thread → final persist
        let snap = std::fs::read(dir.join(SNAPSHOT_FILE)).expect("snapshot");
        let model = snapshot::decode(&snap).expect("decode");
        assert!(model.id("parser").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_snapshot_wipes_and_reingests() {
        let dir = std::env::temp_dir().join(format!(
            "operant-pred-corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join(SNAPSHOT_FILE), b"{ broken").expect("junk");
        // TWO prompts carrying 'review' — hygiene needs a word in ≥2
        // distinct prompts before it may suggest.
        let engine = spawn_at(
            dir.clone(),
            &[
                "please review the parser".to_string(),
                "review the diff".to_string(),
            ],
        )
        .expect("spawns");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        engine.complete(1, "please rev".into(), "rev".into(), None, tx);
        let reply = rx.blocking_recv().expect("reply");
        assert!(reply.suggestion.is_some(), "re-ingested history answers");
        drop(engine);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Replay the operator's real history: train on all but the last 200
/// single-line prompts, replay the rest, and report keystrokes-saved rate
/// (KSR) and wrong-ghost rate the way OMP's tuning bench does. `#[ignore]`
/// by default — it depends on the operator's home; run explicitly with
/// `cargo test -- --ignored replay_ksr` when measuring (plan M5 gate).
#[test]
#[ignore = "reads the operator's real history; run explicitly to measure"]
fn replay_ksr_on_real_history() {
    let history = crate::tui::input_history::load();
    let single_line: Vec<String> = history
        .iter()
        .rev()
        .filter(|p| !p.contains('\n'))
        .cloned()
        .collect();
    if single_line.len() < 300 {
        eprintln!(
            "history too thin to replay ({} prompts), skipping",
            single_line.len()
        );
        return;
    }
    let holdout = 200usize.min(single_line.len() / 4);
    let split = single_line.len() - holdout;
    let mut model = Model::default();
    for p in &single_line[..split] {
        model.learn(p);
    }
    let mut session: Vec<String> = Vec::new();
    let mut saved = 0usize;
    let mut typed = 0usize;
    let mut wrong_words = 0usize;
    let mut words_total = 0usize;
    for prompt in &single_line[split..] {
        for w in prompt.split_whitespace() {
            words_total += 1;
            typed += w.chars().count();
            let mut first_correct: Option<usize> = None;
            let mut any_wrong = false;
            let lower_word = w.to_lowercase();
            for cut in 1..lower_word.chars().count() {
                let prefix: String = lower_word.chars().take(cut).collect();
                let before = prompt[..prompt.find(w).unwrap_or(0)].to_string();
                match query::complete(&model, &session, &before, &prefix, None) {
                    Some(s)
                        if prefix.chars().count() + s.suffix.chars().count()
                            == lower_word.chars().count()
                            && format!("{prefix}{}", s.suffix) == lower_word =>
                    {
                        // Correct ghost: a typist who tabs at the FIRST
                        // correct cut saves the suffix length.
                        if first_correct.is_none() {
                            first_correct = Some(s.suffix.chars().count());
                        }
                    }
                    Some(_) => any_wrong = true,
                    None => {}
                }
            }
            if let Some(suf_len) = first_correct {
                saved += suf_len;
            } else if any_wrong {
                wrong_words += 1;
            }
        }
        model.learn(prompt);
        session.push(prompt.clone());
        if session.len() > 8 {
            session.remove(0);
        }
    }
    let ksr = saved as f64 / typed.max(1) as f64 * 100.0;
    let wrong_per_100 = wrong_words as f64 / words_total.max(1) as f64 * 100.0;
    eprintln!(
        "REPLAY prompts={} holdout={} KSR={:.2}% wrong-words/100w={:.1}",
        single_line.len(),
        holdout,
        ksr,
        wrong_per_100
    );
}
