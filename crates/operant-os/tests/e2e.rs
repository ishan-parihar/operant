//! W1's end-to-end acceptance (audit §5): a REAL `holon dispatch --prepare`
//! environment drives `operant run` with a scripted provider, and the run
//! must engage everything phase C/D built:
//!
//! - the "Holon run detected — policy.json enforcement mounted" log line;
//! - `trajectory.json` beside the brief, in the kernel's shape;
//! - usage merged into the result.
//!
//! The provider is a canned HTTP server (one OpenAI-style completion) so
//! the test never touches a network. `HOLON_REPO` gates it (O-28: a skip
//! is loud).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn holon_repo() -> Option<std::path::PathBuf> {
    std::env::var_os("HOLON_REPO").map(std::path::PathBuf::from)
}

/// The operant binary under test: $OPERANT_BIN (made absolute — a relative
/// one breaks when the harness's cwd differs from the workspace root,
/// observed live) else the repo's own target/debug/operant.
fn operant_under_test() -> std::path::PathBuf {
    let operant = std::env::var_os("OPERANT_BIN")
        .map(std::path::PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            }
        })
        .unwrap_or_else(|| {
            // find target/debug/operant from the crate root (two up)
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop();
            p.pop();
            p.join("target/debug/operant")
        });
    assert!(
        operant.exists(),
        "no operant binary at {} — build it (cargo build -p operant-cli --bin operant) or set OPERANT_BIN (absolute)",
        operant.display()
    );
    operant
}

/// One canned chat completion server: answers every POST /v1/chat/completions
/// with a fixed assistant message, empty tool_calls, and usage numbers.
fn canned_provider() -> (
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
    u16,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = std::sync::Arc::clone(&hits);
    let handle = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let hits = std::sync::Arc::clone(&hits2);
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                let Ok(n) = stream.read(&mut buf) else { return };
                let _ = std::str::from_utf8(&buf[..n]);
                hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = serde_json::json!({
                    "id": "chatcmpl-test", "object": "chat.completion",
                    "created": 0, "model": "scripted",
                    "choices": [{"index": 0, "finish_reason": "stop",
                        "message": {"role": "assistant", "content": "{\"summary\":\"ok\",\"outcome\":\"noop\",\"notes\":\"scripted\"}"}}],
                    "usage": {"prompt_tokens": 111, "completion_tokens": 22, "total_tokens": 133}
                });
                let text = body.to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    text.len(),
                    text
                );
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    (hits, handle, port)
}

