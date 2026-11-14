//! Unit tests for the inline `$skill` mention grammar.
//!
//! The grammar is a security-adjacent parser: it decides whether text the user
//! typed is an instruction to invoke a skill, in a shell where `$HOME`,
//! `$(cmd)` and `${VAR}` are three spellings of something else entirely. Every
//! rejection rule below has a case; every acceptance rule has a case.

use super::*;

fn names(text: &str) -> Vec<String> {
    find_skill_mentions(text)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

// ── the accepted grammar ───────────────────────────────────────────────────

#[test]
fn a_mention_is_found_mid_sentence() {
    let mentions = find_skill_mentions("please apply $ponytail to this diff");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].name, "ponytail");
    assert_eq!(mentions[0].start, 13);
    // "$ponytail" is 9 bytes: the `$` at 13 and the name's last byte at 21.
    assert_eq!(mentions[0].end, 22);
}

#[test]
fn a_mention_found_at_the_start_and_at_the_end() {
    assert_eq!(names("$ponytail"), vec!["ponytail"]);
    assert_eq!(names("use $ponytail"), vec!["ponytail"]);
}

#[test]
fn every_name_character_is_accepted() {
    assert_eq!(names("$skill_name"), vec!["skill_name"]);
    assert_eq!(names("$skill-name"), vec!["skill-name"]);
    assert_eq!(names("$skill123"), vec!["skill123"]);
    assert_eq!(names("$plugin:skill"), vec!["plugin:skill"]);
    // iter-2254: `/` makes a category-qualified name ONE token, because that
    // is the path the listing shows (`software-development/axi`).
    assert_eq!(names("$cat/leaf"), vec!["cat/leaf"]);
}

#[test]
fn an_empty_path_segment_is_not_a_mention() {
    // The popup must not offer a name `is_plain_relative_path` would refuse at
    // resolve time, and the expander cannot run.
    assert!(names("$foo/").is_empty(), "trailing slash");
    assert!(names("$/foo").is_empty(), "leading slash");
    assert!(names("$a//b").is_empty(), "empty middle segment");
    // A real two-segment name is still one mention.
    assert_eq!(names("$a/b"), vec!["a/b"]);
}

#[test]
fn two_mentions_in_one_message_are_both_found_in_order() {
    assert_eq!(
        names("use $ponytail then $axi"),
        vec!["ponytail".to_string(), "axi".to_string()]
    );
}

#[test]
fn a_second_dollar_inside_a_word_is_not_a_mention() {
    // `$a$b` is ONE mention: the second `$` follows a name character, so the
    // word-boundary rule rejects it — the same rule that rejects `a$b`.
    assert_eq!(names("$a$b"), vec!["a"]);
    // Separated by a word, both are mentions.
    assert_eq!(names("use $a and $b"), vec!["a", "b"]);
}

#[test]
fn a_longer_message_keeps_mention_offsets_correct() {
    let mentions = find_skill_mentions("first $aaa, then $bbb, then $ccc");
    let offsets: Vec<(usize, usize)> = mentions.iter().map(|m| (m.start, m.end)).collect();
    assert_eq!(offsets, vec![(6, 10), (17, 21), (28, 32)]);
}

// ── the rejected grammar (shell syntax) ────────────────────────────────────

#[test]
fn an_all_caps_name_is_a_shell_variable_not_a_skill() {
    assert_eq!(names("echo $HOME"), Vec::<String>::new());
    assert_eq!(names("run $PATH now"), Vec::<String>::new());
    assert_eq!(names("in $MY_DIR"), Vec::<String>::new());
}

#[test]
fn a_numeric_name_is_a_positional_parameter() {
    assert_eq!(names("costs $5 total"), Vec::<String>::new());
    assert_eq!(names("the $12 rules"), Vec::<String>::new());
}

#[test]
fn a_parameter_expansion_is_rejected_because_the_brace_is_not_a_name_char() {
    assert_eq!(names("in ${OPERANT_SKILL_DIR}"), Vec::<String>::new());
}

#[test]
fn a_command_substitution_is_rejected_the_same_way() {
    assert_eq!(names("in $(echo hi) done"), Vec::<String>::new());
    assert_eq!(names("$(</dev/null)"), Vec::<String>::new());
}

#[test]
fn a_dollar_followed_by_nothing_or_punctuation_is_not_a_mention() {
    assert_eq!(names("it costs $"), Vec::<String>::new());
    assert_eq!(names("it costs $."), Vec::<String>::new());
    assert_eq!(names("it costs $!"), Vec::<String>::new());
    assert_eq!(names("it costs $?"), Vec::<String>::new());
}

