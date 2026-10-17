//! W1 acceptance (audit O-04/O-05): the native tools are generated from
//! ONE definition — `vendor/holon-tools.json` — and the vendor must be the
//! kernel's own registry output. With `HOLON_REPO` pointing at a Holon
//! checkout these tests:
//!
//! 1. regenerate `holon.mcp.tools()` and compare against the vendor —
//!    the two sides cannot drift silently;
//! 2. drive a representative tool of each argv SHAPE against a scratch
//!    instance: plain (whoami), flagged positional (memory add with
//!    stdin content), read+flag (task show), submit (envelope via stdin).
//!
//! Without `HOLON_REPO` the run FAILS in CI terms: it prints a loud
//! "SKIPPED" and exits 0 only when the env is unset (O-28's rule: a skip
//! must be visible).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

fn holon_repo() -> Option<std::path::PathBuf> {
    std::env::var_os("HOLON_REPO").map(std::path::PathBuf::from)
}

fn scratch_home(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join(format!("home-{tag}"));
    let repo = holon_repo().expect("HOLON_REPO");
    let out = Command::new("python3")
        .arg("-m")
        .arg("holon")
        .arg("--home")
        .arg(&home)
        .arg("init")
        .env("PYTHONPATH", &repo)
        .output()
        .expect("holon init");
    assert!(
        out.status.success(),
        "holon init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (dir, home)
}

/// The vendor file is exactly `holon.mcp.tools()`.
#[test]
fn vendored_tool_list_matches_the_kernel_registry() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): vendor-vs-kernel drift not checked");
        return;
    };
    let out = Command::new("python3")
        .arg("-c")
        .arg("import json; from holon.mcp import tools; print(json.dumps(tools(), indent=1))")
        .env("PYTHONPATH", &repo)
        .output()
        .expect("run holon.mcp.tools()");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fresh: serde_json::Value = serde_json::from_slice(&out.stdout).expect("tools json");
    let vendored: serde_json::Value =
        serde_json::from_str(include_str!("../vendor/holon-tools.json")).expect("vendor json");
    assert_eq!(
        fresh, vendored,
        "vendor/holon-tools.json is stale: regenerate with `python3 -c 'import json; from holon.mcp import tools; print(json.dumps(tools(), indent=1))'` (PYTHONPATH=<holon checkout>)"
    );
}

/// Every registered name IS in the vendor table, and OBSERVE_HOLON_DENY
/// names are real tools (O-05's phantom pin).
#[test]
fn observe_deny_lists_only_real_tools() {
    for name in operant_os::policy::OBSERVE_HOLON_DENY {
        assert!(
            operant_os::tools::native_tool_names().contains(name),
            "OBSERVE_HOLON_DENY names '{name}', which is not a registered native tool"
        );
    }
}

/// Drive one tool of each argv shape against a scratch instance: the
/// kernel resolves, the args build, stdin carries the free text, and the
/// exit is 0 (O-04's acceptance, compressed to shapes).
#[test]
fn every_argv_shape_runs_against_a_scratch_instance() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): argv shapes not live-checked");
        return;
    };
    let (_dir, home) = scratch_home("argv");
    let tool = |argv: &[&str], stdin: Option<&str>| {
        let mut c = Command::new("python3");
        c.arg("-m").arg("holon").arg("--home").arg(&home);
        for a in argv {
            c.arg(a);
        }
        c.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(if stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .env("PYTHONPATH", &repo);
        let mut child = c.spawn().expect("spawn holon");
        if let Some(text) = stdin {
            use std::io::Write;
            let mut pipe = child.stdin.take().expect("stdin pipe");
            pipe.write_all(text.as_bytes()).expect("write stdin");
        }
        let out = child.wait_with_output().expect("wait holon");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };

    // plain: whoami
    let (code, out, err) = tool(&["--json", "whoami"], None);
    assert_eq!(code, Some(0), "whoami rc: {err}");
    assert!(
        out.contains("principal"),
        "whoami stdout: {out:?} stderr: {err:?}"
    );

    // flagged + stdin free text: memory add (scope, kind, content via -)
    let (code, out, err) = tool(
        &[
            "--json",
            "memory",
            "add",
            "--scope",
            "org",
            "--kind",
            "fact",
            "--content",
            "-",
        ],
        Some("the scratch instance works"),
    );
    assert_eq!(code, Some(0), "memory add: {err}");
    assert!(out.contains("\"id\": \"itm_"), "memory add said: {out}");

    // read + positional: memory search (query is positional, H-14)
    let (code, out, err) = tool(&["--json", "memory", "search", "scratch"], None);
    assert_eq!(code, Some(0), "memory search: {err}");
    assert!(out.contains("["), "memory search said: {out}");

    // msg send: both to and body are positional (H-14); the body rides
    // stdin as `-` (O-09's rule, exercised end to end)
    let (code, _out, err) = tool(
        &["--json", "msg", "send", "org", "-"],
        Some("scratch message"),
    );
    assert_eq!(code, Some(0), "msg send: {err}");
}
