//! Inline `$skill` mentions — the dollar-sigil invocation form.
//!
//! A message can name a skill mid-sentence ("apply $ponytail to this diff"),
//! and the mention expands into the SAME invocation message `/skill <name>`
//! builds, so one code path — and one curator telemetry write — serves the TUI
//! composer, the gateway channels and the agent's own lane.
//!
//! Grammar (codex parity, `chat_composer.rs`'s `$` tier):
//!
//! * the name is `[A-Za-z0-9_-]` (plus `:`, the plugin-owned skill spelling
//!   codex allows), so a mention never spans whitespace;
//! * the `$` must sit on a word boundary — `a$b` and `USD$5` are not mentions;
//! * a name with no lowercase letter, or a name of digits only, is shell
//!   syntax (`$HOME`, `$1`) and is never read as a skill;
//! * `${VAR}` and `$(cmd)` are rejected by the same rule in one step: the byte
//!   after `$` is not a name character, so no name is collected.

use std::path::Path;

use crate::agent::skill_preprocessing::build_skill_invocation_message_in;

/// Characters that may appear in a mention name.
///
/// codex `chat_composer.rs::is_mention_name_char`, plus `/` so a
/// category-qualified name (`$software-development/axi`) is ONE token — the
/// listing shows that path, and the popup has to complete it. `/` cannot
/// begin a shell variable name, so the shell-spelling guard below still
/// rejects every real shell form.
fn is_mention_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'/')
}

/// True when a bare `$name` is shell syntax rather than a skill name.
///
/// The registry's skill names always carry a lowercase letter (they are
/// directory names), so an all-uppercase or all-digit spelling is a shell
/// variable or positional parameter: `$HOME`, `$PATH`, `$1`, `$12`. Rejecting
/// the whole class costs one comparison and needs no env-var allowlist.
pub fn is_shell_spelling(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.bytes().all(|b| b.is_ascii_digit()) || !name.bytes().any(|b| b.is_ascii_lowercase())
}

/// True when a name is not a usable path-shaped skill name: an empty segment
/// (`foo/`, `/foo`, `a//b`) can never resolve, and `is_plain_relative_path`
/// would refuse it at resolve time — so the popup must not offer it.
#[must_use]
pub fn has_empty_path_segment(name: &str) -> bool {
    name.split('/').any(str::is_empty)
}

/// One `$name` occurrence inside a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMention {
    /// The skill name typed after the `$`.
    pub name: String,
    /// Byte offset of the `$`.
    pub start: usize,
    /// Byte offset just past the name.
    pub end: usize,
}

/// Every `$name` token in `text` that is a plausible skill mention.
///
/// Mentions never overlap: the scan resumes at the end of each accepted name,
/// so `$a$b` is two mentions and `$a-$b` is one (`a-` plus `b`... no: `a-_b` is
/// one name) — the boundary rule is the char before the `$`.
pub fn find_skill_mentions(text: &str) -> Vec<SkillMention> {
    let bytes = text.as_bytes();
    let mut out: Vec<SkillMention> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        // Word boundary: the byte before `$` must not itself be a name char,
        // so `a$b`, `USD$5` and an email-like `x$y` are left alone.
        let preceded_by_name_char = index > 0
            && text
                .as_bytes()
                .get(index - 1)
                .is_some_and(|b| is_mention_name_char(*b));
        if preceded_by_name_char {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && is_mention_name_char(bytes[end]) {
            end += 1;
        }
        // `end` is a char boundary: every consumed byte was ASCII, and the
        // loop stopped at a non-name byte which is either ASCII or the start
        // of a multi-byte char.
        if let Some(name) = text.get(index + 1..end)
            && !name.is_empty()
            && !is_shell_spelling(name)
            && !has_empty_path_segment(name)
        {
            out.push(SkillMention {
                name: name.to_string(),
                start: index,
                end,
            });
        }
        index = end;
    }
    out
}

/// The text that surrounds the mentions: everything with every mention token
/// removed and the seam between words closed.
///
/// One space directly after a mention is swallowed so "use $ponytail here"
/// yields "use here", not "use  here". A message with no mention returns its
/// own trimmed text, which is what makes a bare `$name` submit behave exactly
/// like `/skill <name>` with an empty instruction.
pub fn mention_residue(text: &str) -> String {
    let mentions = find_skill_mentions(text);
    if mentions.is_empty() {
        return text.trim().to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for mention in &mentions {
        out.push_str(&text[last..mention.start]);
        last = mention.end;
        if text[last..].starts_with(' ') {
            last += 1;
        }
    }
    out.push_str(&text[last..]);
    out.trim().to_string()
}

/// The `$query` token the cursor sits in, for a composer's mention popup.
///
/// Returns `None` unless `input` ENDS with a `$`-prefixed, name-shaped token:
/// the popup follows the live token, exactly like the `@` file tier. A trailing
/// space, or a name the shell rules reject, closes the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSkillToken<'a> {
    /// Byte offset of the `$`, so an accepted suggestion can replace the token.
    pub start: usize,
    /// The name typed so far (empty for a bare `$`).
    pub query: &'a str,
}

