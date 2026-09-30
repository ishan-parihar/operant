//! Wave 0 — the pool compiler against REAL organism manifests.
//!
//! This file exists because the shipped `PoolManifest` was green in CI and
//! inert in production: it was tested against a hand-written fixture with
//! dotted verb service names, a shape no real organism pool has. Every
//! `.yaml` under `tests/fixtures/` is a byte-for-byte copy of a real
//! `~/.hermes/organism/**/_org.yaml` or `_pool.yaml` (see
//! `tests/fixtures/README.md` for the source path and harvest date of each).
//!
//! **If a fixture stops parsing, the bug is in the parser, not the file.**

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use operant_harness::pool::ParsedManifest;
use operant_harness::pool::{
    AccessMode, ManifestDialect, WritePolicy, check_registry_freshness, compile_organism,
    compile_organism_in, load_and_compile, load_and_compile_str, parse_legacy, parse_organism,
    require_fresh_registry,
};
// Deliberately NOT added to the crate root re-exports: importing from the
// module keeps this wave inside pool.rs / discovery.rs / tests/, with no
// edit to lib.rs.
use operant_harness::discovery::{default_organism_root, list as list_pools, route as route_pool};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// Every real fixture, with the pool name each one declares. The name is
/// asserted so a fixture cannot be silently swapped for a file that parses
/// but declares something else.
const REAL_FIXTURES: &[(&str, &str)] = &[
    // --- _org.yaml, canonical dialect ---------------------------------
    ("meta-governance__org.yaml", "meta-governance"),
    ("task-grid__org.yaml", "task-grid"),
    ("workforce-ops__org.yaml", "workforce-ops"),
    ("agent-fabric__org.yaml", "agent-fabric"),
    ("identity-core__org.yaml", "identity-core"),
    ("platform-infra__org.yaml", "platform-infra"),
    ("research-vault__org.yaml", "research-vault"),
    ("career-pipeline__org.yaml", "career-pipeline"),
    ("brand-integrating__org.yaml", "brand-integrating"),
    ("graveyard__org.yaml", "graveyard"),
    (
        "research-engine-20260907-retired__org.yaml",
        "research-engine",
    ),
    // --- added after the live-tree run surfaced 4 more shapes -----------
    ("alignment-engine__org.yaml", "alignment-engine"),
    ("forge__org.yaml", "forge"),
    ("contacts-dup-conflict__org.yaml", "contacts"),
    ("living-architecture__org.yaml", "living-architecture"),
    // --- _pool.yaml, legacy dialect -----------------------------------
    ("presentation-engine__pool.yaml", "presentation-engine"),
    ("financial-os__pool.yaml", "financial-os"),
    ("personal-os__pool.yaml", "personal-os"),
    ("scratch-test__pool.yaml", "scratch-test"),
    ("personal-brand-tech__pool.yaml", "personal-brand-tech"),
    ("content__pool.yaml", "content"),
    ("revenue-engine__pool.yaml", "revenue-engine"),
];

fn fixture_path(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("fixture {name} must be readable: {e}"))
}

// ---------------------------------------------------------------------------
// 1. THE REGRESSION THIS ENTIRE EXERCISE EXISTS FOR
// ---------------------------------------------------------------------------

/// Every real organism manifest parses and compiles.
///
/// The pre-fix struct failed all 18 of these: 11 with
/// `services_offered[0]: missing field 'name'` (hard serde error) and 7 by
/// parsing to an empty top-level `name` and then failing the compiler's
/// empty-name check. Both failure modes are asserted against below so a
/// regression in EITHER direction is caught.
#[test]
fn all_real_fixtures_parse_and_compile() {
    let mut failures: Vec<String> = Vec::new();
    for (file, expected_name) in REAL_FIXTURES {
        let raw = read_fixture(file);

        // Stage 1: parse under the real schema.
        let manifest = match parse_organism(&raw) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{file}: parse: {e}"));
                continue;
            }
        };
        // The identity block is nested under `pool:` — this is the exact
        // field the old struct read at the top level.
        if manifest.pool.name.trim() != *expected_name {
            failures.push(format!(
                "{file}: pool.name = {:?}, expected {expected_name:?}",
                manifest.pool.name
            ));
            continue;
        }

        // Stage 2: compile.
        let compiled = match compile_organism(&manifest, &WritePolicy::read_only()) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{file}: compile: {e}"));
                continue;
            }
        };
        if compiled.name != *expected_name {
            failures.push(format!(
                "{file}: compiled name = {:?}, expected {expected_name:?}",
                compiled.name
            ));
            continue;
        }
        // Every service carries an id, and claims are exactly those ids.
        for svc in &manifest.services_offered {
            if svc.id().trim().is_empty() {
                failures.push(format!("{file}: a services_offered entry has an empty id"));
            }
            if !compiled.claims.iter().any(|c| c == svc.id().trim()) {
                failures.push(format!("{file}: claim {} missing from output", svc.id()));
            }
        }
        // The family row is a well-formed architecture row.
        assert_eq!(compiled.family_row.source, "pool", "{file}");
        assert_eq!(
            compiled.family_row.kind.as_deref(),
            Some("pool.family"),
            "{file}"
        );
    }
    assert!(
        failures.is_empty(),
        "REAL MANIFEST REGRESSION — {} of {} fixtures failed:\n  {}",
        failures.len(),
        REAL_FIXTURES.len(),
        failures.join("\n  ")
    );
}