/// A two-response scripted provider for the multi-turn body: the first
/// request answers with a file_write tool call aimed OUTSIDE the org's
/// [scope] roots, every later request answers with the final plan JSON.
/// Usage numbers differ per response so per-call metering is observable.
fn multiturn_provider() -> (
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
    u16,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = std::sync::Arc::clone(&hits);
    let handle = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let hits = std::sync::Arc::clone(&hits2);
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                let Ok(n) = stream.read(&mut buf) else { return };
                let _ = std::str::from_utf8(&buf[..n]);
                let n = hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // raw strings: the tool-call frame carries a JSON string
                // INSIDE a JSON string — escaping it through the json! macro
                // is unreadable
                let body = if n == 0 {
                    r#"{"id":"chatcmpl-tool","object":"chat.completion","created":0,"model":"scripted","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"file_write","arguments":"{\"path\":\"/etc/holon-e2e-fence-probe.txt\",\"content\":\"probe\"}"}}]}}],"usage":{"prompt_tokens":111,"completion_tokens":22,"total_tokens":133}}"#
                } else {
                    r#"{"id":"chatcmpl-final","object":"chat.completion","created":0,"model":"scripted","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"{\"kind\":\"plan\",\"findings\":\"done\",\"steps\":[\"one\"]}"}}],"usage":{"prompt_tokens":222,"completion_tokens":22,"total_tokens":244}}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    (hits, handle, port)
}

/// R-07 (AUDIT-2026-10-16-capability-layer): a canned provider that
/// RECORDS every request body, so a test asserts on the built messages —
/// the actual wire — instead of the builder's internals.
fn capturing_provider() -> (
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::thread::JoinHandle<()>,
    u16,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bodies2 = std::sync::Arc::clone(&bodies);
    let handle = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let bodies = std::sync::Arc::clone(&bodies2);
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 16384];
                // read headers, then Content-Length bytes of body — the
                // message array of a Holon run exceeds one chunk
                let header_end = loop {
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
                let want = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < header_end + want {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let body_end = (header_end + want).min(buf.len());
                let body = String::from_utf8_lossy(&buf[header_end..body_end]).to_string();
                bodies.lock().expect("bodies lock").push(body);
                let payload = serde_json::json!({
                    "id": "chatcmpl-r07", "object": "chat.completion",
                    "created": 0, "model": "scripted",
                    "choices": [{"index": 0, "finish_reason": "stop",
                        "message": {"role": "assistant", "content": "{\"summary\":\"ok\",\"outcome\":\"noop\"}"}}],
                    "usage": {"prompt_tokens": 111, "completion_tokens": 22, "total_tokens": 133}
                });
                let out = payload.to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    out.len(),
                    out
                );
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    (bodies, handle, port)
}

/// The real-kernel half both e2e tests share: init → hire (the scratch org
/// has no adapter-installed `operant` driver, so hire the builtin
/// `generic-shell` one) → task add → dispatch --prepare. Returns the
/// prepared env exactly as the kernel exported it. `extra_toml` is
/// appended to the instance's holon.toml after init (the multi-turn test
/// declares [scope] roots so the write fence has something to enforce).
struct RealPrepare {
    env_map: serde_json::Map<String, serde_json::Value>,
    run_dir: std::path::PathBuf,
    employee: String,
    holon_cmd: String,
}

fn real_prepare(repo: &std::path::Path, home: &std::path::Path, extra_toml: &str) -> RealPrepare {
    let holon = |args: &[&str]| {
        let mut c = Command::new("python3");
        c.arg("-m").arg("holon").arg("--home").arg(home);
        for a in args {
            c.arg(a);
        }
        c.env("PYTHONPATH", repo);
        c.output().expect("holon cli")
    };
    let out = holon(&["init"]);
    assert!(
        out.status.success(),
        "init: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    if !extra_toml.is_empty() {
        let current = std::fs::read_to_string(home.join("holon.toml")).unwrap_or_default();
        std::fs::write(
            home.join("holon.toml"),
            format!("{current}\n{extra_toml}\n"),
        )
        .expect("scratch config append");
    }
    let out = holon(&[
        "hr",
        "hire",
        "--name",
        "E2E",
        "--title",
        "E2E",
        "--dept",
        "executive",
        "--jd",
        "probe the run-env contract",
        "--driver",
        "generic-shell",
        "--roles",
        "planner,executor,verifier",
        "--reason",
        "e2e",
    ]);
    assert!(
        out.status.success(),
        "hr hire: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hire_out = String::from_utf8_lossy(&out.stdout).to_string();
    let emp = hire_out
        .lines()
        .find_map(|l| l.split_whitespace().find(|w| w.starts_with("emp_")))
        .expect("hired employee id")
        .to_string();
    let out = holon(&[
        "task",
        "add",
        "E2E probe",
        "--intent",
        "reply ok",
        "--dept",
        "executive",
    ]);
    assert!(
        out.status.success(),
        "task add: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let prep = holon(&["--json", "dispatch", "--as", &emp, "--prepare"]);
    let stdout = String::from_utf8_lossy(&prep.stdout).to_string();
    let shell: serde_json::Value = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    let prepared = shell
        .get("prepared")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let prepared = prepared.as_object().map(|o| o.clone()).unwrap_or_default();
    let env_map = prepared
        .get("env")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let get = |k: &str| env_map.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let run_dir = get("HOLON_RUN_DIR")
        .or_else(|| get("HOLON_BRIEF_DIR"))
        .unwrap_or_else(|| {
            panic!(
                "the kernel's prepare env carries no HOLON_RUN_DIR/HOLON_BRIEF_DIR: {}",
                serde_json::to_string(&env_map).unwrap_or_default()
            )
        });
    RealPrepare {
        run_dir: std::path::PathBuf::from(&run_dir),
        employee: get("HOLON_EMPLOYEE_ID").expect("HOLON_EMPLOYEE_ID in the prepare env"),
        holon_cmd: get("HOLON").expect("HOLON in the prepare env"),
        env_map,
    }
}

#[test]
fn a_real_prepare_env_engages_enforcement_trajectory_and_usage() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): the prepare-env E2E did not run");
        return;
    };
    // The operant binary under test: env override (a debug build made by
    // the operator; an ABSOLUTE path — a relative one breaks when the
    // test harness's cwd differs from the workspace root, observed live)
    // else the repo's own target (cargo test runs in it).
    let operant = operant_under_test();

    let (_hits, _server, port) = canned_provider();

    // 1. scratch instance + a REAL prepare (shared helper): the exact env
    // a real body runs under — the kernel exports it (H-01), operant
    // consumes it (O-01); driving the real thing means a kernel rename or
    // shape change FAILS here instead of at runtime.
    let dir = tempfile::tempdir().expect("tempdir");
    let prep = real_prepare(&repo, &dir.path().join("home"), "");
    let env_map = prep.env_map;
    let run_dir = prep.run_dir;
    let employee = prep.employee;
    let holon_cmd = prep.holon_cmd;
    // policy.json comes with the claim (the kernel writes it at claim);
    // if this kernel predates it, the run below degrades to no-authority —
    // the mount assertion catches that as a failure.

    // 2. the run: observe config + the REAL prepared env
    let cfg = dir.path().join("observe.toml");
    std::fs::write(&cfg, format!(
        "[client]\nbase_url = \"http://127.0.0.1:{port}/v1\"\ntimeout_secs = 60\napi_key = \"sk-test\"\n[agent]\nmodel = \"scripted\"\nmax_iterations = 1\nstream = false\nshow_reasoning = false\n[tui]\nrich_output = false\n[tools]\ndisabled_tools = [\"file_write\"]\n"
    )).expect("cfg");

    let mut cmd = Command::new(&operant);
    cmd.arg("run")
        .arg("-c")
        .arg(&cfg)
        .arg("--max-iterations")
        .arg("1")
        .arg("--no-stream")
        .arg("-q")
        .arg("Reply with a JSON object.")
        .env("HERMES_HOME", dir.path().join("ohome"))
        .env("RUST_LOG", "info")
        .env("HOLON_RUN_DIR", &run_dir)
        .env("HOLON_EMPLOYEE_ID", &employee)
        .env("HOLON", &holon_cmd);
    // the rest of the kernel's prepared env rides too (HOLON_HOME,
    // HOLON_RUN_ID, the run token, stage/posture…) — exactly what a body
    // sees; layering the whole map is the contract this test exists to pin
    for (k, v) in &env_map {
        let Some(v) = v.as_str() else {
            continue;
        };
        cmd.env(k, v);
    }
    let out = cmd.output().expect("operant run");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("policy.json enforcement mounted")
            || stdout.contains("policy.json enforcement mounted"),
        "enforcement never mounted. stderr: {stderr}"
    );

    // 3. trajectory.json in the kernel's shape, beside the brief
    let traj_path = run_dir.join("trajectory.json");
    assert!(
        traj_path.exists(),
        "no trajectory.json in {}",
        run_dir.display()
    );
    let traj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&traj_path).expect("trajectory"))
            .expect("trajectory parses");
    assert!(traj.get("steps").is_some(), "trajectory shape: {traj}");

    // 4. usage.json written always (O-21) — with the PROVIDER's numbers
    // (O-19: metered per call, never the chars/4 estimate — the scripted
    // provider reports prompt_tokens=111/completion_tokens=22, which an
    // estimate cannot reproduce) and the build stamp (O-20).
    let usage_path = run_dir.join("usage.json");
    let usage: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&usage_path)
            .unwrap_or_else(|_| panic!("no usage.json in {}", run_dir.display())),
    )
    .expect("usage.json parses");
    assert_eq!(
        usage["tokens_in"].as_u64(),
        Some(111),
        "usage.json carries the provider-metered input, not an estimate: {usage}"
    );
    assert_eq!(
        usage["tokens_out"].as_u64(),
        Some(22),
        "usage.json carries the provider-metered output: {usage}"
    );
    assert!(
        usage["build"].as_str().is_some_and(|b| !b.is_empty()),
        "the build stamp rides usage.json (O-20): {usage}"
    );
}

