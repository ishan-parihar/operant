//! Authority-filtered tool availability — §2.4.
//!
//! Spec: `docs/ORG-AUTHORITY-ARCHITECTURE.md` §2.1, §2.4, §2.5.
//!
//! ## Why this file exists separately from the board-side authority tests
//!
//! `authority.rs`'s own tests prove the lattice and the grant table are
//! internally correct. They do not prove the thing the owner actually asked
//! for: that authority constrains **what the model is shown**, not just what
//! the board accepts. A tool an actor may not use must be **absent from the
//! advertised list** — blocking execution alone is insufficient, because a tool
//! the model can see but cannot call costs a turn of confusion and a retry
//! every time it is reached for.
//!
//! ## The six properties, in order of how badly a regression would hurt
//!
//! 1. a non-crossing scope is offered its own department's tools **and records
//!    no grant row** (hierarchy is the primary grant; an inert grant row would
//!    misattribute authority the org chart already gave);
//! 2. a crossing scope is refused with no explicit grant;
//! 3. an explicit grant admits the crossing tool;
//! 4. an **expired** grant does not admit it;
//! 5. a grant to a **vacant/invalid seat is refused and not persisted**;
//! 6. a filtered-out tool is **absent from the advertised list**, not merely
//!    blocked at execution.
//!
//! Cycle detection in `reports_to` belongs to the hierarchy packet and is
//! deliberately **not** duplicated here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use operant_core::org::authority::{AuthorityScope, Grant, GrantDb, resolve_scope};
use operant_core::org::employee::Employee;
use operant_core::schema::ToolSchema;
use operant_core::tools::{
    AuthorityActor, OperantTool, SeatDirectory, ToolAuthority, ToolContext, ToolRegistry,
    ToolResult, issue_grant, tool_authority_check,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------- fixtures

/// A clock relative to now. Every timestamp in this file goes through here, so
/// the expiry tests are about the *comparison* rather than about wall-clock
/// luck. Must match [`operant_core::org::notice::rfc3339`]'s fixed millisecond
/// width, because that is what makes the lexicographic `<` chronological.
fn at(offset_secs: i64) -> String {
    operant_core::org::notice::rfc3339(chrono::Utc::now() + chrono::Duration::seconds(offset_secs))
}

/// Two departments and their two capabilities (§2.5's naming shape).
const PLATFORM: &str = "platform_infra";
const CONTENT: &str = "content";
const CAP_PLATFORM_TOOLING: &str = "platform_infra.tooling";
const CAP_CONTENT_READ: &str = "content.read";

/// The rung the departmental tools demand. §2.2 makes a department the rung
/// at which an actor acts inside a department, so the fixture's in-department
/// actors hold exactly that rung — narrow enough that the *department* rule is
/// what admits them, not the scope rule above it.
const DEPT_RUNG: AuthorityScope = AuthorityScope::Department;

/// An employee row. `skills` is populated so [`Employee::is_valid`] holds; the
/// invalid-seat test empties it deliberately.
fn employee(id: &str, department: Option<&str>) -> Employee {
    Employee {
        employee_id: id.to_string(),
        name: format!("{id} title"),
        role: "automator".to_string(),
        department: department.map(|d| d.to_string()),
        skills: vec!["audit".to_string()],
        agent_type: None,
        persona: None,
        status: "active".to_string(),
        reason: "org_authority_tools fixture".to_string(),
        created_at: "2026-09-30T00:00:00Z".to_string(),
        updated_at: "2026-09-30T00:00:00Z".to_string(),
    }
}

/// A [`SeatDirectory`] over an in-memory roster.
///
/// Real enough for the decision under test — the vacant-seat and validity rules
/// both read this — and it keeps the file free of a database, which is the
/// point: what is under test is the *decision*, not sqlite.
#[derive(Clone, Default)]
struct Roster {
    seats: HashMap<String, Employee>,
}

impl Roster {
    /// Add (or replace) a seat.
    fn with(mut self, emp: Employee) -> Self {
        self.seats.insert(emp.employee_id.clone(), emp);
        self
    }
}

impl SeatDirectory for Roster {
    fn employee(&self, employee_id: &str) -> Option<Employee> {
        self.seats.get(employee_id).cloned()
    }
}

/// A grants store on a temp file. The returned [`TempDir`] must stay alive for
/// the store to remain open, hence the binding rather than `drop`.
fn grant_store() -> (GrantDb, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = GrantDb::init(dir.path().join("operant_kanban.db")).expect("grant db init");
    (db, dir)
}

/// Three filled seats: the CEO, a `platform_infra` department head, and a
/// `content` employee.
fn three_seats() -> Roster {
    Roster::default()
        .with(employee("emp-ceo", None))
        .with(employee("emp-platform-head", Some(PLATFORM)))
        .with(employee("emp-content-writer", Some(CONTENT)))
}

/// An actor holding exactly the `Department` rung and sitting in `dept`.
///
/// `Department` (not the `Descendants` `resolve_scope(true, false)` returns) so
/// that the department rule — not the scope rule — is what admits these actors
/// to their own department's tools.
fn writer_in(dept: &str) -> AuthorityActor {
    AuthorityActor::new(
        format!("emp-{dept}-writer"),
        Some(dept.to_string()),
        DEPT_RUNG,
        Vec::new(),
    )
}

/// A department head: `resolve_scope(true, false)` = `Descendants`.
fn head_of(dept: &str) -> AuthorityActor {
    AuthorityActor::new(
        format!("emp-{dept}-head"),
        Some(dept.to_string()),
        resolve_scope(true, false),
        Vec::new(),
    )
}

/// The org lead: `resolve_scope(false, true)` = `Org`.
fn ceo() -> AuthorityActor {
    AuthorityActor::new("emp-ceo", None, resolve_scope(false, true), Vec::new())
}

/// A live, unexpired, `Department`-scoped grant over `capability` for `dept`.
fn live_grant(grantee: &str, capability: &str, dept: &str, expires_offset: i64) -> Grant {
    Grant::new(
        "emp-ceo",
        grantee,
        capability,
        DEPT_RUNG,
        Some(dept.to_string()),
        "org_authority_tools: fixture grant",
        Some(at(expires_offset)),
    )
}

// ------------------------------------------------------------------- tools

/// A tool whose only interesting field is its name.
struct NamedTool {
    name: &'static str,
}

#[async_trait]
impl OperantTool for NamedTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "authority-filtered tool fixture"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name,
            "authority-filtered tool fixture",
            json!({"type": "object", "properties": {}}),
        )
    }
    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::success("call_1", json!({"ok": true}))
    }
}