/// The same files through the public `load_and_compile` entry point, which
/// the CLI's `pool import` command calls. This is the end-to-end path that
/// was broken in production.
#[test]
fn real_fixtures_load_and_compile_from_disk() {
    for (file, expected_name) in REAL_FIXTURES {
        let path = fixture_path(file);
        let compiled =
            load_and_compile(&path).unwrap_or_else(|e| panic!("{file} must load and compile: {e}"));
        assert_eq!(&compiled.name, expected_name, "{file}");
        assert_eq!(compiled.dialect, ManifestDialect::Organism, "{file}");
    }
}

/// Contract fields survive compilation (AD-070: entry points are executable
/// verbatim). The old compiler dropped `entry_point` / `accepts` / `returns`
/// entirely, so a service claim was a name with no way to call it.
#[test]
fn real_fixtures_carry_executable_contracts() {
    // identity-core offers `identity` with a folded multi-line entry_point.
    let raw = read_fixture("identity-core__org.yaml");
    let m = parse_organism(&raw).unwrap();
    let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
    let identity = c
        .service("identity")
        .expect("identity-core offers `identity`");
    let contract = identity.contract.as_deref().expect("entry_point present");
    assert!(
        contract.contains("CLI: identity"),
        "contract = {contract:?}"
    );
    assert_eq!(identity.accepts, vec!["section"]);
    assert_eq!(identity.returns, vec!["identity_doc"]);

    // presentation-engine is the ONLY live legacy file and uses `interface:`
    // with no `entry_point:` at all. The compiler must fall back to it.
    let raw = read_fixture("presentation-engine__pool.yaml");
    let m = parse_organism(&raw).unwrap();
    let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
    let render = c.service("pe-render").expect("pe-render is offered");
    assert_eq!(
        render.contract.as_deref(),
        Some("pe render <type> <spec.json> <output.html> --engine <name> --quality showcase")
    );
    assert_eq!(c.claims.len(), 4);
    // the whole genome block survived parsing
    assert!(m.genome.contains_key("produce"));
    assert!(m.genome.contains_key("operate"));
    assert!(m.genome.contains_key("observe"));
    assert!(m.genome.contains_key("align"));
}

/// The `relationships` block (AD-062) the plan brief said the plan never
/// mentioned. Three real files carry it.
#[test]
fn real_relationships_block_is_modelled() {
    let m = parse_organism(&read_fixture("task-grid__org.yaml")).unwrap();
    assert_eq!(m.relationships.depends_on, vec!["platform-infra"]);
    assert_eq!(m.relationships.loops, vec!["platform-infra"]);

    let m = parse_organism(&read_fixture("platform-infra__org.yaml")).unwrap();
    assert_eq!(m.relationships.depends_on, vec!["task-grid"]);

    let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
    assert_eq!(
        c.family_row.config["relationships"]["depends_on"][0],
        "task-grid"
    );
}

/// The two declared sub-system dialects both parse, including the file that
/// carries BOTH shapes in two different forms.
#[test]
fn both_sub_system_dialects_parse() {
    // Bare-string list: the _org.yaml dialect.
    let m = parse_organism(&read_fixture("platform-infra__org.yaml")).unwrap();
    let names: Vec<&str> = m.sub_systems.iter().map(|s| s.name()).collect();
    assert!(names.contains(&"vps-operations"), "names = {names:?}");
    assert!(names.contains(&"browser-infrastructure"));

    // Map list: the _pool.yaml dialect, in the same file as the bare-string
    // `pooled_sub_systems`. presentation-engine carries both.
    let m = parse_organism(&read_fixture("presentation-engine__pool.yaml")).unwrap();
    assert_eq!(m.pooled_sub_systems.len(), 1);
    assert_eq!(m.pooled_sub_systems[0].name(), "archify");
    assert!(m.pooled_sub_systems[0].is_pooled());
    assert_eq!(m.pooled_sub_systems[0].target(), Some("hermes-agent-infra"));
    assert_eq!(m.sub_systems.len(), 1);
    assert_eq!(m.sub_systems[0].name(), "archify");
    assert_eq!(m.sub_systems[0].target(), Some("~/.hermes/skills/archify"));
}

