//! `/learn` command — build the standards-guided prompt that turns whatever
//! the user described into a reusable skill.
//!
//! Ported from `hermes-agent/agent/learn_prompt.py`.
//!
//! `/learn` is open-ended. The user can point it at anything they can describe:
//! a directory of code, an API doc URL, a workflow they just walked the agent
//! through in this conversation, or pasted notes. This module builds ONE prompt
//! that instructs the live agent to:
//!
//!   1. Gather the sources the user named, using the tools it already has
//!      (`read_file` / `search_files` for dirs, `web_extract` for URLs, the
//!      current conversation for "what I just did", the user's text for pasted
//!      material).
//!   2. Author a single `SKILL.md` via `skill_manage` that follows the
//!      Operant skill-authoring standards.
//!
//! There is no separate distillation engine and no model-tool footprint: the
//! agent does the work with its existing toolset, so this works identically on
//! local, Docker, and remote terminal backends.

/// Returns the house-style rules for skill authoring.
///
/// Built as a function (not a raw string constant) because the markdown content
/// contains `\"##` sequences that prematurely close raw string delimiters.
fn authoring_standards() -> &'static str {
    "Follow the Operant skill-authoring standards exactly. These are the same \
     HARDLINE rules a maintainer enforces in review:\n\
     \n\
     Frontmatter:\n\
     - name: lowercase-hyphenated, <=64 chars, no spaces.\n\
     - description: ONE sentence, **<=60 characters**, ends with a period. State the\n\
       capability, not the implementation. No marketing words (powerful,\n\
       comprehensive, seamless, advanced, robust). Do NOT repeat the skill name. If\n\
       the description contains a colon, wrap the whole value in double quotes.\n\
       This is the most-violated rule and it is NOT cosmetic: the system-prompt\n\
       skill index truncates the description to 60 chars and loads it every\n\
       session, so anything past char 60 is silently cut and never routes. After\n\
       you write the description, COUNT the characters; if it is over 60, cut it\n\
       down before saving \u{2014} do not ship a sentence and hope.\n\
         Good (<=60): `Search arXiv papers by keyword, author, or ID.`\n\
         Bad (123):   `A comprehensive skill that lets the agent search arXiv for\n\
                       academic papers using keywords, authors, and categories.`\n\
     - version: 0.1.0\n\
     - author: always the literal value `Operant`. NEVER fill it from the host\n\
       environment \u{2014} the OS/login username, git config, or any identity you can\n\
       probe must not be written. Skills get shared and published, so an\n\
       environment-derived name is a privacy leak the user never opted into.\n\
     - platforms: declare `[macos]`, `[linux]`, and/or `[windows]` IF the skill\n\
       uses OS-bound primitives. Prefer fixing it cross-platform first; gate only\n\
       when the dependency is genuinely platform-bound. Omit the field for portable\n\
       skills.\n\
     - metadata.operant.tags: a few Capitalized, Relevant, Tags.\n\
     \n\
     Body section order (omit a section only if it genuinely has no content):\n\
     1. \"# <Human Title>\" then a 2-3 sentence intro: what it does, what it does NOT\n\
        do, and the key dependency stance (e.g. \"stdlib only\").\n\
     2. \"## When to Use\" \u{2014} bullet list of concrete trigger phrases.\n\
     3. \"## Prerequisites\" \u{2014} exact env vars, install steps, credentials.\n\
     4. \"## How to Run\" \u{2014} the canonical invocation, framed through Operant tools.\n\
     5. \"## Quick Reference\" \u{2014} a flat command/endpoint list, no narration.\n\
     6. \"## Procedure\" \u{2014} numbered steps with copy-paste-exact commands.\n\
     7. \"## Pitfalls\" \u{2014} known limits, rate limits, things that look broken but aren't.\n\
     8. \"## Verification\" \u{2014} a single command/check that proves the skill worked.\n\
     \n\
     Operant-tool framing (this is what makes it a skill, not shell docs):\n\
     - Frame running scripts as \"invoke through the `terminal` tool\".\n\
     - Reference Operant tools by name in backticks: `terminal`, `read_file`,\n\
       `write_file`, `search_files`, `patch`, `web_extract`, `web_search`,\n\
       `browser_navigate`, `delegate_task`, `memory`, `skill_manage`.\n\
     - Do NOT name shell utilities the agent already has wrapped: say `read_file`\n\
       not cat/head/tail, `search_files` not grep/rg/find/ls, `patch` not sed/awk,\n\
       `web_extract` not curl-to-scrape, `write_file` not echo>file or heredocs.\n\
     - Third-party CLIs (ffmpeg, gh, an SDK) are fine inside a script file, but the\n\
       prose still frames them as \"invoke through the `terminal` tool\". If the\n\
       skill needs an MCP server, name it and document its setup in Prerequisites.\n\
     \n\
     Quality bar:\n\
     - Prefer exact commands, endpoint URLs, function signatures, and config keys\n\
       that appear VERBATIM in the source. NEVER invent flags, paths, or APIs \u{2014} if\n\
       you didn't see it in the source, don't write it.\n\
     - Keep it tight and scannable: ~100 lines for a simple skill, ~200 for a\n\
       complex one. Don't re-paste the source docs.\n\
     - Don't write a router/index/hub skill that only points at other skills.\n\
     - Larger scripts/parsers belong in a `scripts/` file (added via\n\
       `skill_manage` write_file), referenced from SKILL.md by relative path \u{2014} not\n\
       inlined for the agent to re-type every run. References go in `references/`,\n\
       templates in `templates/`."
}