/// A registry carrying three tools:
///
/// - `datetime` — undeclared, i.e. global;
/// - `vps_reboot` — on `platform_infra`'s surface, gated by
///   `platform_infra.tooling`;
/// - `cms_publish` — on `content`'s surface, gated by `content.read`.
async fn registry() -> ToolRegistry {
    let reg = ToolRegistry::new(Duration::from_secs(5));
    for tool in [
        NamedTool { name: "datetime" },
        NamedTool { name: "vps_reboot" },
        NamedTool {
            name: "cms_publish",
        },
    ] {
        reg.register(tool).await.expect("register");
    }
    assert!(
        reg.set_tool_authority(
            "vps_reboot",
            ToolAuthority::in_department(PLATFORM, CAP_PLATFORM_TOOLING, DEPT_RUNG),
        )
        .await,
        "set_tool_authority must report the tool as registered"
    );
    assert!(
        reg.set_tool_authority(
            "cms_publish",
            ToolAuthority::in_department(CONTENT, CAP_CONTENT_READ, DEPT_RUNG),
        )
        .await
    );
    reg
}

/// The names in an advertised list, sorted, for stable assertions.
fn advertised_names(schemas: Vec<ToolSchema>) -> Vec<String> {
    let mut names: Vec<String> = schemas.into_iter().map(|s| s.name).collect();
    names.sort();
    names
}

// =====================================================================
// 1. Hierarchy is the primary grant: own department needs no grant, and
//    records no grant row.
// =====================================================================