// ---------------------------------------------------------------------------
// 2. The shapes the LIVE-TREE run discovered (fixtures added in response)
// ---------------------------------------------------------------------------

/// `alignment-engine` is the file that broke the first parser pass twice:
/// its `services_offered` is a bare-string list, AND its `services_consumed`
/// is a `{service: provider}` single-entry-map list. Neither shape exists
/// anywhere in the synthetic fixtures.
#[test]
fn bare_string_services_and_pair_maps_both_parse() {
    let m = parse_organism(&read_fixture("alignment-engine__org.yaml")).unwrap();

    // services_offered: `- strategy_bridge_sync` (a bare string)
    let offered: Vec<&str> = m.services_offered.iter().map(|e| e.id()).collect();
    assert_eq!(
        offered,
        vec![
            "strategy_bridge_sync",
            "research_protocol_directives",
            "compass_alignment_feed"
        ]
    );

    // services_consumed: `- feedback_engine: campaigns/feedback-engine`
    // (a single-entry map), and these must resolve to a real provider —
    // not to an empty edge, which is what `#[serde(untagged)]` produced.
    assert_eq!(m.services_consumed.len(), 4);
    let providers: Vec<&str> = m.services_consumed.iter().map(|e| e.provider()).collect();
    assert_eq!(
        providers,
        vec![
            "campaigns/feedback-engine",
            "campaigns/research-protocols",
            "compass/alignment",
            "compass/grounding",
        ]
    );

    // and they become real `requires` claims
    let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
    assert_eq!(
        c.claims,
        vec![
            "strategy_bridge_sync",
            "research_protocol_directives",
            "compass_alignment_feed"
        ]
    );
    let requires: Vec<String> = c.family_row.config["requires"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        requires.contains(&"feedback_engine".to_string()),
        "requires = {requires:?}"
    );
}

/// `forge` declares `services_consumed` as bare strings (`- trajectory`).
#[test]
fn bare_string_consumed_service_parses() {
    let m = parse_organism(&read_fixture("forge__org.yaml")).unwrap();
    let providers: Vec<&str> = m.services_consumed.iter().map(|e| e.provider()).collect();
    assert!(
        providers.contains(&"trajectory"),
        "providers = {providers:?}"
    );
    compile_organism(&m, &WritePolicy::read_only()).unwrap();
}

/// `living-architecture` is one of the three real files whose `returns` is
/// a bare PROSE STRING where the schema expects a list. It must widen into
/// a one-element list, not be dropped.
#[test]
fn scalar_returns_widens_to_a_list() {
    let m = parse_organism(&read_fixture("living-architecture__org.yaml")).unwrap();
    let svc = m
        .services_offered
        .iter()
        .find(|e| e.id() == "living-arch-cli")
        .expect("living-arch-cli is offered");
    let returns = svc.service().returns.to_vec();
    assert_eq!(returns.len(), 1, "prose returns must survive as one value");
    assert!(
        returns[0].contains("compiled architecture snapshot"),
        "returns = {returns:?}"
    );
}

/// `contacts` declares `directories.core/.required` twice with DIFFERENT
/// values (`false` then `true`). PyYAML — the organism's own reader — keeps
/// the last, so we must too, or operant would disagree with the organism
/// about what the same file means.
#[test]
fn conflicting_duplicate_keys_resolve_last_wins_like_pyyaml() {
    let raw = read_fixture("contacts-dup-conflict__org.yaml");

    // the default parse loads, and takes the LAST value (PyYAML semantics)
    let m = parse_organism(&raw).unwrap();
    let core = m
        .directories
        .get("core/")
        .expect("directories.core/ is declared");
    let required = core
        .get("required")
        .and_then(serde_yaml::Value::as_bool)
        .expect("directories.core/.required is a bool");
    assert!(
        required,
        "last-wins: required must be `true`, got {required}"
    );

    // and the strict parse reports the divergence instead of hiding it
    let err = operant_harness::pool::parse_organism_strict(&raw).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("conflicting duplicate key"), "got: {msg}");
    assert!(msg.contains("required"), "got: {msg}");
}

/// `scratch-test` is a documented template: `services_offered:` followed only
/// by comments. YAML reads that as null. It means "no services".
///
/// Note this is the SAME file that covers the comment-only case in the
/// inventory above — there is no second real file in the tree whose service
/// lists are null, so a second copy would have been a duplicate of the
/// first. Confirmed by scanning every `_org.yaml`/`_pool.yaml` in the tree:
/// the only null-service manifests are scratch-test, scratch-test2 and
/// scratch-test3, and 2 and 3 differ only in their own name/AD prefix.
#[test]
fn null_service_list_means_no_services() {
    let m = parse_organism(&read_fixture("scratch-test__pool.yaml")).unwrap();
    assert!(m.services_offered.is_empty());
    assert!(m.services_consumed.is_empty());
    let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
    assert_eq!(c.name, "scratch-test");
    assert!(c.claims.is_empty());
}

