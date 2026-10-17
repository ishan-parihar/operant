//! W2/O-07: the supervisor hands a body ONLY the OS baseline plus the
//! kernel's prepared env — never its own. A body that dumps its
//! environment must show neither the supervisor's secrets (channel
//! tokens, provider keys, the Chair key) nor its HERMES_HOME.

#![allow(clippy::unwrap_used, clippy::expect_used)]


use operant_os::run::Prepared;

fn dump_driver(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("dump.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nenv | sort > \"$DUMP_OUT\"\necho body-ran\n",
    )
    .expect("write driver");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

#[tokio::test]
async fn a_body_sees_only_the_baseline_and_the_prepared_env() {
    let dir = tempfile::tempdir().expect("tempdir");
    let driver = dump_driver(dir.path());
    let dump = dir.path().join("env.txt");
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).expect("run dir");

    let mut prepared = Prepared {
        run: "run_test".into(),
        stage: "plan".into(),
        task: "tsk_test".into(),
        employee: "emp_test".into(),
        driver: "dump".into(),
        command: format!("{} 2>/dev/null", driver.display()),
        cwd: dir.path().to_path_buf(),
        env: [
            ("HOLON_RUN_TOKEN".to_string(), "run-token-x".to_string()),
            ("HOLON_RUN_DIR".to_string(), run_dir.display().to_string()),
            ("HOLON".to_string(), "python3 -m holon".to_string()),
        ]
        .into_iter()
        .collect(),
        timeout_s: 30,
        brief_md: run_dir.join("BRIEF.md"),
        result_path: run_dir.join("result.json"),
        policy_json: run_dir.join("policy.json"),
        lease_until: "2030-01-01T00:00:00Z".into(),
    };

    // the supervisor's own poisoned environment: secrets that must NEVER
    // reach the body
    // SAFETY: this test runs serially against its own process env; the
    // supervisor reads it only to copy the allowlisted keys. DUMP_OUT
    // rides PATH-baseline smuggling — no: it is set in the PREPARED env
    // below, the only side channel a body may use.
    unsafe {
        std::env::set_var("HOLON_CHAIR_KEY", "chair-secret-DO-NOT-LEAK");
        std::env::set_var("HOLON_KEY", "kernel-key-DO-NOT-LEAK");
        std::env::set_var("OPENAI_API_KEY", "sk-supervisor-DO-NOT-LEAK");
        std::env::set_var("TELEGRAM_BOT_TOKEN", "tg-DO-NOT-LEAK");
        std::env::set_var("HERMES_HOME", "/supervisor-home-DO-NOT-LEAK");
    }
    prepared
        .env
        .insert("DUMP_OUT".to_string(), dump.display().to_string());
    let code = operant_os::supervisor::run_body(
        std::path::Path::new("/tmp"),
        Some("/bin/true"),
        &prepared,
    )
    .await
    .expect("run body");
    unsafe {
        std::env::remove_var("HOLON_CHAIR_KEY");
        std::env::remove_var("HOLON_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("HERMES_HOME");
    }
    assert_eq!(code, 0, "the dump body must run to completion");

    let env_text = std::fs::read_to_string(&dump).expect("the body dumped its env");
    for secret in [
        "chair-secret-DO-NOT-LEAK",
        "kernel-key-DO-NOT-LEAK",
        "sk-supervisor-DO-NOT-LEAK",
        "tg-DO-NOT-LEAK",
        "/supervisor-home-DO-NOT-LEAK",
    ] {
        assert!(
            !env_text.contains(secret),
            "O-07: the supervisor's `{secret}` reached the body's environment"
        );
    }
    assert!(
        env_text.contains("run-token-x"),
        "the prepared env must arrive"
    );
    assert!(env_text.contains("PATH="), "the OS baseline must arrive");
}
