# Family 02 — command safety and shell parsing

**Question this file answers:** codex knows what a shell command *means* before it judges it.
operant judges the string. How deep is that gap, and where does the token-based layer already
exist in operant?

**Ground state:** operant `main` @ `f53907e8`. Reference `../external-repos/codex/codex-rs` @ `588a277b`.

---

## CODEX-SIDE

### What the mechanism is

codex's safety analysis runs **after tokenization**, and tokenization is shell-aware. Two entry
points in `shell-command/src/bash.rs`:

```rust
// bash.rs:106-119
pub fn extract_bash_command(command: &[String]) -> Option<(&str, &str)> {
    let [shell, flag, script] = command else { return None };
    if !matches!(flag.as_str(), "-lc" | "-c") || !matches!(
        detect_shell_type(PathBuf::from(shell)),
        Some(ShellType::Zsh) | Some(ShellType::Bash) | Some(ShellType::Sh)) { return None }
    Some((shell, script))
}

// bash.rs:13-20
pub fn try_parse_shell(shell_lc_arg: &str) -> Option<Tree> {
    let lang = BASH.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).expect("load bash grammar");
    parser.parse(shell_lc_arg, None)
}
```

So the pipeline is: **unwrap the interpreter** (`bash -c "…"`, `zsh -lc "…"`) → **tree-sitter-parse
the script** → walk every `command` node → extract the *literal* commands → classify each one.

`is_dangerous_command.rs` (`shell-command/src/command_safety/is_dangerous_command.rs`, 332 LOC)
then classifies **a token vector**, not a string:

```rust
// is_dangerous_command.rs:141-159 — the whole match, quoted
match cmd0.as_deref() {
    Some("rm") if rm_args_include_force_option(&command[1..]) => Some(DangerousCommandMatch::ForcedRm),
    Some("sudo")  => dangerous_command_match_with_depth(&command[1..], wrapper_depth + 1, platform),  // unwrap sudo
    Some("env")   => dangerous_command_match_for_env(command, wrapper_depth, platform),               // skip env assignments
    Some("trap")  => dangerous_command_match_for_trap(command, wrapper_depth, platform),              // trap holds shell source
    _ => None,
}

// is_dangerous_command.rs:205-214 — flag-cluster logic
fn rm_args_include_force_option(args: &[String]) -> bool {
    args.iter().take_while(|arg| arg.as_str() != "--").any(|arg| {
        arg == "--force" || arg.strip_prefix('-').is_some_and(|flags| !flags.starts_with('-') && flags.contains('f'))
    })
}
```

Wrapper depth is capped at 8 (`MAX_DANGEROUS_COMMAND_WRAPPER_DEPTH`, `:36`). Beyond 8 it
**defaults to `Other`** — fail-closed, not fail-open (`:59-61`).

`try_parse_word_only_commands_sequence` (`bash.rs:29+`) is the strict variant: it returns `Some`
only when the script contains **word-only commands joined by `&&`, `||`, `;`, `|`** and the parse
tree has no error node — so parentheses, redirections, substitutions and control flow all force
`None`. That is the "I can reason about this script, or I refuse to" contract.

`parse_command.rs` (2,766 LOC) tokenizes both bash and PowerShell (`shlex::split`) for display and
replay.

### The load-bearing units (file:line + wc -l)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `dangerous_command_match*` | `shell-command/src/command_safety/is_dangerous_command.rs:37-215` | 332 (file) | token-vector classification, sudo/env/trap unwrap, depth cap |
| `extract_bash_command` | `shell-command/src/bash.rs:106-119` | 14 | the interpreter unwrap |
| `try_parse_shell` | `shell-command/src/bash.rs:13-20` | 8 | tree-sitter parse |
| `parse_shell_lc_literal_commands` | `shell-command/src/bash.rs:136-160` | 25 | walk the tree, collect literal `command` nodes |
| `try_parse_word_only_commands_sequence` | `shell-command/src/bash.rs:29+` | ~60 | the strict "word-only" gate |
| `parse_command.rs` | `shell-command/src/parse_command.rs:1` | 2,766 | bash + PowerShell tokenization |

Dependencies: `tree-sitter`, `tree-sitter-bash`, `tree-sitter-powershell` (`shell-command/Cargo.toml:20-22`).

---

## OPERANT-SIDE

### What exists today — and it is more layered than a first reading suggests

There are **three** layers, and the token-based one already exists.

**Layer 1 — the hardline blocklist, string-based.**
`crates/operant-core/src/approval.rs:526-557`:

```rust
fn check_hardline_blocklist(command: &str) -> Option<&'static str> {
    let command_lower = command.to_lowercase();
    for (entry_idx, entry) in HARDLINE_BLOCKLIST.iter().enumerate() {
        for (pattern_idx, pattern) in entry.patterns.iter().enumerate() {
            let matched = if entry.is_regex {
                HARDFINE_REGEXES[entry_idx][pattern_idx].is_match(command)
            } else {
                command_lower.contains(&pattern.to_lowercase())
            };
            if matched { /* warn */ return Some(entry.category); }
        }
    }
}
```

The `rm` rules there are **literal strings** (`approval.rs:209`):

```rust
HardlineEntry {
    category: "PACKAGE_RISK",
    patterns: &["rm -rf /etc", "rm -rf /usr", "rm -rf /bin", "rm -rf /boot"],
    is_regex: false,
},
```

so a flag permutation — `rm -fr /etc`, `rm --force --recursive /etc`, `rm -r -f /etc` — does not
match this layer. And `check_dangerous_patterns` (`approval.rs:561-578`) is the same shape: regex
against the raw string, 47 of 61 patterns.