// ---------------------------------------------------------------------------
// 3. Back-compat: the legacy v1 schema must keep working
// ---------------------------------------------------------------------------

/// A synthetic legacy `_pool.yaml` in the OLD v1 schema still parses and
/// compiles. This is the back-compat proof: `operant-core` and the CLI call
/// `compile`/`load_and_compile` against this shape, and other agents are
/// editing those crates right now.
#[test]
fn legacy_v1_schema_still_parses_and_compiles() {
    let legacy = r#"
name: relationship-intel
services_offered:
  - name: query.contacts
    description: Find contacts by name/email
  - name: search.history
    description: Search communication history
services_consumed:
  - name: auth.identity
    description: Identity provider
pooled_sub_systems:
  - name: contacts
    path: ~/.hermes/systems/relationship-intel/contacts
  - name: history
    path: ~/.hermes/systems/relationship-intel/history
"#;
    // parses under the legacy struct
    let m: operant_harness::PoolManifest = serde_yaml::from_str(legacy).unwrap();
    // and under the autodetecting entry point, as LegacyV1 (not Organism)
    let parsed = ParsedManifest::from_str(legacy).unwrap();
    assert_eq!(parsed.dialect(), ManifestDialect::LegacyV1);

    // compiles through the ORIGINAL public function
    let c = operant_harness::compile_pool(&m).unwrap();
    assert_eq!(c.name, "relationship-intel");
    assert_eq!(c.claims, vec!["query.contacts", "search.history"]);
    assert_eq!(c.bundle_rows.len(), 2);
    assert_eq!(c.dialect, ManifestDialect::LegacyV1);

    // and the v1 write-verb rejection is unchanged
    let bad: operant_harness::PoolManifest = serde_yaml::from_str(
        "name: bad\nservices_offered:\n  - name: delete.contacts\n    description: x\n",
    )
    .unwrap();
    let err = operant_harness::compile_pool(&bad).unwrap_err();
    assert!(err.to_string().contains("non-read-only"));

    // parse_legacy is still exported and works standalone
    assert_eq!(parse_legacy(legacy).unwrap().name, "relationship-intel");
}

/// A v1 manifest on disk still loads through `load_and_compile`, and the
/// dialect is reported honestly.
#[test]
fn legacy_v1_on_disk_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("_pool.yaml");
    std::fs::write(
        &path,
        "name: legacy-on-disk\nservices_offered:\n  - name: get.thing\n    description: d\n",
    )
    .unwrap();
    let c = load_and_compile(&path).unwrap();
    assert_eq!(c.name, "legacy-on-disk");
    assert_eq!(c.dialect, ManifestDialect::LegacyV1);
    assert_eq!(c.claims, vec!["get.thing"]);
}

/// Autodetection must not silently swallow a real manifest as v1: a file
/// with a `pool:` block is always the real dialect.
#[test]
fn autodetect_prefers_the_real_schema_when_pool_block_present() {
    let raw = read_fixture("task-grid__org.yaml");
    let parsed = ParsedManifest::from_str(&raw).unwrap();
    assert_eq!(parsed.dialect(), ManifestDialect::Organism);
}

// ---------------------------------------------------------------------------
// 4. Read-only is a property of the access mode
// ---------------------------------------------------------------------------

/// THE WRITE-ACCESS GATE. Mutation-proven: see the report — removing the
/// `AccessMode::Unknown` check from `compile_organism_in` makes this test
/// fail.
///
/// A consumer edge whose access mode the organism has not classified cannot
/// compile. It must be classified in the organism, or granted through a
/// `WritePolicy` carrying an explicit approval token. A manifest cannot
/// grant itself the access it wants.
#[test]
fn unclassified_access_mode_is_rejected() {
    let raw = r#"
pool:
  name: gate-probe
  type: system
  category: test
  ad_scope: TEST-AD-001
  parent: null
services_consumed:
  - id: sneaky
    provider: elsewhere
    purpose: unclassified write-ish access
    access: shellout
"#;
    let m = parse_organism(raw).unwrap();

    // 1. default policy (empty) => REJECTED
    let err = compile_organism(&m, &WritePolicy::read_only())
        .expect_err("unclassified access must be rejected by default");
    let msg = err.to_string();
    assert!(
        msg.contains("unclassified access mode"),
        "error should name the cause, got: {msg}"
    );
    assert!(
        msg.contains("shellout"),
        "error should quote the mode, got: {msg}"
    );
    assert!(
        msg.contains("gate-probe"),
        "error should name the pool, got: {msg}"
    );

    // 2. a policy that grants SOME OTHER capability is still rejected —
    //    the grant must be specific, not a blanket.
    let other = WritePolicy::with_approval(vec!["access:unrelated".to_string()], "AD-1").unwrap();
    assert!(compile_organism(&m, &other).is_err());

    // 3. a policy with a BLANK approval token cannot be constructed at all
    assert!(WritePolicy::with_approval(vec!["access:sneaky".to_string()], "   ").is_none());

    // 4. the specific grant passes
    let granted = WritePolicy::with_approval(vec!["access:sneaky".to_string()], "TEST-AD-001")
        .expect("a non-empty token is a valid approval");
    let ok = compile_organism(&m, &granted).unwrap();
    assert_eq!(ok.name, "gate-probe");
}

