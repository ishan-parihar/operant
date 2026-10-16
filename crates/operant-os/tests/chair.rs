//! Adversarial pins for the Chair's channel (H8) — the identity check is
//! the security boundary between "the organization's owner typed this" and
//! "some chat user typed this". Exact-match only; degradation = nobody.

use std::path::Path;

use operant_os::chair::{is_chair, run_as_chair};

/// A fixture launcher standing in for `holon` — prints the given JSON on
/// stdout (the `chair-channels` contract).
fn launcher_printing(stdout: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("holon.sh");
    // write-then-rename: exec'ing a file some thread just wrote races
    // ETXTBSY on tmpfs under the parallel test harness (observed: two
    // different tests, "Text file busy" at random). The renamed-in inode
    // has no writers, ever.
    let tmp = dir.path().join("holon.sh.tmp");
    std::fs::write(&tmp, format!("#!/bin/sh\nprintf '%s\\n' '{stdout}'\n")).expect("write script");
    std::fs::rename(&tmp, &script).expect("rename script");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (dir, script.to_string_lossy().into_owned())
}

#[tokio::test]
async fn an_exact_identity_is_the_chair() {
    let (_dir, bin) = launcher_printing(r#"{"channels": ["telegram:99887", "notion:u-7"]}"#);
    assert!(
        is_chair(Path::new("/tmp"), Some(&bin), "telegram", "99887")
            .await
            .expect("probe")
    );
}

#[tokio::test]
async fn a_prefix_spoof_is_not_the_chair() {
    // configured telegram:99887; an attacker claims a shorter or longer
    // id sharing the prefix — exact `platform:user_id` only.
    let (_dir, bin) = launcher_printing(r#"{"channels": ["telegram:99887"]}"#);
    for spoof in ["9988", "998877", "099887"] {
        assert!(
            !is_chair(Path::new("/tmp"), Some(&bin), "telegram", spoof)
                .await
                .expect("probe"),
            "prefix spoof '{spoof}' must not match telegram:99887"
        );
    }
}

#[tokio::test]
async fn a_cross_platform_id_is_not_the_chair() {
    // configured telegram:99887; the same numeric id on discord is a
    // different person.
    let (_dir, bin) = launcher_printing(r#"{"channels": ["telegram:99887"]}"#);
    assert!(
        !is_chair(Path::new("/tmp"), Some(&bin), "discord", "99887")
            .await
            .expect("probe")
    );
}

#[tokio::test]
async fn an_unreadable_channel_list_means_nobody_is_the_chair() {
    // garbage from the launcher → the sender is chat, never the Chair.
    let (_dir, bin) = launcher_printing("not json at all");
    assert!(
        !is_chair(Path::new("/tmp"), Some(&bin), "telegram", "99887")
            .await
            .expect("probe")
    );
}

#[tokio::test]
async fn an_empty_channel_list_means_nobody_is_the_chair() {
    let (_dir, bin) = launcher_printing(r#"{"channels": []}"#);
    assert!(
        !is_chair(Path::new("/tmp"), Some(&bin), "telegram", "99887")
            .await
            .expect("probe")
    );
}

#[tokio::test]
async fn a_refused_chair_command_is_a_named_error() {
    // `run_as_chair` is dispatch-only; a refusing launcher (exit != 0)
    // must surface as an error, not silently succeed.
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("holon.sh");
    // write-then-rename — same ETXTBSY guard as launcher_printing
    let tmp = dir.path().join("holon.sh.tmp");
    std::fs::write(&tmp, "#!/bin/sh\nexit 4\n").expect("write");
    std::fs::rename(&tmp, &script).expect("rename");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let bin = script.to_string_lossy().into_owned();
    let err = run_as_chair(Path::new("/tmp"), Some(&bin), &["org"])
        .await
        .expect_err("must fail");
    assert!(
        err.to_string().contains("refused"),
        "named refusal expected, got {err}"
    );
}

/// O-14: the decision router. "approve <ref>" dispatches the kernel's
/// `plan approve` through run_as_chair (the argv the launcher receives is
/// the proof); "reject <ref> because …" carries the note; a non-verb
/// message falls to the classifier; a too-long ref falls through (not a
/// command, someone pasting text).
#[tokio::test]
async fn a_decision_verb_routes_to_the_kernel_plan_command() {
    // the launcher records the argv it received, then answers like the
    // kernel's approve output
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("holon.sh");
    let argv_path = dir.path().join("argv");
    let tmp = dir.path().join("holon.sh.tmp");
    std::fs::write(
        &tmp,
        format!(
            "#!/bin/sh\nprintf '%s\n' \"$@\" > '{}'\nprintf '%s\n' '{{\"approved\": true}}'\n",
            argv_path.display()
        ),
    )
    .expect("write");
    std::fs::rename(&tmp, &script).expect("rename");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let bin = script.to_string_lossy().into_owned();

    // approve: exactly the kernel's argv
    let out = operant_os::chair::dispatch_message(Path::new("/tmp"), Some(&bin), "approve pln_123")
        .await
        .expect("dispatch");
    assert!(
        out.contains("approved"),
        "the kernel's answer rides back: {out}"
    );
    let argv = std::fs::read_to_string(&argv_path).expect("argv recorded");
    let argv_line = argv.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        argv_line.contains("--json plan approve pln_123"),
        "approve routes to the kernel's plan command: {argv}"
    );

    // reject with a because-clause: the note rides --note
    std::fs::write(&argv_path, "").expect("clear");
    let _ = operant_os::chair::dispatch_message(
        Path::new("/tmp"),
        Some(&bin),
        "reject pln_123 because the acceptance is unmeasurable",
    )
    .await
    .expect("dispatch");
    let argv = std::fs::read_to_string(&argv_path).expect("argv recorded");
    let argv_line = argv.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        argv_line.contains("plan reject pln_123 --note"),
        "reject carries its note: {argv}"
    );
    assert!(
        argv_line.contains("the acceptance is unmeasurable"),
        "the note text is the kernel's, verbatim: {argv}"
    );

    // a chaired address still works
    std::fs::write(&argv_path, "").expect("clear");
    let _ =
        operant_os::chair::dispatch_message(Path::new("/tmp"), Some(&bin), "chair, approve pln_9")
            .await
            .expect("dispatch");
    let argv = std::fs::read_to_string(&argv_path).expect("argv recorded");
    let argv_line = argv.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        argv_line.contains("plan approve pln_9"),
        "addressed: {argv}"
    );
}

/// O-14: non-verbs stay classifier traffic — the router must not eat
/// ordinary chat, and a runaway "approve" with no ref is not a command.
#[tokio::test]
async fn a_non_decision_message_falls_to_the_classifier() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("holon.sh");
    // the launcher answers the CLASSIFY shape (the router only calls it
    // after the decision check missed)
    let tmp = dir.path().join("holon.sh.tmp");
    std::fs::write(&tmp, "#!/bin/sh\nprintf '%s\n' '{\"lane\": \"full\"}'\n").expect("write");
    std::fs::rename(&tmp, &script).expect("rename");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let bin = script.to_string_lossy().into_owned();

    // ordinary chat and a verb with no ref: both classify
    for text in ["how is the refactor going?", "approve"] {
        let out = operant_os::chair::dispatch_message(Path::new("/tmp"), Some(&bin), text)
            .await
            .expect("classify falls through");
        assert!(
            out.contains("lane"),
            "'{text}' classified, not routed: {out}"
        );
    }
}