/// §2.4: *"A `Department`-scoped agent is offered the department's tool
/// surface."* §2.1: hierarchy is sufficient inside a department.
///
/// The load-bearing half is the **grant table count**. An implementation that
/// lazily issued a grant to make the own-department path "uniform" would pass
/// every availability assertion and still be wrong.
#[tokio::test]
async fn non_crossing_scope_gets_its_own_departments_tools_and_records_no_grant() {
    let (grants, _dir) = grant_store();
    let reg = registry().await;
    let platform_writer = writer_in(PLATFORM);

    let advertised = advertised_names(reg.advertise_for(&platform_writer, &at(0)).await);

    // The department's own surface tool is offered…
    assert!(
        advertised.contains(&"vps_reboot".to_string()),
        "a department member must be offered its own department's tool surface; got {advertised:?}"
    );
    // …the global tool is offered…
    assert!(
        advertised.contains(&"datetime".to_string()),
        "an undeclared tool is global and must be offered to every actor; got {advertised:?}"
    );
    // …and the sibling department's tool is not.
    assert!(
        !advertised.contains(&"cms_publish".to_string()),
        "a sibling department's tool must not be on a non-crossing actor's list; got {advertised:?}"
    );
    assert_eq!(
        advertised,
        vec!["datetime".to_string(), "vps_reboot".to_string()],
        "a non-crossing actor sees exactly its own department plus the globals"
    );

    // THE ASSERTION THAT MATTERS: no grant row was written.
    assert_eq!(
        grants.count().expect("count"),
        0,
        "a non-crossing scope must record NO grant row — hierarchy already \
         answered the question, and a row here would misattribute the authority"
    );

    // The verdict says so in words, so an operator reading a log line learns
    // *why* no grant exists rather than assuming one is still owed.
    let check = tool_authority_check(
        reg.tool_authority("vps_reboot").await.as_ref(),
        &platform_writer,
        "vps_reboot",
        &at(0),
    );
    assert!(check.is_allowed(), "own-department tool must be allowed");
    assert_eq!(
        check.required_scope, DEPT_RUNG,
        "an allow must still report the rung the tool demands"
    );
    assert!(
        check.reason.contains("the primary grant"),
        "the allow reason must name the primary grant; got: {}",
        check.reason
    );
    assert!(
        check
            .reason
            .contains("no capability grant is required or recorded"),
        "the allow reason must state that no grant row exists; got: {}",
        check.reason
    );

    // Re-query: still no row. A lazily-written row would show up here.
    reg.advertise_for(&platform_writer, &at(0)).await;
    reg.advertise_for(&platform_writer, &at(0)).await;
    assert_eq!(
        grants.count().expect("count"),
        0,
        "repeated queries must still not record a grant row"
    );

    // A department head (`Descendants`, strictly wider) is admitted by the same
    // own-department rule and also records nothing.
    let platform_head = head_of(PLATFORM);
    assert!(
        reg.is_available_for(&platform_head, "vps_reboot", &at(0))
            .await,
        "a wider-scoped actor in the same department keeps its own-department reach"
    );
    assert_eq!(grants.count().expect("count"), 0);
}

// =====================================================================
// 2. Crossing a department is refused without an explicit grant.
// =====================================================================