/// Every access mode the REAL organism tree declares must compile with the
/// default empty policy. This is the other half of the gate: it must not be
/// so strict that real pools stop working.
#[test]
fn every_real_declared_access_mode_compiles_under_default_policy() {
    // The full set measured across all 126 physical manifests, including the
    // compound `symlink+cli` that 4 real consumed edges use.
    for mode in ["cli", "symlink", "import", "file", "symlink+cli"] {
        let raw = format!(
            "pool:\n  name: mode-probe\n  type: system\nservices_consumed:\n  \
             - id: thing\n    provider: elsewhere\n    purpose: p\n    access: {mode}\n"
        );
        let m = parse_organism(&raw).unwrap();
        compile_organism(&m, &WritePolicy::read_only())
            .unwrap_or_else(|e| panic!("declared access mode `{mode}` must compile: {e}"));
    }
    // Absent access is the least-privileged mode and must compile.
    let m = parse_organism(
        "pool:\n  name: mode-probe\n  type: system\nservices_consumed:\n  \
         - id: thing\n    provider: elsewhere\n    purpose: p\n",
    )
    .unwrap();
    compile_organism(&m, &WritePolicy::read_only()).unwrap();
}

/// The `WritePolicy` itself is empty by default and cannot be widened from
/// YAML. There is no serde `Deserialize` on it, so no manifest field can
/// populate it.
#[test]
fn write_policy_is_empty_by_default_and_not_deserializable() {
    let d = WritePolicy::read_only();
    assert!(d.capabilities().is_empty());
    assert!(d.approved_by().is_none());
    assert!(!d.allows("access:anything"));
    assert!(!d.allows(""));
}

/// `READ_ONLY_VERBS` is retained, and still fires for a manifest that
/// actually uses dotted verb-style names — which no real organism file does
/// (measured: 0 of 126).
#[test]
fn read_only_verbs_still_gate_dotted_verb_names() {
    assert!(!operant_harness::READ_ONLY_VERBS.is_empty());
    let bad = parse_organism(
        "pool:\n  name: v1ish\n  type: system\nservices_offered:\n  - id: delete.contacts\n    entry_point: x\n",
    )
    .unwrap();
    let err = compile_organism(&bad, &WritePolicy::read_only()).unwrap_err();
    assert!(err.to_string().contains("read-only verb"), "got {err}");

    // a real slug passes
    for slug in [
        "axe-feedback",
        "knowledge-graph",
        "ad-rg-lifecycle",
        "identity",
    ] {
        let ok = parse_organism(&format!(
            "pool:\n  name: slug\n  type: system\nservices_offered:\n  - id: {slug}\n    entry_point: x\n"
        ))
        .unwrap();
        compile_organism(&ok, &WritePolicy::read_only())
            .unwrap_or_else(|e| panic!("real service id `{slug}` must compile: {e}"));
    }
}

/// Every access mode the organism declares is classified as read-only, and
/// the one non-read-only variant is unreachable from a manifest.
#[test]
fn access_mode_read_only_classification_is_fail_closed() {
    for mode in ["cli", "symlink", "import", "file", "symlink+cli"] {
        assert!(AccessMode::parse(Some(mode)).is_read_only(), "{mode}");
    }
    // unclassified is held read-only, not promoted to write
    assert!(AccessMode::parse(Some("shellout")).is_read_only());
    assert!(AccessMode::parse(None).is_read_only());
    // and `Write` is never produced by parsing
    assert_ne!(AccessMode::parse(Some("write")), AccessMode::Write);
    assert_ne!(AccessMode::parse(Some("rw")), AccessMode::Write);
}

// ---------------------------------------------------------------------------
// 5. Registry staleness (M2 / AD-061 parity)
// ---------------------------------------------------------------------------