pub fn active_skill_token(input: &str) -> Option<ActiveSkillToken<'_>> {
    let start = input.rfind('$')?;
    let preceded_by_name_char = start > 0
        && input
            .as_bytes()
            .get(start - 1)
            .is_some_and(|b| is_mention_name_char(*b));
    if preceded_by_name_char {
        return None; // inside a word: `a$b`
    }
    let query = &input[start + 1..];
    if !query.bytes().all(is_mention_name_char) {
        return None; // a space or a non-name byte ends the token
    }
    if !query.is_empty() && is_shell_spelling(query) {
        return None; // `$HOME` is not a skill being typed
    }
    Some(ActiveSkillToken { start, query })
}

/// The outcome of expanding a message's mentions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MentionExpansion {
    /// The message to submit instead of the original text. `None` means the
    /// text carried no resolvable mention and must be submitted unchanged.
    pub message: Option<String>,
    /// Names that were typed but did not resolve, for a status notice.
    pub unresolved: Vec<String>,
    /// Names that resolved, in the order they appeared.
    pub resolved: Vec<String>,
}

impl MentionExpansion {
    /// True when the text carries at least one mention, resolvable or not.
    pub fn is_mention(&self) -> bool {
        !self.resolved.is_empty() || !self.unresolved.is_empty()
    }
}

/// Expand every `$name` mention in `text` against `skills_dir`.
///
/// Each resolved name becomes the invocation message
/// [`build_skill_invocation_message_in`] builds for `/skill <name>` — including
/// the curator use telemetry that function records. The user's surrounding
/// words ride the FIRST block as its instruction, so a single mention behaves
/// exactly like `/skill <name> <instruction>` and the words are not repeated
/// when several skills are named at once.
pub fn expand_skill_mentions_in(
    skills_dir: &Path,
    text: &str,
    session_id: Option<&str>,
    vendor_roots: &[std::path::PathBuf],
) -> MentionExpansion {
    let mentions = find_skill_mentions(text);
    let mut out = MentionExpansion::default();
    if mentions.is_empty() {
        return out;
    }
    let residue = mention_residue(text);
    let mut blocks: Vec<String> = Vec::new();
    for mention in &mentions {
        // The user's words are the instruction for the first skill only.
        let instruction = if blocks.is_empty() {
            residue.as_str()
        } else {
            ""
        };
        match build_skill_invocation_message_in(
            skills_dir,
            &mention.name,
            instruction,
            session_id,
            vendor_roots,
        ) {
            Some(message) => {
                out.resolved.push(mention.name.clone());
                blocks.push(message);
            }
            None => out.unresolved.push(mention.name.clone()),
        }
    }
    if !blocks.is_empty() {
        out.message = Some(blocks.join("\n\n"));
    }
    out
}

/// The installed, loadable skills as `(name, description)` pairs.
///
/// One owner for the "what can actually run" filter: a skill whose
/// prerequisites are unmet is omitted, so a listing never offers a skill the
/// agent cannot follow. Surfaced this way by the TUI popup and the gateway's
/// picker alike.
pub fn loadable_skills(
    skills_dir: &Path,
    vendor_roots: &[std::path::PathBuf],
) -> Vec<(String, String)> {
    loadable_skills_in(&roots_from(skills_dir, vendor_roots))
}

/// [`loadable_skills`] over a resolved root list.
///
/// The pair form above stays for the surfaces that have not been threaded
/// yet; a surface that HAS a [`crate::skills::SkillRoots`] calls this so it
/// cannot disagree with the manager.
pub fn loadable_skills_in(roots: &crate::skills::SkillRoots) -> Vec<(String, String)> {
    loadable_skills_ranked_in(roots)
        .into_iter()
        .map(|(name, description, _rank)| (name, description))
        .collect()
}

/// [`loadable_skills_in`] plus each skill's root precedence index.
///
/// Search needs the rank to break a match-quality tie: two skills that match a
/// query equally well must resolve by ROOT PRECEDENCE (operator pool > extra
/// roots > vendor), not by the alphabet, because the result is truncated and
/// an alphabetical tiebreak can drop the operator's own skill in favour of an
/// integration's (iter-2469). The rank is 0 for the pool and rises from there.
pub fn loadable_skills_ranked_in(
    roots: &crate::skills::SkillRoots,
) -> Vec<(String, String, usize)> {
    let mut manager = crate::skills::SkillManager::from_roots(roots);
    if manager.load_all().is_err() {
        return Vec::new();
    }
    manager
        .list()
        .into_iter()
        .filter(|(name, _)| manager.get(name).is_some_and(|s| manager.is_available(s)))
        .map(|(name, description)| {
            let rank = manager.root_rank_of(&name).unwrap_or(usize::MAX);
            (name, description, rank)
        })
        .collect()
}