/// G10 (REMAINING-GAPS): the multi-turn body. A tool-call round trip inside
/// a real Holon run proves four things one-shot tests cannot:
/// 1. the agent LOOP runs under the prepared env (call → tool → result →
///    call), and usage meters EVERY provider call (the arithmetic derives
///    from the verified call count — a chars/4 estimate cannot);
/// 2. the live policy denies a mutating call from a plan-stage body: the
///    observe posture escalates file_write at the consult, the headless
///    body is denied, and the refusal is RECORDED (O-17 — the ledger the
///    result's `refusals[]` merges, the blocker the kernel reads);
/// 3. the refused write never touches disk;
/// 4. the loop's corrected-retry response to the error result is counted,
///    not assumed away (metering must hold under retries too).
#[test]
fn a_multiturn_body_meters_every_call_and_the_live_fence_refuses() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): the multi-turn E2E did not run");
        return;
    };
    let operant = operant_under_test();
    let (hits, _server, port) = multiturn_provider();

    let dir = tempfile::tempdir().expect("tempdir");
    // [scope] roots = the tempdir only: /etc is outside, so the scripted
    // file_write there must be refused and recorded
    let scope = format!("[scope]\nroots = [\"{}\"]\n", dir.path().display());
    let prep = real_prepare(&repo, &dir.path().join("home"), &scope);
    let run_dir = prep.run_dir.clone();

    // the run: NO disabled_tools — the model must be able to call
    // file_write and be REFUSED by the fence, not by the tool registry
    let cfg = dir.path().join("act.toml");
    std::fs::write(
        &cfg,
        format!(
            "[client]\nbase_url = \"http://127.0.0.1:{port}/v1\"\ntimeout_secs = 60\napi_key = \"sk-test\"\n[agent]\nmodel = \"scripted\"\nmax_iterations = 2\nstream = false\nshow_reasoning = false\ncreation_nudge_interval = 1\n[tui]\nrich_output = false\n"
        ),
    )
    .expect("cfg");

    let mut cmd = Command::new(&operant);
    cmd.arg("run")
        .arg("-c")
        .arg(&cfg)
        .arg("--max-iterations")
        .arg("2")
        .arg("--no-stream")
        .arg("-q")
        .arg("Do the write, then reply with a JSON object.")
        .env("HERMES_HOME", dir.path().join("ohome"))
        .env("RUST_LOG", "info");
    for (k, v) in &prep.env_map {
        let Some(v) = v.as_str() else {
            continue;
        };
        cmd.env(k, v);
    }
    let out = cmd.output().expect("operant run");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("policy.json enforcement mounted")
            || stderr.contains("policy.json enforcement mounted"),
        "enforcement never mounted.\nstderr: {stderr}\nstdout: {stdout}"
    );

    // iter-878's gate: the nudge interval is 1, so the trigger fires on
    // this body's very turn — and a Holon body must SKIP the review
    // spawn (W6's seventh learner). The skip line proves the gate branch;
    // no "Spawning background skill review" line proves the exit-path
    // spawns stayed dormant (they fire on review_fired, which the Holon
    // branch must not latch).
    assert!(
        stdout.contains("skill review skipped") || stderr.contains("skill review skipped"),
        "the Holon-run gate must fire with creation_nudge_interval = 1.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("Spawning background skill review")
            && !stderr.contains("Spawning background skill review"),
        "the skill review must never spawn inside a Holon body\nstdout: {stdout}"
    );

    // multi-turn verified: the tool-call frame plus the loop's response
    // to the fence's error result (the anomaly-retry call) — the exact
    // count is the loop's machinery, not the contract; the contract is
    // that more than one call happened and EVERY call was metered
    let calls = hits.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        calls >= 2,
        "the loop must make more than one provider call: {calls}\nstdout: {stdout}"
    );

    // the refused write never happened
    assert!(
        !std::path::Path::new("/etc/holon-e2e-fence-probe.txt").exists(),
        "a refused write never touches disk"
    );

    // the fence recorded its refusal in the run dir (O-17's ledger) — the
    // observe posture escalates file_write at the CONSULT (lockdown allow
    // list), before the write fence ever runs; the recorded rule names that
    // path. The escalated tool result then triggers the loop's
    // corrected-retry machinery — counted, not assumed away.
    let refusals = std::fs::read_to_string(run_dir.join("refusals.jsonl"))
        .unwrap_or_else(|_| panic!("no refusals.jsonl in {}", run_dir.display()));
    let first: serde_json::Value =
        serde_json::from_str(refusals.lines().next().unwrap_or("{}")).expect("refusal parses");
    assert_eq!(
        first["tool"], "file_write",
        "the refusal names the tool: {first}"
    );
    assert_eq!(first["policy_rule"], "policy.json:escalate", "{first}");

    // usage meters EVERY call: 111 for the tool-call turn, 222 for each
    // later one — the arithmetic derives from the VERIFIED call count, so
    // the loop's retry machinery cannot break the metering pin
    let want_in = 111 + 222 * (calls as u64 - 1);
    let want_out = 22 * calls as u64;
    let usage: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("usage.json"))
            .unwrap_or_else(|_| panic!("no usage.json in {}", run_dir.display())),
    )
    .expect("usage.json parses");
    assert_eq!(
        usage["tokens_in"].as_u64(),
        Some(want_in),
        "every call metered ({calls} calls): {usage}"
    );
    assert_eq!(
        usage["tokens_out"].as_u64(),
        Some(want_out),
        "every call metered ({calls} calls): {usage}"
    );
}