/// §2.1: authority is **never inferred** from a grant that does not exist.
#[tokio::test]
async fn crossing_scope_is_refused_without_an_explicit_grant() {
    let reg = registry().await;
    let content_writer = writer_in(CONTENT);

    // Sanity: this actor clears every earlier rule, so the only thing standing
    // between it and `vps_reboot` is the missing grant. Without this the test
    // could pass for the wrong reason.
    assert!(
        reg.is_available_for(&content_writer, "cms_publish", &at(0))
            .await,
        "the fixture actor must reach its OWN department's tool, else the \
         denial below would prove nothing"
    );

    assert!(
        !reg.is_available_for(&content_writer, "vps_reboot", &at(0))
            .await,
        "a content employee must not reach platform_infra's tool without a grant"
    );

    // The denial names the capability a grantor would have to name, so the
    // operator can act. A bare `false` proves nothing about why.
    let check = tool_authority_check(
        reg.tool_authority("vps_reboot").await.as_ref(),
        &content_writer,
        "vps_reboot",
        &at(0),
    );
    assert!(!check.is_allowed());
    assert_eq!(
        check.required_scope,
        AuthorityScope::Org,
        "a crossing denial requires the Org rung — §2.1's boundary"
    );
    assert!(
        check.reason.contains(CAP_PLATFORM_TOOLING),
        "the denial must name the capability to grant; got: {}",
        check.reason
    );
    assert!(
        check.reason.contains("no grant covers"),
        "the denial must say the grant does not exist, not that the scope was \
         too narrow; got: {}",
        check.reason
    );

    // Matching is exact, not departmental: a grant naming a different
    // capability does not admit this tool.
    let wrong_capability = AuthorityActor::new(
        &content_writer.employee_id,
        Some(CONTENT.to_string()),
        DEPT_RUNG,
        vec![live_grant(
            &content_writer.employee_id,
            CAP_CONTENT_READ,
            PLATFORM,
            3600,
        )],
    );
    assert!(
        !reg.is_available_for(&wrong_capability, "vps_reboot", &at(0))
            .await,
        "a grant naming a different capability must not admit this tool"
    );

    // …and a grant aimed at a different department does not either.
    let wrong_dept = AuthorityActor::new(
        &content_writer.employee_id,
        Some(CONTENT.to_string()),
        DEPT_RUNG,
        vec![live_grant(
            &content_writer.employee_id,
            CAP_PLATFORM_TOOLING,
            CONTENT,
            3600,
        )],
    );
    assert!(
        !reg.is_available_for(&wrong_dept, "vps_reboot", &at(0))
            .await,
        "a grant aimed at another department must not admit this tool"
    );

    // A grant that covers the right capability and department but extends too
    // little scope also does not admit it.
    let too_narrow = AuthorityActor::new(
        &content_writer.employee_id,
        Some(CONTENT.to_string()),
        DEPT_RUNG,
        vec![Grant::new(
            "emp-ceo",
            &content_writer.employee_id,
            CAP_PLATFORM_TOOLING,
            AuthorityScope::Peers,
            Some(PLATFORM.to_string()),
            "org_authority_tools: grant extending less than the tool demands",
            Some(at(3600)),
        )],
    );
    assert!(
        !reg.is_available_for(&too_narrow, "vps_reboot", &at(0))
            .await,
        "a grant that does not extend far enough must not admit the tool"
    );

    // An actor in NO department is not in every department — fail closed on
    // absence, so an unbackfilled row never becomes the widest seat in the org.
    let undept = AuthorityActor::new("emp-nobody", None, DEPT_RUNG, Vec::new());
    assert!(
        !reg.is_available_for(&undept, "vps_reboot", &at(0)).await,
        "an actor with no department must not reach a departmental tool"
    );
    assert!(
        reg.is_available_for(&undept, "datetime", &at(0)).await,
        "an actor with no department still gets global tools"
    );
}

// =====================================================================
// 3. An explicit, attributable, unexpired grant admits the crossing tool.
// =====================================================================