/// THE REGISTRY-STALENESS GATE. Mutation-proven: see the report — removing
/// the `require_fresh_registry` refusal makes this test fail.
///
/// M2 says the registry is a derived build artifact, so it cannot lie. In
/// the organism a stale registry is a WARN; in operant the registry is what
/// the kernel routes on, so a stale one is a REJECT.
#[test]
fn stale_registry_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("architecture-registry.yaml");
    let manifest = dir.path().join("_org.yaml");
    std::fs::write(&manifest, b"pool:\n  name: x\n  type: system\n").unwrap();

    // (a) registry missing entirely => rejected
    let err = require_fresh_registry(&registry, std::slice::from_ref(&manifest))
        .expect_err("a missing registry must be rejected");
    assert!(err.to_string().contains("unreadable"), "got: {err}");

    // (b) registry older than the manifest => rejected
    std::fs::write(&registry, b"registry: architecture-registry\n").unwrap();
    // push the registry's mtime backwards past the manifest's
    let manifest_time = std::fs::metadata(&manifest).unwrap().modified().unwrap();
    let old = manifest_time - Duration::from_secs(60);
    set_mtime(&registry, old);
    let freshness = check_registry_freshness(&registry, std::slice::from_ref(&manifest)).unwrap();
    assert!(
        !freshness.fresh,
        "registry older than manifest must be stale"
    );
    let reason = freshness.reason.clone().unwrap();
    assert!(
        reason.contains("AD-061"),
        "reason should cite AD-061: {reason}"
    );

    let err = require_fresh_registry(&registry, std::slice::from_ref(&manifest))
        .expect_err("a stale registry must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("registry staleness"), "got: {msg}");
    assert!(msg.contains("AD-061"), "got: {msg}");

    // (c) re-emitting the registry (mtime now newer) => accepted
    set_mtime(&registry, SystemTime::now() + Duration::from_secs(60));
    let freshness = check_registry_freshness(&registry, std::slice::from_ref(&manifest)).unwrap();
    assert!(freshness.fresh, "re-emitted registry must be fresh");
    assert!(freshness.reason.is_none());
    require_fresh_registry(&registry, std::slice::from_ref(&manifest))
        .expect("a fresh registry must be accepted");
}

fn set_mtime(path: &Path, time: SystemTime) {
    let f = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime");
    f.set_modified(time).expect("set mtime");
}

// ---------------------------------------------------------------------------
// 6. Bundling is symlink-based
// ---------------------------------------------------------------------------

/// AD-060: pooling is expressed as exact-name symlinks inside
/// `sub-systems/`. One bundle row per symlink target found on disk — not
/// per declared field, and not a hardcoded `read_only: true`.
#[test]
fn bundles_come_from_symlinks_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let pool = dir.path().join("foundations").join("demo");
    std::fs::create_dir_all(pool.join("sub-systems")).unwrap();

    // native organ: a real directory
    std::fs::create_dir_all(pool.join("sub-systems/native")).unwrap();
    // pooled organ: a symlink into a foundation (AD-060)
    let foundation = dir.path().join("foundations").join("other");
    std::fs::create_dir_all(&foundation).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&foundation, pool.join("sub-systems/pooled")).unwrap();

    // A manifest that declares NOTHING about sub-systems — the old compiler
    // would emit zero bundle rows here, because it only read the
    // `pooled_sub_systems` field.
    let m = parse_organism("pool:\n  name: demo\n  type: system\n").unwrap();
    let c = compile_organism_in(&m, &WritePolicy::read_only(), Some(&pool)).unwrap();

    let names: Vec<&str> = c.sub_systems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["native", "pooled"],
        "one row per sub-systems/ entry"
    );

    let pooled = c.sub_systems.iter().find(|s| s.name == "pooled").unwrap();
    assert!(pooled.is_symlink, "provenance: pooled entry IS a symlink");
    let native = c.sub_systems.iter().find(|s| s.name == "native").unwrap();
    assert!(
        !native.is_symlink,
        "provenance: native entry is NOT a symlink"
    );

    // read_only is no longer a bare hardcoded literal: the row carries why
    let row = c
        .bundle_rows
        .iter()
        .find(|r| r.id == "pool.demo.pooled")
        .unwrap();
    assert_eq!(row.config["read_only"], serde_json::json!(true));
    assert_eq!(row.config["symlink"], serde_json::json!(true));
    assert!(
        row.config["read_only_reason"]
            .as_str()
            .unwrap()
            .contains("ad-060"),
        "read_only must carry provenance, got {}",
        row.config["read_only_reason"]
    );
}