#[test]
fn a_dollar_inside_a_word_is_not_a_mention() {
    assert_eq!(names("USD$5 is a lot"), Vec::<String>::new());
    assert_eq!(names("user$name"), Vec::<String>::new());
    assert_eq!(names("a$b"), Vec::<String>::new());
}

#[test]
fn a_shell_spelling_rule_matches_exactly_what_it_claims() {
    // Digits alone, or a name with NO lowercase letter, is shell syntax. The
    // registry's skill names always carry a lowercase letter, so this rejects
    // `$1`, `$HOME` and `$M1` without an env-var allowlist to maintain.
    assert!(is_shell_spelling("HOME"));
    assert!(is_shell_spelling("PATH"));
    assert!(is_shell_spelling("MY_DIR"));
    assert!(is_shell_spelling("M1"));
    assert!(is_shell_spelling("1"));
    assert!(is_shell_spelling("12"));
    assert!(!is_shell_spelling("ponytail"));
    assert!(!is_shell_spelling("a"));
    assert!(!is_shell_spelling("skill-name"));
    assert!(!is_shell_spelling("skill:name"));
}

// ── the residue ───────────────────────────────────────────────────────────

#[test]
fn a_single_mention_residue_is_the_users_instruction() {
    assert_eq!(
        mention_residue("use $ponytail to review this"),
        "use to review this"
    );
    assert_eq!(mention_residue("$ponytail"), "");
    assert_eq!(mention_residue("$ponytail review this"), "review this");
}

#[test]
fn the_seam_between_words_is_closed() {
    assert_eq!(mention_residue("use $a and $b here"), "use and here");
}

#[test]
fn a_message_with_no_mention_is_returned_trimmed_unchanged() {
    assert_eq!(mention_residue("  no mention here  "), "no mention here");
    // Formatting the user typed is preserved: only the mention seams are touched.
    assert_eq!(
        mention_residue("line one\n\nline two"),
        "line one\n\nline two"
    );
}

// ── the composer's live token ─────────────────────────────────────────────

#[test]
fn the_live_token_is_only_the_trailing_one() {
    let token = active_skill_token("please use $pon").expect("live token");
    assert_eq!(token.start, 11);
    assert_eq!(token.query, "pon");
}

#[test]
fn a_bare_dollar_is_a_live_token_with_an_empty_query() {
    let token = active_skill_token("use $").expect("bare dollar");
    assert_eq!(token.query, "");
}

#[test]
fn a_trailing_space_closes_the_popup() {
    assert!(active_skill_token("use $ponytail ").is_none());
}

#[test]
fn a_dollar_mid_word_is_not_live() {
    assert!(active_skill_token("use a$b").is_none());
    assert!(active_skill_token("costs $5").is_none());
    assert!(active_skill_token("echo $HOME").is_none());
}

// ── expansion against a real skills tree ──────────────────────────────────

/// Write a minimal skill into `dir/name`.
fn write_skill(dir: &std::path::Path, name: &str) {
    let skill_dir = dir.join(name);
    std::fs::create_dir_all(&skill_dir).expect("skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: The {name} skill.\n---\n\n# {name}\n"),
    )
    .expect("SKILL.md");
}

/// A minimal skill tree: two skills, one loadable and one not.
fn skill_tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let alpha = dir.path().join("alpha-skill");
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::write(
        alpha.join("SKILL.md"),
        "---\nname: alpha-skill\ndescription: The alpha skill.\n---\n\nAlpha body.\n",
    )
    .unwrap();
    let beta = dir.path().join("beta-skill");
    std::fs::create_dir_all(&beta).unwrap();
    std::fs::write(
        beta.join("SKILL.md"),
        "---\nname: beta-skill\ndescription: The beta skill.\nprerequisites_commands:\n  - -definitely-not-a-command-xyz\n---\n\nBeta body.\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_message_with_no_mention_is_left_alone() {
    let dir = skill_tree();
    let out = expand_skill_mentions_in(dir.path(), "just a plain message", None, &[]);
    assert!(!out.is_mention());
    assert!(out.message.is_none());
}

#[test]
fn a_single_mention_expands_into_the_skill_body_and_the_instruction() {
    let dir = skill_tree();
    let out = expand_skill_mentions_in(dir.path(), "do this with $alpha-skill now", None, &[]);
    assert!(out.unresolved.is_empty());
    assert_eq!(out.resolved, vec!["alpha-skill".to_string()]);
    let message = out.message.expect("an expansion");
    assert!(message.contains("alpha-skill"), "{message}");
    assert!(message.contains("Alpha body."), "{message}");
    assert!(
        message.contains("User instruction: do this with now"),
        "{message}"
    );
}