/// §2.5's happy path, through the real store: issue → resolve → advertise.
#[tokio::test]
async fn an_explicit_grant_admits_the_crossing_tool() {
    let (db, _dir) = grant_store();
    let seats = three_seats();
    let reg = registry().await;

    let grantee = "emp-content-writer";
    let grant = live_grant(grantee, CAP_PLATFORM_TOOLING, PLATFORM, 3600);
    let grant_id = issue_grant(&grant, AuthorityScope::Org, &seats, &db).expect("grant issued");
    assert_eq!(
        grant_id, grant.grant_id,
        "issue_grant returns the id it stored"
    );
    assert_eq!(
        db.count().expect("count"),
        1,
        "a valid grant must be persisted exactly once"
    );

    let actor = AuthorityActor::resolve(grantee, DEPT_RUNG, &seats, &db)
        .expect("actor resolves from a filled seat");
    let advertised = advertised_names(reg.advertise_for(&actor, &at(0)).await);

    assert!(
        advertised.contains(&"vps_reboot".to_string()),
        "the granted tool must now be advertised; got {advertised:?}"
    );
    assert_eq!(
        advertised,
        vec![
            "cms_publish".to_string(),
            "datetime".to_string(),
            "vps_reboot".to_string()
        ],
        "the grant admits one extra tool and no more; got {advertised:?}"
    );

    // The allow reason is attributable — grantor, id, capability — which is
    // §2.5's whole point.
    let check = tool_authority_check(
        reg.tool_authority("vps_reboot").await.as_ref(),
        &actor,
        "vps_reboot",
        &at(0),
    );
    assert!(check.is_allowed());
    assert!(
        check.reason.contains(&grant.grant_id) && check.reason.contains("emp-ceo"),
        "the allow reason must attribute the grant; got: {}",
        check.reason
    );

    // Execution follows the same verdict (defence in depth).
    let executed = reg
        .execute_for(
            &actor,
            "vps_reboot",
            "call_1",
            json!({}),
            ToolContext::default(),
            &at(0),
        )
        .await;
    assert!(
        executed.is_ok(),
        "a granted actor may execute: {executed:?}"
    );

    // The Org-scoped CEO gets **no** free pass. The CEO sits in no department,
    // so hierarchy gives no answer for reaching `platform_infra`'s tooling, and
    // §2.1 requires an explicit grant precisely where hierarchy is silent.
    //
    // This is deliberately the *same* answer `can_post_to` gives an Org-scoped
    // actor posting into a sibling department (`authority.rs`, §2.3.1): that
    // function has no Org bypass either. Letting the tool filter admit on
    // scope while the board filter demands a grant would make the two
    // enforcement sites disagree, and §2.3's whole warning is that an
    // unenforced site is how the model becomes decorative.
    let ceo = ceo();
    let ceo_sees = advertised_names(reg.advertise_for(&ceo, &at(0)).await);
    assert_eq!(
        ceo_sees,
        vec!["datetime".to_string()],
        "Org scope is not a self-executing crossing pass: the CEO's grant names \
         emp-content-writer, so the CEO's own crossing request is refused and only \
         the global tool remains; got {ceo_sees:?}"
    );
    // An Org actor with no grant at all is refused at the same rung the board
    // site would demand.
    let bare_ceo = AuthorityActor::new("emp-ceo", None, AuthorityScope::Org, Vec::new());
    assert!(
        !reg.is_available_for(&bare_ceo, "vps_reboot", &at(0)).await,
        "Org scope is the power to GRANT, not a self-executing crossing pass"
    );
    let check = tool_authority_check(
        reg.tool_authority("vps_reboot").await.as_ref(),
        &bare_ceo,
        "vps_reboot",
        &at(0),
    );
    assert_eq!(
        check.required_scope,
        AuthorityScope::Org,
        "an Org actor's crossing denial still reports Org, as the board site does"
    );
    assert_eq!(
        db.count().expect("count"),
        1,
        "the CEO's own reach must come from a grant, never a silently minted row"
    );
}

// =====================================================================
// 4. An EXPIRED grant does not admit the tool.
// =====================================================================

/// §2.5: "grants expire by default". The expiry comparison is made in the
/// authority check against the supplied clock.
///
/// This deliberately hands the actor the **full grant history**, lapsed row
/// included, rather than the pre-filtered `list_for_grantee` output. A caller
/// that passes lapsed rows must still be refused, or the rule is enforced only
/// by one particular read path and any other read becomes a hole.
#[tokio::test]
async fn an_expired_grant_does_not_admit_the_tool() {
    let (db, _dir) = grant_store();
    let seats = three_seats();
    let reg = registry().await;

    let grantee = "emp-content-writer";
    let expired = live_grant(grantee, CAP_PLATFORM_TOOLING, PLATFORM, -60);
    issue_grant(&expired, AuthorityScope::Org, &seats, &db).expect("grant issued");

    // The audit view keeps the row; the live read drops it. Expiry is a read
    // filter, not a sweeper, so the record stays lossless.
    let history = db.history_for_grantee(grantee).expect("history");
    assert_eq!(
        history.len(),
        1,
        "the lapsed row must still be on record — the audit trail stays lossless"
    );
    assert!(
        db.list_for_grantee(grantee).expect("live list").is_empty(),
        "a lapsed grant must be absent from the live read"
    );

    // Re-attach the lapsed row directly, so this test goes red if the expiry
    // comparison inside the authority check is ever dropped and the read filter
    // is all that is left standing.
    let mut lapsed =
        AuthorityActor::resolve(grantee, DEPT_RUNG, &seats, &db).expect("actor resolves");
    lapsed.grants = history;
    assert!(
        !lapsed.grants.is_empty(),
        "the fixture must actually carry the lapsed row into the actor"
    );

    assert!(
        !reg.is_available_for(&lapsed, "vps_reboot", &at(0)).await,
        "an EXPIRED grant must not admit the tool even when the row is handed \
         to the authority check directly"
    );

    let check = tool_authority_check(
        reg.tool_authority("vps_reboot").await.as_ref(),
        &lapsed,
        "vps_reboot",
        &at(0),
    );
    assert!(!check.is_allowed());
    assert!(
        check.reason.contains("lapsed"),
        "the denial must say the grant lapsed, which is a different operator \
         action from never having had one; got: {}",
        check.reason
    );

    // Absent from the advertised list, not merely blocked on call.
    let advertised = advertised_names(reg.advertise_for(&lapsed, &at(0)).await);
    assert!(
        !advertised.contains(&"vps_reboot".to_string()),
        "an expired grant must not surface the tool at all; got {advertised:?}"
    );
    let denied = reg
        .execute_for(
            &lapsed,
            "vps_reboot",
            "call_1",
            json!({}),
            ToolContext::default(),
            &at(0),
        )
        .await
        .expect_err("an expired grant must not admit execution either");
    assert!(denied.to_string().contains("lapsed"), "got: {denied}");

    // Renewal restores it: expiry is a fact about `now`, not a permanent
    // revocation. This half is what makes the lapse test a real clock test
    // rather than a boolean.
    let renewed = live_grant(grantee, CAP_PLATFORM_TOOLING, PLATFORM, 3600);
    issue_grant(&renewed, AuthorityScope::Org, &seats, &db).expect("renewal issued");
    let after = AuthorityActor::resolve(grantee, DEPT_RUNG, &seats, &db)
        .expect("actor resolves after renewal");
    assert!(
        reg.is_available_for(&after, "vps_reboot", &at(0)).await,
        "a freshly issued grant must admit the tool again"
    );

    // A revoked grant does not admit it either — the other half of liveness.
    assert!(
        db.revoke(&renewed.grant_id, "no longer needed")
            .expect("revoke")
    );
    let revoked = AuthorityActor::resolve(grantee, DEPT_RUNG, &seats, &db)
        .expect("actor resolves after revocation");
    assert!(
        !reg.is_available_for(&revoked, "vps_reboot", &at(0)).await,
        "a revoked grant must not admit the tool"
    );
}