/// `.gitkeep` and dot-entries are not organs.
#[test]
fn sub_system_walk_skips_dotfiles() {
    let dir = tempfile::tempdir().unwrap();
    let pool = dir.path().join("p");
    std::fs::create_dir_all(pool.join("sub-systems")).unwrap();
    std::fs::write(pool.join("sub-systems/.gitkeep"), "").unwrap();
    std::fs::create_dir_all(pool.join("sub-systems/_private")).unwrap();
    std::fs::create_dir_all(pool.join("sub-systems/real")).unwrap();

    let m = parse_organism("pool:\n  name: p\n  type: system\n").unwrap();
    let c = compile_organism_in(&m, &WritePolicy::read_only(), Some(&pool)).unwrap();
    let names: Vec<&str> = c.sub_systems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["real"]);
}

// ---------------------------------------------------------------------------
// 7. Discovery: route + list
// ---------------------------------------------------------------------------

fn make_pool(root: &Path, stratum: &str, name: &str, parent: Option<&str>) -> PathBuf {
    let dir = root.join(stratum).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let parent_line = match parent {
        Some(p) => format!("  parent: {p}\n"),
        None => "  parent: null\n".to_string(),
    };
    std::fs::write(
        dir.join("_org.yaml"),
        format!(
            "pool:\n  name: {name}\n  type: system\n  category: test\n{parent_line}  \
             ad_scope: TEST-AD-001\n  role: test\n  domain: test\n"
        ),
    )
    .unwrap();
    dir
}

/// `route` resolves a deep path inside a pool to the pool that owns it.
#[test]
fn route_resolves_a_deep_path_to_its_owning_pool() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let pool = make_pool(root, "foundations", "identity-core", None);
    let deep = pool.join("sub-systems").join("professional").join("data");
    std::fs::create_dir_all(&deep).unwrap();

    let found = route_pool(&deep).unwrap();
    assert_eq!(found.name, "identity-core");
    assert_eq!(found.dir, std::fs::canonicalize(&pool).unwrap());
    assert_eq!(found.stratum.as_deref(), Some("foundations"));
    assert_eq!(found.manifest.file_name().unwrap(), "_org.yaml");
    assert!(found.parent.is_none());
}

#[test]
fn route_prefers_org_yaml_over_legacy_pool_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let pool = make_pool(root, "foundations", "both-files", None);
    // presentation-engine is the one real pool carrying BOTH; so is this one
    std::fs::write(
        pool.join("_pool.yaml"),
        "pool:\n  name: legacy-name\n  type: system\n",
    )
    .unwrap();
    let found = route_pool(&pool).unwrap();
    assert_eq!(
        found.name, "both-files",
        "_org.yaml wins, matching find_pool_config"
    );
    assert_eq!(found.manifest.file_name().unwrap(), "_org.yaml");
}

#[test]
fn route_falls_back_to_legacy_pool_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let pool = dir.path().join("graveyard").join("legacy-only");
    std::fs::create_dir_all(&pool).unwrap();
    std::fs::write(
        pool.join("_pool.yaml"),
        "pool:\n  name: legacy-only\n  type: system\n",
    )
    .unwrap();
    let found = route_pool(&pool).unwrap();
    assert_eq!(found.name, "legacy-only");
    assert_eq!(found.manifest.file_name().unwrap(), "_pool.yaml");
}

#[test]
fn route_reports_the_real_cause_when_nothing_owns_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let orphan = dir.path().join("nowhere");
    std::fs::create_dir_all(&orphan).unwrap();
    let err = route_pool(&orphan).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("no pool manifest found"), "got: {msg}");
    assert!(
        msg.contains("_org.yaml") && msg.contains("_pool.yaml"),
        "got: {msg}"
    );
}

#[test]
fn route_through_a_pooled_symlink_lands_on_the_foundation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let foundation = make_pool(root, "foundations", "content-campaigns", None);
    let graduate = make_pool(root, "ventures", "brand-integrating", None);
    // AD-060: the graduate pools the foundation as an exact-name symlink
    #[cfg(unix)]
    {
        std::fs::create_dir_all(graduate.join("sub-systems")).unwrap();
        std::os::unix::fs::symlink(
            &foundation,
            graduate.join("sub-systems").join("content-campaigns"),
        )
        .unwrap();
    }

    // Routing through the symlink must land on the FOUNDATION, which owns
    // the genome — not on the graduate that borrowed it.
    let via_symlink = graduate.join("sub-systems").join("content-campaigns");
    let found = route_pool(&via_symlink).unwrap();
    assert_eq!(found.name, "content-campaigns");
    assert_eq!(found.dir, std::fs::canonicalize(&foundation).unwrap());

    // and the graduate itself is still routable
    assert_eq!(route_pool(&graduate).unwrap().name, "brand-integrating");
}