/// Build the agent prompt for an open-ended `/learn` request.
///
/// # Arguments
/// * `user_request` — the free-text the user gave after `/learn` — a
///   description of the workflow, paths, URLs, or "what I just did".
///
/// # Returns
/// A complete instruction the agent runs as a normal turn. The agent
/// gathers the described sources with its existing tools and authors the
/// skill via `skill_manage`.
pub fn build_learn_prompt(user_request: &str) -> String {
    let req = if user_request.trim().is_empty() {
        "the workflow we just went through in this conversation \u{2014} review \
         the steps taken and distill them into a reusable skill"
    } else {
        user_request.trim()
    };

    format!(
        "[/learn] The user wants you to learn a reusable skill from the \
         request below, and save it.\n\n\
         THE REQUEST:\n{req}\n\n\
         The request is open-ended and may mix two kinds of content, in any \
         order: SOURCES to gather (directories, file paths, URLs, \"what we \
         just did\", pasted notes) AND REQUIREMENTS that shape the skill \
         (what to focus on, what to leave out, scope, naming, the angle to \
         take). Treat EVERY part of the request as load-bearing. In \
         particular, prose that comes after a path or link is NOT incidental \
         \u{2014} it is the user telling you what they want from that source. A \
         request like `<url> focus on the auth flow, skip the deprecated \
         endpoints` means: gather the URL AND honor \"focus on auth, skip \
         deprecated\" as authoring requirements. Never fetch the first source \
         and ignore the rest.\n\n\
         Do this:\n\
         1. Gather every source the user named, using the tools you already \
         have \u{2014} `read_file`/`search_files` for local files or directories, \
         `web_extract` for URLs, the current conversation history if they \
         referred to something you just did, and the text they pasted as-is. \
         If the request is ambiguous about scope, make a reasonable choice \
         and note it; do not stall.\n\
         1b. Apply every requirement, focus, and constraint in the request to \
         the skill you author \u{2014} these govern what the SKILL.md covers and \
         emphasizes, not just which sources you read.\n\
         2. Author ONE SKILL.md and save it with the `skill_manage` tool \
         (action=\"create\"). Pick a sensible category. If the procedure needs \
         a non-trivial script, add it under the skill's `scripts/` with \
         `skill_manage` write_file and reference it by relative path.\n\n\
         {}\n\n\
         When done, tell the user the skill name, its category, and a \
         one-line summary of what it captured.",
        authoring_standards()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_learn_prompt_with_request() {
        let prompt = build_learn_prompt("learn how to deploy to AWS");
        assert!(prompt.contains("[/learn]"));
        assert!(prompt.contains("deploy to AWS"));
        assert!(prompt.contains("skill_manage"));
        assert!(prompt.contains("Frontmatter:"));
    }

    #[test]
    fn test_build_learn_prompt_empty_request() {
        let prompt = build_learn_prompt("");
        assert!(prompt.contains("[/learn]"));
        assert!(prompt.contains("the workflow we just went through"));
    }

    #[test]
    fn test_build_learn_prompt_whitespace_only() {
        let prompt = build_learn_prompt("   \n  ");
        assert!(prompt.contains("the workflow we just went through"));
    }

    #[test]
    fn test_build_learn_prompt_contains_standards() {
        let prompt = build_learn_prompt("test");
        assert!(prompt.contains("<=60 characters"));
        assert!(prompt.contains("lowercase-hyphenated"));
        assert!(prompt.contains("HARDLINE"));
    }

    #[test]
    fn test_build_learn_prompt_mentions_tool_framing() {
        let prompt = build_learn_prompt("test");
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("search_files"));
        assert!(prompt.contains("web_extract"));
    }
}