/// R-07 (AUDIT-2026-10-16-capability-layer): a Holon body's only org
/// guidance is the kernel's brief — the single-injector law. The e2e
/// plants what the ungated code injects today: a legacy `AGENTS.md` in
/// the workdir the body runs from, and the owner's global context file
/// under a scratch XDG_CONFIG_HOME (never the operator's real one).
/// The Holon run's built messages must contain neither; the personal
/// run, same planted files, must still contain both — the gate must not
/// leak outside Holon runs.
#[test]
fn a_holon_body_gets_only_the_kernels_guidance_r07() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): the R-07 E2E did not run");
        return;
    };
    let operant = operant_under_test();
    let (bodies, _server, port) = capturing_provider();

    let dir = tempfile::tempdir().expect("tempdir");
    let prep = real_prepare(&repo, &dir.path().join("home"), "");

    // the planted legacy tree (the workdir) and the planted owner context
    let work = dir.path().join("work");
    let xdg = dir.path().join("xdg");
    let ctx = xdg.join("operant").join("context");
    std::fs::create_dir_all(&work).expect("work dir");
    std::fs::create_dir_all(&ctx).expect("xdg context dir");
    std::fs::write(
        work.join("AGENTS.md"),
        "R07E2E LEGACY WORKSPACE MARKER - a stale venture protocol",
    )
    .expect("planted workspace file");
    std::fs::write(ctx.join("global.md"), "R07E2E OWNER GLOBAL MARKER")
        .expect("planted global file");

    let cfg = dir.path().join("observe.toml");
    std::fs::write(
               &cfg,
        format!(
                        "[client]\nbase_url = \"http://127.0.0.1:{port}/v1\"\ntimeout_secs = 60\napi_key = \"sk-test\"\n[agent]\nmodel = \"scripted\"\nmax_iterations = 1\nstream = false\nshow_reasoning = false\n[tui]\nrich_output = false\n[tools]\ndisabled_tools = [\"file_write\"]\n[memory]\nenabled = true\n"
        ),
    )
    .expect("cfg");

    // 1. the Holon run: the full prepare env, cwd = the planted workdir,
    // the owner's context dir redirected to the scratch XDG home
    let mut cmd = Command::new(&operant);
    cmd.arg("run")
        .arg("-c")
        .arg(&cfg)
        .arg("--max-iterations")
        .arg("1")
        .arg("--no-stream")
        .arg("-q")
        .arg("Reply with a JSON object.")
        .env("HERMES_HOME", dir.path().join("ohome"))
        .env("RUST_LOG", "info")
        .env("XDG_CONFIG_HOME", &xdg)
        .current_dir(&work);
    for (k, v) in &prep.env_map {
        let Some(v) = v.as_str() else {
            continue;
        };
        cmd.env(k, v);
    }
    let out = cmd.output().expect("operant run");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("policy.json enforcement mounted")
            || stderr.contains("policy.json enforcement mounted"),
        "the run must be a Holon body. stderr: {stderr}"
    );
    let wire = bodies.lock().expect("bodies").clone();
    assert!(
        !wire.is_empty(),
        "the run must reach the provider: {stdout}"
    );
    for body in &wire {
        assert!(
            !body.contains("R07E2E LEGACY WORKSPACE MARKER"),
            "a Holon body must not carry the legacy tree's AGENTS.md: {body}"
        );
        assert!(
            !body.contains("R07E2E OWNER GLOBAL MARKER"),
            "a Holon body must not carry the owner's global context: {body}"
        );
        assert!(
            !body.contains("<workspace_context>"),
            "a Holon body gets no workspace context block: {body}"
        );
        // R-08: memory is enabled in this config, but a Holon body opens no
        // memory provider — nothing to prefetch, so no <memory_context>
        assert!(
            !body.contains("<memory_context>"),
            "a Holon body gets no memory prefetch — the kernel's bundle is the run's memory: {body}"
        );
    }
    assert!(
        !dir.path().join("ohome").join("memory_wire.sqlite").exists(),
        "R-08: a Holon run must not even create the memory bank in the isolated home"
    );

    // R-09: the body's skill list is the run's department-resolved export
    // (runs/<id>/skills, named in brief.json) — every listed skill exists
    // there, and nothing else is listed. The adapter home holds NO export in
    // this e2e (ohome/skills is absent), so any listed skill can only have
    // come from the run's library.
    let run_dir = prep.run_dir.clone();
    let run_skills = run_dir.join("skills");
    let exported: Vec<String> = std::fs::read_dir(&run_skills)
        .unwrap_or_else(|_| panic!("no skills export in {}", run_skills.display()))
        .filter_map(|e| e.ok().and_then(|e| e.file_name().into_string().ok()))
        .filter(|n| !n.starts_with('.'))
        .collect();
    assert!(
        !exported.is_empty(),
        "the kernel exported the run's resolved library: {run_skills:?}"
    );
    // the wire is JSON — quotes inside the system-prompt string ride as
    // \" — normalize before picking skill names out of the block
    let mut listed: Vec<String> = Vec::new();
    for b in &wire {
        let norm = b.replace("\\\"", "\"").replace("\\n", "\n");
        let Some(block) = norm
            .split("<available_skills>")
            .nth(1)
            .and_then(|rest| rest.split("</available_skills>").next())
        else {
            continue;
        };
        for line in block.lines() {
            if let Some(name) = line
                .trim()
                .strip_prefix("<skill name=\"")
                .and_then(|rest| rest.split('"').next())
            {
                listed.push(name.to_string());
            }
        }
    }
    assert!(
        !listed.is_empty(),
        "the Holon body lists the run's exported skills: {wire:?}"
    );
    for name in &listed {
        assert!(
            exported.contains(name),
            "R-09: skill {name} is listed but is not in the run's export — a twin from another library"
        );
    }

    // 2. personal mode, same planted files: both sources must still ride
    bodies.lock().expect("bodies").clear();
    let mut cmd = Command::new(&operant);
    cmd.arg("run")
        .arg("-c")
        .arg(&cfg)
        .arg("--max-iterations")
        .arg("1")
        .arg("--no-stream")
        .arg("-q")
        .arg("Reply with a JSON object.")
        .env("HERMES_HOME", dir.path().join("ohome"))
        .env("XDG_CONFIG_HOME", &xdg)
        .current_dir(&work);
    let out = cmd.output().expect("operant run (personal)");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let wire = bodies.lock().expect("bodies").clone();
    assert!(
        !wire.is_empty(),
        "the personal run must reach the provider: {stderr}"
    );
    assert!(
        wire.iter()
            .any(|b| b.contains("R07E2E LEGACY WORKSPACE MARKER")),
        "personal mode keeps the workspace walk: {wire:?}"
    );
    assert!(
        wire.iter()
            .any(|b| b.contains("R07E2E OWNER GLOBAL MARKER")),
        "personal mode keeps the owner's global context: {wire:?}"
    );
    // R-08: the personal run, same config, DOES open the memory bank in the
    // isolated home — proving the Holon-mode absence above is the gate, not
    // a config accident
    assert!(
        dir.path().join("ohome").join("memory_wire.sqlite").exists(),
        "R-08: personal mode builds its memory bank under the same config"
    );
}