// =====================================================================
// 5. A grant to a vacant/invalid seat is refused and NOT persisted.
// =====================================================================

/// A grant to a vacant seat must never become **inert fabricated authority** —
/// a row naming an employee who does not exist, which every later read then
/// has to defend against.
///
/// "Refused" and "not persisted" are two different claims; only the second is
/// the interesting one, so the count assertions carry the weight here.
#[tokio::test]
async fn a_grant_to_a_vacant_or_invalid_seat_is_refused_and_not_persisted() {
    let (db, _dir) = grant_store();
    let reg = registry().await;

    // `emp-platform-head` is deliberately NOT in the roster (a vacant seat);
    // `emp-broken` is present but fails §3.2's required-field check.
    let mut broken = employee("emp-broken", Some(PLATFORM));
    broken.skills.clear();
    let seats = Roster::default()
        .with(employee("emp-ceo", None))
        .with(employee("emp-content-writer", Some(CONTENT)))
        .with(broken);

    // (a) vacant grantee.
    let vacant = live_grant("emp-platform-head", CAP_PLATFORM_TOOLING, PLATFORM, 3600);
    let err = issue_grant(&vacant, AuthorityScope::Org, &seats, &db)
        .expect_err("a grant to a vacant seat must be refused");
    assert!(
        err.to_string().contains("vacant"),
        "the refusal must name the vacant seat; got: {err}"
    );
    assert_eq!(
        db.count().expect("count"),
        0,
        "a refused grant must leave the table EMPTY — not a row, not a tombstone"
    );
    assert!(
        db.get(&vacant.grant_id).expect("get").is_none(),
        "the refused grant id must not exist in the store at all"
    );

    // (b) invalid grantee — a required identity field is missing.
    let invalid = live_grant("emp-broken", CAP_PLATFORM_TOOLING, PLATFORM, 3600);
    let err = issue_grant(&invalid, AuthorityScope::Org, &seats, &db)
        .expect_err("a grant to an invalid seat must be refused");
    assert!(
        err.to_string().contains("invalid") && err.to_string().contains("skills"),
        "the refusal must name the missing field so it can be fixed; got: {err}"
    );
    assert_eq!(
        db.count().expect("count"),
        0,
        "a refused grant to an invalid seat must not be persisted"
    );

    // (c) vacant grantor — §2.5 requires the grant to be attributable, so a
    // grant naming a grantor nobody holds is refused for the same reason.
    let orphan = Grant::new(
        "emp-nobody",
        "emp-content-writer",
        CAP_PLATFORM_TOOLING,
        DEPT_RUNG,
        Some(PLATFORM.to_string()),
        "org_authority_tools: grant from a seat nobody holds",
        Some(at(3600)),
    );
    let err = issue_grant(&orphan, AuthorityScope::Org, &seats, &db)
        .expect_err("a grant from a vacant seat must be refused");
    assert!(err.to_string().contains("grantor"), "got: {err}");
    assert_eq!(db.count().expect("count"), 0);

    // (d) amplification — delegation must not widen authority.
    let amplifying = Grant::new(
        "emp-content-writer",
        "emp-content-writer",
        CAP_PLATFORM_TOOLING,
        AuthorityScope::Org,
        Some(PLATFORM.to_string()),
        "org_authority_tools: delegation attempt that amplifies",
        Some(at(3600)),
    );
    let err = issue_grant(&amplifying, DEPT_RUNG, &seats, &db)
        .expect_err("a grantor holding less than the granted scope must be refused");
    assert!(err.to_string().contains("amplify"), "got: {err}");

    // Four refusals, zero rows: the invariant as a single number.
    assert_eq!(
        db.count().expect("count"),
        0,
        "NO refused grant may ever reach the table — fabricated authority for \
         a seat that cannot use it is the failure this function exists to stop"
    );

    // And the refused grants confer nothing observable.
    assert!(
        !reg.is_available_for(&writer_in(CONTENT), "vps_reboot", &at(0))
            .await,
        "a refused grant must confer no authority at all"
    );
    // `AuthorityActor::resolve` also refuses a seat nobody holds, so a caller
    // cannot obtain an actor for the vacant grantee at all.
    assert!(
        AuthorityActor::resolve("emp-platform-head", DEPT_RUNG, &seats, &db).is_err(),
        "an actor must not be resolvable for a vacant seat"
    );
    // …and it must refuse an INVALID seat, not just an absent one. An actor
    // built for an identity that cannot act is the same hazard one rung up: the
    // grant table stays clean, but a caller can still hold an `AuthorityActor`
    // for a seat §3.2 would refuse to let work.
    assert!(
        AuthorityActor::resolve("emp-broken", DEPT_RUNG, &seats, &db).is_err(),
        "an actor must not be resolvable for an INVALID seat — resolve must \
         apply the same Employee::is_valid rule issue_grant applies"
    );
}

