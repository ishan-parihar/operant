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

    let (_hits, _server, port) = canned_provider();

    // 1. scratch instance + a REAL prepare: init → hire (the scratch org
    // has no adapter-installed `operant` driver, so hire the builtin
    // `generic-shell` one) → task add → dispatch --prepare. This is the
    // exact env a real body runs under — the kernel exports it (H-01),
    // operant consumes it (O-01); driving the real thing means a kernel
    // rename or shape change FAILS here instead of at runtime.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let holon = |args: &[&str]| {
        let mut c = Command::new("python3");
        c.arg("-m").arg("holon").arg("--home").arg(&home);
        for a in args {
            c.arg(a);
        }
        c.env("PYTHONPATH", &repo);
        c.output().expect("holon cli")
    };
    let out = holon(&["init"]);
    assert!(
        out.status.success(),
        "init: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // probation→active not required for a claim, but keep it explicit
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
    // The H-01 contract, asserted from the REAL kernel output:
    let run_dir = get("HOLON_RUN_DIR")
        .or_else(|| get("HOLON_BRIEF_DIR"))
        .unwrap_or_else(|| {
            panic!(
                "the kernel's prepare env carries no HOLON_RUN_DIR/HOLON_BRIEF_DIR: {}",
                serde_json::to_string(&env_map).unwrap_or_default()
            )
        });
    let run_dir = std::path::PathBuf::from(&run_dir);
    let employee = get("HOLON_EMPLOYEE_ID").expect("HOLON_EMPLOYEE_ID in the prepare env");
    let holon_cmd = get("HOLON").expect("HOLON in the prepare env");
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