/// Rank `loadable_skills` for a query: exact name, then prefix, then substring,
/// bounded by `limit`.
///
/// The same ranking the composer's `/skill <Tab>` tier applies, derived once so
/// the gateway's picker cannot drift from what the TUI offers.
pub fn search_skills(
    skills_dir: &Path,
    vendor_roots: &[std::path::PathBuf],
    query: &str,
    limit: usize,
) -> Vec<(String, String)> {
    search_skills_in(&roots_from(skills_dir, vendor_roots), query, limit)
}

/// [`search_skills`] over a resolved root list.
pub fn search_skills_in(
    roots: &crate::skills::SkillRoots,
    query: &str,
    limit: usize,
) -> Vec<(String, String)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matched: Vec<(u8, usize, String, String)> = Vec::new();
    for (name, description, root_rank) in loadable_skills_ranked_in(roots) {
        // The SAME tiers `rank_skill_entries` applies to the popup's snapshot.
        // Searching by what a skill DOES was silently impossible before this
        // tier existed -- `jsonpath` returned nothing for a skill whose whole
        // description was "query and filter nested JSON documents", so the only
        // way to find a skill was to already know its name.
        let Some(tier) = skill_match_tier(&name, &description, &needle) else {
            continue;
        };
        matched.push((tier, root_rank, name, description));
    }
    // THREE keys, in order: match quality, then ROOT PRECEDENCE, then name.
    //
    // The third key is new (iter-2469). With only (tier, name), a pool skill
    // and a vendor skill of equal match quality were separated by the ALPHABET,
    // so `zzz-vendor` sorted ahead of `aaa-pool` and `take(limit)` dropped the
    // operator's own skill. Root precedence is the order every other surface
    // already resolves by; search was the one that ignored it.
    matched.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    matched
        .into_iter()
        .take(limit)
        .map(|(_, _, name, description)| (name, description))
        .collect()
}

/// Rank `(name, description)` pairs for a query and bound the result.
///
/// The ONE ranking, exposed for the surfaces that already hold their entries
/// in memory (the TUI's `$` popup snapshot) rather than on disk. `search_skills_in`
/// is the same tiers over a `SkillRoots`; this is the same tiers over a list,
/// so the popup and `operant skills search` cannot order the same query
/// differently.
///
/// **An empty query returns every entry, unranked.** That is the browse case --
/// `$` with nothing typed offers the whole catalogue, and `search_skills_in`
/// rejects an empty needle outright. The split is deliberate: a browse is an
/// inventory, a search is a ranking.
#[must_use]
pub fn rank_skill_entries(
    entries: &[(String, String)],
    query: &str,
    limit: usize,
) -> Vec<(String, String)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return entries.iter().take(limit).cloned().collect();
    }
    let mut matched: Vec<(u8, &str, &str)> = entries
        .iter()
        .filter_map(|(name, description)| {
            let tier = skill_match_tier(name, description, &needle)?;
            Some((tier, name.as_str(), description.as_str()))
        })
        .collect();
    matched.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    matched
        .into_iter()
        .take(limit)
        .map(|(_, name, description)| (name.to_string(), description.to_string()))
        .collect()
}

/// The tier `name`/`description` earn for `needle`, or `None` for no match.
/// Extracted so the ranking has ONE definition rather than one per surface.
#[must_use]
fn skill_match_tier(name: &str, description: &str, needle: &str) -> Option<u8> {
    let lowered = name.to_lowercase();
    if lowered == needle {
        Some(0)
    } else if lowered.starts_with(needle) {
        Some(1)
    } else if lowered.contains(needle) {
        Some(2)
    } else if description.to_lowercase().contains(needle) {
        Some(3)
    } else {
        None
    }
}

/// The flat root list the pair-form helpers consume: the pool first, then the
/// vendor roots. `SkillRoots::resolve` is the form that also carries the
/// operator's `extra_roots`, the project root and the refusal report.
fn roots_from(skills_dir: &Path, vendor_roots: &[std::path::PathBuf]) -> crate::skills::SkillRoots {
    crate::skills::SkillRoots {
        roots: std::iter::once(skills_dir.to_path_buf())
            .chain(vendor_roots.iter().cloned())
            .collect(),
        ..crate::skills::SkillRoots::default()
    }
}

#[cfg(test)]
mod tests;
