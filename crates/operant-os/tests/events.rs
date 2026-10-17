//! W5/O-15's acceptance (audit §5), written by the 2026-10-17 red-team: a
//! matching event fires its routine once per cooldown — the kernel's rc-3
//! skip (H-13) is NOT counted as a firing — and the event text reaches the
//! work VERBATIM.
//!
//! The red-team found intake doubly dead against the current kernel: the
//! pattern match included the `event:` kind marker (nothing real could ever
//! contain it) and the payload spoke the pre-H-06 stdin contract
//! (`--payload -` + stdin), which the literal-`--payload` kernel takes as
//! the string "-". CONTRACT §13 is the spec; this pins all three behaviors
//! against a real kernel. HOLON_REPO gates it (O-28: a skip is loud).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

fn holon_repo() -> Option<std::path::PathBuf> {
    std::env::var_os("HOLON_REPO").map(std::path::PathBuf::from)
}

/// Seed a scratch org exactly the way the kernel's own test harness does
/// (tests/helpers.py + tests/test_lanes.py `EventTriggers`): a config with
/// the pipeline stages off, one `eng` department, and one approved event
/// routine whose `--on` pattern is `event:redpill`. Approval is part of the
/// recipe — an unapproved routine is not a schedule nothing can fire.
fn seed_org(home: &std::path::Path, repo: &std::path::Path, workdir: &std::path::Path) {
    let script = "import sys\n\
         from pathlib import Path\n\
         from holon.core.paths import Home\n\
         from holon.execution.service import Kernel\n\
         home = Home(Path(sys.argv[1]))\n\
         home.root.mkdir(parents=True, exist_ok=True)\n\
         home.config_file.write_text(\n\
             '[pipeline]\\nresearch = \"never\"\\nshape = \"never\"\\nretro = \"never\"\\n'\
             '[healing]\\ndoctor_every_min = 0\\nbackup_every_h = 0\\n'\
             '[grading]\\nnovelty_min_verified = 0\\n[autonomy]\\nverify_sample_rate = 0\\n')\n\
         k = Kernel(home)\n\
         k.org.create_dept('eng', 'Engineering', 'Build and fix things')\n\
         a = k.add_routine('Watch for redpills', 'Summarise each one', dept='eng',\n\
                           event='event:redpill', lane='fast', why='each needs judgement',\n\
                           workdir=sys.argv[2])\n\
         ap = k.work.plan_for_approval(a['id'])\n\
         k.approve(ap['id'])\n";
    let out = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(home)
        .arg(workdir)
        // the kernel's own harness sandboxes every scratch instance's Chair
        // key (tests/helpers.py, I-10): never litter the operator's keyring
        .env("HOLON_CHAIR_KEY_FILE", home.with_extension("chair.key"))
        .env("PYTHONPATH", repo)
        .output()
        .expect("python3 seeds a scratch org");
    assert!(
        out.status.success(),
        "seeding failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The last work message's body, through the kernel's own API — the payload
/// rides the firing's work as a message (CONTRACT §13, `fire_routine`).
fn last_work_message(home: &std::path::Path, repo: &std::path::Path) -> String {
    let script = "import sys\n\
         from pathlib import Path\n\
         from holon.core.paths import Home\n\
         from holon.execution.service import Kernel\n\
         k = Kernel(Home(Path(sys.argv[1])))\n\
         row = k.wconn.execute(\n\
             'SELECT body FROM messages ORDER BY rowid DESC LIMIT 1').fetchone()\n\
         print(row[0] if row else '')\n";
    let out = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(home)
        .env("HOLON_CHAIR_KEY_FILE", home.with_extension("chair.key"))
        .env("PYTHONPATH", repo)
        .output()
        .expect("python3 reads the work messages");
    assert!(
        out.status.success(),
        "read-back failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_matching_event_fires_once_per_cooldown_and_the_payload_survives() {
    let Some(repo) = holon_repo() else {
        eprintln!("SKIPPED (HOLON_REPO unset): the O-15 event-intake E2E did not run");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let workdir = dir.path().join("work");
    std::fs::create_dir_all(&workdir).expect("workdir");
    seed_org(&home, &repo, &workdir);

    // `bin` wrapper (the workstation has no `holon` on PATH) — the same
    // pattern tests/stop_after_submit.rs uses.
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

    // 1. a matching event fires exactly the one routine (and only the
    //    post-marker pattern is matched: the text does NOT contain
    //    "event:redpill").
    let text = "please redpill the quarterly report tonight";
    let fired = operant_os::events::match_and_fire(&home, Some(&bin), text)
        .await
        .expect("first intake");
    assert_eq!(fired, 1, "a matching event must fire its routine once");

    // 2. the event text reached the work verbatim (the payload seam the
    //    red-team caught: pre-H-06 stdin semantics delivered \"-\").
    let body = last_work_message(&home, &repo);
    assert!(
        body.contains(text),
        "the fired work's message must carry the event text verbatim; got: {body:?}"
    );

    // 3. within the cooldown the same text fires nothing, and the skip is
    //    NOT counted (H-13: rc 3 is NOTHING).
    let skipped = operant_os::events::match_and_fire(&home, Some(&bin), text)
        .await
        .expect("second intake");
    assert_eq!(
        skipped, 0,
        "a cooldown skip must not count as a firing (H-13)"
    );

    // 4. a non-matching event fires nothing.
    let miss = operant_os::events::match_and_fire(&home, Some(&bin), "completely unrelated words")
        .await
        .expect("non-matching intake");
    assert_eq!(miss, 0, "a non-matching event must fire nothing");
}
