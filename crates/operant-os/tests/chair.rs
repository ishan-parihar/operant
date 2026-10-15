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
