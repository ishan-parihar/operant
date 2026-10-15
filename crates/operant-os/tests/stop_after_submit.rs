//! O-12 (CONTRACT §4): stop-after-submit. A body that hands its result in
//! itself (`holon submit`, the defer path) frees the supervisor's slot the
//! moment the run leaves "running" — not at the driver's timeout ceiling.
//! This drives a REAL kernel prepare (the same init→hire→task→prepare flow
//! the e2e uses), a body that self-submits, and asserts `run_body` returns
//! well inside the driver ceiling with exit 0.
//!
//! HOLON_REPO gates it (O-28: a skip is loud).

use std::process::Command;

fn holon_repo() -> Option<std::path::PathBuf> {
    std::env::var_os("HOLON_REPO").map(std::path::PathBuf::from)
}

#[tokio::test]
async fn a_self_submitting_body_frees_its_slot_immediately() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): the stop-after-submit E2E did not run");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    // A `bin` wrapper: run_body resolves the kernel through `bin` (the
    // workstation has no `holon` on PATH) — and the body's own `submit`
    // needs PYTHONPATH too, which the wrapper carries.
    let wrapper = dir.path().join("holon.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport PYTHONPATH={repo}\nexec python3 -m holon \"$@\"\n",
            repo = repo.display()
        ),
    )
    .expect("write wrapper");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let bin = wrapper.to_string_lossy().into_owned();
    let holon = |args: &[&str]| {
        let mut c = Command::new(&wrapper);
        c.arg("--home").arg(&home);
        for a in args {
            c.arg(a);
        }
        c.output().expect("holon cli")
    };
    let out = holon(&["init"]);
    assert!(
        out.status.success(),
        "init: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = holon(&[
        "hr",
        "hire",
        "--name",
        "SAS",
        "--title",
        "SAS",
        "--dept",
        "executive",
        "--jd",
        "probe stop-after-submit",
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
        "SAS probe",
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
    let prepared_json = shell
        .get("prepared")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let prepared: operant_os::run::Prepared =
        serde_json::from_value(prepared_json.clone()).expect("a prepared run");
    assert!(
        !prepared.run.is_empty(),
        "nothing to prepare — the claim refused: {prepared_json}"
    );

    // The body: writes a valid plan result (kind/findings/steps — the
    // vendored plan schema's required set), self-submits with the run
    // token, then SLEEPS FOREVER. Without stop-after-submit, run_body
    // waits the full driver ceiling for a process that already handed in.
    let run_dir = prepared
        .brief_md
        .parent()
        .map(std::path::Path::to_path_buf)
        .expect("brief dir");
    let result_path = run_dir.join("result.json");
    let run_id = prepared.run.clone();
    let result_json = r#"{"kind":"plan","findings":"probe findings","steps":["step one"]}"#;
    let submit_cmd = format!(
        "{bin} --home {home} --token {token} submit {run_id} --file {file} >/dev/null 2>&1",
        bin = bin,
        home = home.display(),
        token = prepared
            .env
            .get("HOLON_RUN_TOKEN")
            .map(String::as_str)
            .unwrap_or_default(),
        run_id = run_id,
        file = result_path.display(),
    );
    let script = dir.path().join("body.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             printf '%s' '{result}' > '{result_path}'\n\
             {submit_cmd}\n\
             sleep 600\n",
            result = result_json,
            result_path = result_path.display(),
            submit_cmd = submit_cmd,
        ),
    )
    .expect("write body");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let mut body_prepared = prepared;
    body_prepared.command = format!("'{}'", script.display());
    body_prepared.timeout_s = 300; // the driver ceiling this test must NOT wait out

    let start = std::time::Instant::now();
    let code = operant_os::supervisor::run_body(&home, Some(&bin), &body_prepared)
        .await
        .expect("run_body");
    let elapsed = start.elapsed();
    assert_eq!(code, 0, "a stopped-after-submit body reports a clean exit");
    // the cadence floor is 10s; the stop must land within ~2 cadences —
    // far under the 300s ceiling a slot-squatting body would occupy
    assert!(
        elapsed.as_secs() < 120,
        "run_body took {elapsed:?} — the body squatted its slot after handing in"
    );
    assert!(
        elapsed.as_secs() >= 5,
        "run_body returned in {elapsed:?} — the poll never ran (too fast to have checked)"
    );
}