**Layer 2 — the blast-radius classifier, token-based, and it IS in the approval path.**
`crates/operant-core/src/blast_radius.rs`:

- `segments()` (`:596-624`) splits on `;`, `\n`, `&&`, `||`, `|`.
- `has_short_flag(token, c)` (`:200-205`) is **the same flag-cluster logic** as codex's
  `rm_args_include_force_option`:

  ```rust
  fn has_short_flag(token: &str, c: char) -> bool {
      if token.starts_with("--") { return false }
      token.strip_prefix('-').is_some_and(|rest| rest.contains(c))
  }
  ```
- `rate_segment` maps the verb to a `VerbKind` (`:185-195`): `rm|unlink|shred|srm|rmdir|truncate`
  → `Remove`, `chmod|chown|chgrp` → `RecursiveMeta`, `find` → `Finder`, `git` → `Vcs`,
  `fdisk|sfdisk|…` → `Format`.
- recursion detection (`:535-539`): `any(|f| has_short_flag(f, 'r') || f == "--recursive")`.
- `screen()` (`:811-842`) is called **from the approval guard** at `approval.rs:662` and can return
  `Refusal::DeniedPath` / `MissingJustification` / `InsufficientJustification`.

**Layer 3 — the interpretive wall, documented honestly.**
`blast_radius.rs:22` states that indirection through a script or an interpreter (`sh -c`,
`python -c`), and a variable expanded at runtime, is **deliberately not modelled**. That is
accurate and it is exactly the boundary codex crosses with `extract_bash_command` + tree-sitter.

### The gap, stated precisely

Measured, not assumed:

| case | codex | operant layer 1 (blocklist) | operant layer 2 (blast_radius) | net |
|---|---|---|---|---|
| `rm -rf /etc` | `ForcedRm` (token) | matches literal | `Unbounded`/`DeniedPath` | blocked |
| `rm -fr /etc` | `ForcedRm` | **no match** | recursive detected → classified | justification required |
| `rm --force --recursive /etc` | `ForcedRm` | **no match** | `--recursive` detected | justification required |
| `sudo rm -rf /etc` | unwraps `sudo`, then `ForcedRm` | matches literal substring regardless | classified | blocked |
| `bash -c "rm -rf /etc"` | **unwraps and classifies the inner script** | matches on the surface string only | segments see `bash` (no verb) → `None` | **the gap** |
| `env X=1 rm -rf /etc` | skips assignments, classifies `rm` | matches | classified | blocked |
| `trap 'rm -rf /' EXIT` | classifies the trap body | depends on the literal | `trap` unknown verb → `None` | the gap |

Two corrections to the intuitive reading, both from measurement:

1. **The flag-permutation hole is already closed by layer 2.** `blast_radius` has the exact
   flag-cluster logic, and `screen()` runs inside `approval.rs`'s enforced path
   (`approval.rs:662`). So the *severity* of the string-matching is lower than "the rules are
   literal strings" suggests — the second layer tokenizes.
2. **The real, unclosed hole is the interpreter boundary.** `blast_radius.rs:22` names it, codex
   closes it, and operant does not. `bash -c "rm -rf /etc"` is rated `None` by `segments()`
   because the verb it sees is `bash`.

### A second, independent defect found while measuring

`approval.rs:1329` is a **test that asserts a regex compiles when it does not**:

```rust
patterns: &["rm -rf ("], // unclosed group
```

The comment at `:526-530` says the table is "Precompiled at first use: a malformed pattern is
impossible here because the table would already have panicked." So the guard is that the table
panics at first use — and the test exercises exactly that path. That is a real guard, correctly
designed: a typo'd hardline pattern is a **startup panic**, not a silent non-match.

**Recorded as a design note, not a defect:** this means the "silent never-fires" failure mode that
codex's `match`/`not_match` load-time validation solves (`docs/audit/2026-10-30-codex-portability-ledger/02-safety-and-policy.md`
Row 2) **does not exist in operant**. The ledger row was refuted for this reason, and the
measurement confirms the refutation. Nothing to do here.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| Interpreter unwrap (`bash -c`, `zsh -lc`) | `bash.rs:106-119` (14) | none — `blast_radius.rs:22` names it as out of scope | **high** (a safety boundary) | yes, pure string matching | **IMPORT-SMALL** |
| Token-vector classification | `is_dangerous_command.rs:141-215` | `blast_radius.rs:185-205` has it | low | — | **ALREADY-HAVE** |
| Flag-cluster detection (`-rf`, `-fr`, `--force`) | `is_dangerous_command.rs:205-214` | `blast_radius.rs:200-205` identical shape | none | — | **ALREADY-HAVE** |
| `sudo` / `env` unwrap | `is_dangerous_command.rs:146-155` | none | med | yes, ~20 LOC | IMPORT-SMALL |
| tree-sitter script parsing | `bash.rs:13-20` + dep | none | med | no — **3 new deps** | **DO-NOT** unless the unwrap row lands |
| Strict "word-only commands" gate | `bash.rs:29+` | none | low | no | DO-NOT |

**The ponytail reading:** the token layer is already written, in `blast_radius`, in the right
place, and it is already called from the approval path. What is missing is the **14-line
interpreter unwrap** and the **20-line sudo/env wrapper skip**. Those two are `String`-level work
with no new dependency, and they close the one hole codex closes and operant names.

**Do NOT adopt tree-sitter for this.** It is three dependencies for a boundary that a 34-line
`shlex`-free unwrap closes for the common cases. The full tree-sitter path buys analysis of
arbitrary scripts (`$(rm -rf /)`, `for f in …; do …`) which is real but is a *different* order of
ambition than the hole, and the honest way to buy it is to refuse what cannot be unwrapped rather
than to parse everything.