#[test]
fn an_unresolvable_mention_is_reported_and_nothing_is_submitted_for_it() {
    let dir = skill_tree();
    let out = expand_skill_mentions_in(dir.path(), "use $no-such-skill here", None, &[]);
    assert_eq!(out.unresolved, vec!["no-such-skill".to_string()]);
    assert!(out.resolved.is_empty());
    assert!(out.message.is_none());
}

#[test]
fn a_loadable_only_filter_drops_the_skill_whose_prerequisites_are_unmet() {
    let dir = skill_tree();
    let rows = loadable_skills(dir.path(), &[]);
    let listed: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(listed, vec!["alpha-skill"]);
}

#[test]
fn search_ranks_exact_before_prefix_before_substring() {
    let dir = skill_tree();
    std::fs::create_dir_all(dir.path().join("alpha-two")).unwrap();
    std::fs::write(
        dir.path().join("alpha-two").join("SKILL.md"),
        "---\nname: alpha-two\ndescription: second\n---\n\nbody\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("zalpha")).unwrap();
    std::fs::write(
        dir.path().join("zalpha").join("SKILL.md"),
        "---\nname: zalpha\ndescription: third\n---\n\nbody\n",
    )
    .unwrap();
    let hits = search_skills(dir.path(), &[], "alpha", 10);
    let ranked: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(ranked, vec!["alpha-skill", "alpha-two", "zalpha"]);
    assert_eq!(search_skills(dir.path(), &[], "nope", 10).len(), 0);
}

/// A skill is findable by what it DOES, not only by its name.
///
/// `search_skills` matches the name; the description is part of what a caller
/// can see and remember, so it has to be searchable too. A description hit
/// ranks AFTER every name hit, so a real name match is never displaced by a
/// description coincidence.
///
/// MUTATION-PROOF: removing the description tier fails the first assertion.
#[test]
fn search_matches_the_description_after_every_name_hit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let forms = dir.path().join("ntn-forms");
    std::fs::create_dir_all(&forms).unwrap();
    std::fs::write(
        forms.join("SKILL.md"),
        "---\nname: ntn-forms\ndescription: Fill and flatten PDF forms.\n---\n\nbody\n",
    )
    .unwrap();
    let named = dir.path().join("pdf-fill");
    std::fs::create_dir_all(&named).unwrap();
    std::fs::write(
        named.join("SKILL.md"),
        "---\nname: pdf-fill\ndescription: Fill and flatten PDF forms.\n---\n\nbody\n",
    )
    .unwrap();

    // Only the description carries the query for `ntn-forms`, so a name-only
    // search cannot find it.
    let hits = search_skills(dir.path(), &[], "forms", 10);
    let ranked: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        ranked.contains(&"ntn-forms"),
        "a skill must be findable by what it does: {ranked:?}"
    );

    // `pdf` is IN the name of one skill and only in the description of the
    // other: the name hit comes first.
    let hits = search_skills(dir.path(), &[], "pdf", 10);
    let ranked: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        ranked,
        vec!["pdf-fill", "ntn-forms"],
        "a name hit must rank ahead of a description-only hit: {ranked:?}"
    );
}

#[test]
fn loadable_skills_includes_the_vendor_roots() {
    // The invocable pool is the operator's own skills PLUS the vendor roots
    // (the installed integrations and the plugin roots) — the same roots the
    // expansion resolves against. A vendor skill that is absent from this
    // list is a skill the popup cannot offer but the expander would run.
    let dir = tempfile::tempdir().expect("temp dir");
    let pool = dir.path().join("skills");
    let vendor = dir.path().join("vendor");
    write_skill(&pool, "alpha-skill");
    write_skill(&vendor, "ponytail-review");

    let rows = loadable_skills(&pool, std::slice::from_ref(&vendor));
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["alpha-skill", "ponytail-review"],
        "the vendor root's skills belong to the invocable pool"
    );

    // Without the vendor root the pool is the operator's own alone.
    let rows = loadable_skills(&pool, &[]);
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["alpha-skill"]);

    // A vendor root that is not installed is skipped, not an error.
    let rows = loadable_skills(&pool, &[dir.path().join("not-installed")]);
    assert_eq!(
        rows.len(),
        1,
        "a missing vendor root must not empty the pool"
    );
}