/// `list` enumerates pools across strata and recurses into native
/// sub-systems, but does NOT fan out through pooled symlinks.
#[test]
fn list_enumerates_pools_and_native_sub_systems_without_symlink_fanout() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_pool(root, "cortex", "meta-governance", None);
    let identity = make_pool(root, "foundations", "identity-core", None);
    let graduate = make_pool(root, "ventures", "brand-integrating", None);

    // a native sub-system organ
    let organ = identity.join("sub-systems").join("professional");
    std::fs::create_dir_all(&organ).unwrap();
    std::fs::write(
        organ.join("_org.yaml"),
        "pool:\n  name: professional\n  type: sub-system\n  parent: identity-core\n  category: identity\n",
    )
    .unwrap();

    // a pooled symlink from the graduate into the foundation
    #[cfg(unix)]
    {
        std::fs::create_dir_all(graduate.join("sub-systems")).unwrap();
        std::os::unix::fs::symlink(
            &identity,
            graduate.join("sub-systems").join("identity-core"),
        )
        .unwrap();
    }

    let pools = list_pools(root).unwrap();
    let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();

    // every real directory with a manifest is found, including the organ
    for expected in [
        "meta-governance",
        "identity-core",
        "brand-integrating",
        "professional",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    // the symlink target is NOT duplicated under the graduate
    assert_eq!(
        names.iter().filter(|n| **n == "identity-core").count(),
        1,
        "a pooled symlink must not duplicate the foundation: {names:?}"
    );
    // sub-system organs record their parent
    let prof = pools.iter().find(|p| p.name == "professional").unwrap();
    assert_eq!(prof.parent.as_deref(), Some("identity-core"));
    assert_eq!(prof.stratum.as_deref(), Some("foundations"));
}

#[test]
fn list_reports_a_missing_root_rather_than_returning_empty() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-organism");
    let err = list_pools(&missing).unwrap_err();
    assert!(err.to_string().contains("not a directory"), "got: {err}");
}

/// The default root is `~/.hermes/organism` and is derived from `$HOME`.
#[test]
fn default_organism_root_is_under_home() {
    let root = default_organism_root().expect("$HOME is set in CI");
    assert!(root.ends_with(".hermes/organism"), "got {}", root.display());
    let home = std::env::var("HOME").unwrap();
    assert!(root.starts_with(home));
}

// ---------------------------------------------------------------------------
// 8. Cross-check against the LIVE organism tree when it is present
// ---------------------------------------------------------------------------

/// If `~/.hermes/organism` exists on this machine, run the real regression
/// over the WHOLE live tree, not just the fixtures. Skips cleanly when it
/// does not, so the suite stays green on a machine without an organism.
#[test]
fn live_organism_tree_compiles_when_present() {
    let Some(root) = default_organism_root() else {
        eprintln!("skipped: no $HOME");
        return;
    };
    if !root.is_dir() {
        eprintln!("skipped: {} not present", root.display());
        return;
    }
    // Every manifest at or under the root, recursively, deduped by realpath.
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut manifests: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(md) = std::fs::metadata(&p) else {
                continue;
            };
            if md.is_dir() {
                // do not follow symlinks: the graduates' symlink-fans would
                // walk the same real file thousands of times
                if p.is_symlink() {
                    continue;
                }
                stack.push(p);
            } else if p.is_file() {
                let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if n != "_org.yaml" && n != "_pool.yaml" {
                    continue;
                }
                let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                manifests.push(p);
            }
        }
    }
    manifests.sort();

    let mut parse_fail: Vec<String> = Vec::new();
    let mut compile_fail: Vec<String> = Vec::new();
    let mut ok = 0usize;
    for path in &manifests {
        let raw = std::fs::read_to_string(path).unwrap();
        match load_and_compile_str(&raw, path) {
            Ok(_) => ok += 1,
            Err(e) => {
                // the root organism/_org.yaml is ORG-GLOBAL config (teams,
                // structure) and is not a pool manifest at all
                if path.file_name().and_then(|n| n.to_str()) == Some("_org.yaml")
                    && path.parent() == Some(root.as_path())
                {
                    continue;
                }
                let msg = e.to_string();
                if msg.contains("yaml parse") {
                    parse_fail.push(format!("{}: {msg}", path.display()));
                } else {
                    compile_fail.push(format!("{}: {msg}", path.display()));
                }
            }
        }
    }
    eprintln!(
        "live organism: {ok}/{} manifests compiled ({} parse-fail, {} compile-fail)",
        manifests.len(),
        parse_fail.len(),
        compile_fail.len()
    );
    assert!(
        parse_fail.is_empty(),
        "LIVE REGRESSION — {} real manifests fail to PARSE:\n  {}",
        parse_fail.len(),
        parse_fail.join("\n  ")
    );
    assert!(
        compile_fail.is_empty(),
        "LIVE REGRESSION — {} real manifests fail to COMPILE:\n  {}",
        compile_fail.len(),
        compile_fail.join("\n  ")
    );
    assert!(ok > 0, "no live manifests were found — the walk is broken");
}