// =====================================================================
// 6. A filtered-out tool is ABSENT from the advertised list — the headline
//    requirement.
// =====================================================================

/// Tested twice over: once on the **advertised** list (absence), and once by
/// showing the *unfiltered* list still contains the tool. The second half is
/// what gives the first half meaning — if `get_schemas` were silently filtered
/// too, this test would pass against an implementation that never advertised a
/// restricted tool at all.
#[tokio::test]
async fn a_filtered_out_tool_is_absent_from_the_advertised_list_not_merely_blocked() {
    let reg = registry().await;
    let content_writer = writer_in(CONTENT);

    // The tool EXISTS. This is not a "the tool was never mounted" test.
    let unfiltered = advertised_names(reg.get_schemas().await);
    assert!(
        unfiltered.contains(&"vps_reboot".to_string()),
        "the restricted tool must be registered and present in the unfiltered \
         list, or this test proves nothing; got {unfiltered:?}"
    );
    assert!(reg.contains("vps_reboot").await);
    assert!(reg.is_available("vps_reboot").await);

    // ABSENT from the advertised list. Not "listed but will fail".
    let advertised = advertised_names(reg.advertise_for(&content_writer, &at(0)).await);
    assert!(
        !advertised.contains(&"vps_reboot".to_string()),
        "ABSENCE is the requirement: a tool the model may not use must not be \
         offered to it; got {advertised:?}"
    );
    assert_eq!(
        advertised,
        vec!["cms_publish".to_string(), "datetime".to_string()],
        "the content writer sees exactly its own department's surface plus the \
         global tool — no more"
    );

    // The same verdict by name, for a log line or a prompt footer.
    let names = reg.tools_for(&content_writer, &at(0)).await;
    assert!(!names.contains(&"vps_reboot".to_string()));
    assert!(names.contains(&"cms_publish".to_string()));

    // Execution is blocked as well — defence in depth, and the reason the
    // absence assertion above is load-bearing rather than decorative.
    let denied = reg
        .execute_for(
            &content_writer,
            "vps_reboot",
            "call_1",
            json!({}),
            ToolContext::default(),
            &at(0),
        )
        .await
        .expect_err("execution must be refused");
    assert!(
        denied.to_string().contains("authority:"),
        "the refusal must be an authority refusal, not a missing-tool error; got: {denied}"
    );
    assert!(
        denied.to_string().contains(CAP_PLATFORM_TOOLING),
        "the refusal must name the capability; got: {denied}"
    );

    // The unfiltered paths are untouched: every pre-existing caller keeps
    // exactly the list it had before this feature existed.
    assert_eq!(
        advertised_names(reg.get_schemas().await),
        unfiltered,
        "get_schemas must remain the UNFILTERED list; the authority filter is \
         opt-in through advertise_for"
    );
    assert_eq!(
        advertised_names(reg.get_available_schemas_filtered(&[]).await),
        unfiltered,
        "get_available_schemas_filtered must also remain unfiltered by authority"
    );

    // A declared rung is enforced even for a global tool: authority is not
    // only about departments.
    let strict = ToolRegistry::new(Duration::from_secs(5));
    strict
        .register(NamedTool {
            name: "org_dissolve",
        })
        .await
        .expect("register");
    strict
        .set_tool_authority(
            "org_dissolve",
            ToolAuthority::global_requiring(AuthorityScope::Org),
        )
        .await;
    assert!(
        !strict
            .is_available_for(&head_of(PLATFORM), "org_dissolve", &at(0))
            .await,
        "a Descendants-scoped actor must not run an Org-rung tool"
    );
    assert!(
        strict
            .is_available_for(&ceo(), "org_dissolve", &at(0))
            .await,
        "an Org-scoped actor may run an Org-rung tool"
    );

    // A half-declared binding (a department but no capability) can never be
    // admitted across the boundary: no grant can name what it never declared.
    let half = ToolAuthority {
        department: Some(PLATFORM.to_string()),
        capability: None,
        required_scope: DEPT_RUNG,
    };
    let wide_grant = AuthorityActor::new(
        "emp-content-writer",
        Some(CONTENT.to_string()),
        AuthorityScope::Org,
        vec![Grant::new(
            "emp-ceo",
            "emp-content-writer",
            CAP_PLATFORM_TOOLING,
            AuthorityScope::Org,
            None,
            "org_authority_tools: broad grant against an undeclared capability",
            None,
        )],
    );
    assert!(
        !tool_authority_check(Some(&half), &wide_grant, "half_bound", &at(0)).is_allowed(),
        "a department-bound tool with no declared capability must refuse every \
         crossing request, even with a broad grant in hand"
    );

    // Bindings survive a registry clone: the clone shares the map, the same way
    // it shares the tool map and the disabled sets.
    let cloned = reg.clone();
    assert_eq!(
        advertised_names(cloned.advertise_for(&content_writer, &at(0)).await),
        advertised,
        "a cloned registry must filter identically"
    );

    // …and survive re-registration under the same name.
    reg.register(NamedTool { name: "vps_reboot" })
        .await
        .expect("re-register");
    assert!(
        reg.tool_authority("vps_reboot").await.is_some(),
        "the binding is keyed by name and must survive re-registration"
    );
    assert!(
        !reg.is_available_for(&content_writer, "vps_reboot", &at(0))
            .await,
        "re-registering a tool must not silently drop its authority binding"
    );

    // The two filters compose rather than one masking the other.
    reg.disable_tool("cms_publish").await;
    assert!(
        !advertised_names(reg.advertise_for(&content_writer, &at(0)).await)
            .contains(&"cms_publish".to_string()),
        "the disable filter must compose with the authority filter"
    );
    reg.enable_tool("cms_publish").await;
    assert!(
        advertised_names(reg.advertise_for(&content_writer, &at(0)).await)
            .contains(&"cms_publish".to_string()),
        "re-enabling restores the tool"
    );
    assert!(
        advertised_names(reg.advertise_for(&content_writer, &at(0)).await) == advertised,
        "the restored list must match the original exactly"
    );
}
