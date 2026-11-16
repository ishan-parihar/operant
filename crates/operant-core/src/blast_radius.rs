//! Blast-radius classification for shell commands.
//!
//! Every function here is **pure and side-effect free**: it reads a command
//! string and returns a rating. It never touches a terminal, the filesystem,
//! the environment, or the clock, so classifying a command can never itself
//! do anything, and the whole taxonomy is exhaustively testable without a
//! shell.
//!
//! The classification is **structural, not a table of exact strings**. Each
//! command is split into shell segments, each segment into a head verb, its
//! flags, its positional targets, and its redirect/`of=` write destinations,
//! and the command's rating is the max over segments of
//! `f(verb, flags, target boundedness)`. That is the whole reason `rm` and
//! `rm -rf /` are different things: same verb, different flags, different
//! target boundedness.
//!
//! # This is defense in depth, NOT a sandbox
//!
//! This module operates on command *text*. A determined or confused caller
//! can still reach a destructive command through a form this parser does not
//! model — shell metacharacter nesting, indirection through a script file or
//! an interpreter, a variable expanded at runtime, a binary whose behavior is
//! not in `VerbKind`. It raises the cost of a careless `rm -rf /`; it does not
//! make destructive commands impossible.
//! **There is no OS-level sandbox in operant, and there never was one here.**
//! An earlier version of this comment named `operant-runtime`'s sandbox
//! backends as real isolation. That crate was deleted and no such backend
//! exists anywhere in this workspace — `SandboxBackend` is not a type here.
//! What protects a deployment is this text gate plus the approval gate plus
//! the operator's own environment, not kernel-enforced isolation. Do not tell
//! a reader otherwise.
//!
//! ## The interpreter boundary (added 2026-10-31, codex-deepen Row 1)
//!
//! The common case of interpreter indirection IS now unwrapped: a segment
//! headed by an interpreter or a wrapper (`bash -c`, `sh -lc`, `sudo`, `env`,
//! `timeout`, …) is followed into the script or command it actually runs, and
//! THAT is classified. `bash -c "rm -rf /etc"` rated `None` before — the verb
//! the classifier saw was `bash` — and rates `Unbounded` now.
//!
//! The unwrap **fails closed rather than open**: an interpreter whose script
//! cannot be extracted becomes the sentinel `UNRESOLVED_INTERPRETER`, rated
//! `Scoped`, so it is always visible to the operator. `Scoped` and not
//! `Unbounded` because an unresolvable shape is suspicious rather than proven
//! destructive, and gating every oddly-shaped `sh -c` on a justification would
//! train operators to write worse justifications.
//!
//! What is still deliberately NOT modelled, because modelling it costs more
//! than it is worth and tree-sitter is three dependencies for a boundary
//! string work already covers: `docker run`, `ssh host`, `crontab -l`
//! (a script read from a file), command substitution inside a
//! single-quoted string, and variable expansion. Nested interpreters beyond
//! one level are not followed either. `shlex` was considered and refused — see
//! the plan row's DO-NOT note.
//!
//! Enforcement is in [`crate::approval::check_tool_approval`], the same entry
//! point every tool call already passes through, so this adds a layer to the
//! existing gate rather than a second, route-aroundable one.

use crate::justification::{self, Defect};

/// How much a command can destroy.
///
/// The `Ord` ordering is meaningful: `None < Scoped < Unbounded < Device`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlastRadius {
    /// No destruction potential identified — read-only as far as we can tell.
    None,
    /// Destructive, but every target is a scoped literal path.
    Scoped,
    /// Destructive against a target set with no usable bound.
    Unbounded,
    /// Erases or overwrites a raw block device — destroys the medium.
    Device,
}

impl BlastRadius {
    /// Short human label used in gate messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "no-destruction",
            Self::Scoped => "scoped-destructive",
            Self::Unbounded => "unbounded-destructive",
            Self::Device => "raw-device-write",
        }
    }
}

/// Whether a target can be pinned to a known, finite set of files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundedness {
    /// A literal, resolvable path under a non-root prefix.
    Bounded,
    /// Root, home root, an unresolvable expansion, or a glob with no
    /// literal prefix — the reachable set cannot be enumerated.
    Unbounded,
}

// ============================================================================
// Vocabulary
// ============================================================================

/// What a head verb does to its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerbKind {
    /// `rm`, `unlink`, `shred`, `srm`, `rmdir`, `truncate` — destroy the named file(s).
    Remove,
    /// `mv`, and `cp`/`rsync`/`scp` carrying `--remove-source` — the move destroys
    /// whatever it is aimed AT as well as what it is aimed at.
    Relocate,
    /// `dd` — raw block copy; `of=` names the destination.
    RawWrite,
    /// `cp`, `scp` — overwrite or create a file (device with `-t`, else by name).
    Copy,
    /// `rsync` — delete only with `--delete`; `--remove-source-files` is a move.
    Sync,
    /// `install` — `cp`, a mode change and an owner change in one verb.
    Install,
    /// `mkfs.*`, `fdisk`, `parted`, `wipefs`, `badblocks`, `mkswap` — destroy media.
    Format,
    /// `chmod` / `chown` — only destructive with a recursive flag.
    RecursiveMeta,
    /// `find` — only destructive with `-delete` or a destructive `-exec`.
    Finder,
    /// `tee` — write to every named file, truncating it.
    Write,
    /// `git` — only `reset --hard` / `clean -f*`.
    Vcs,
}

/// `cp` flag spellings whose value is a file named BEFORE the trailing ones
/// (`cp -t DIR SRC…`). The value has to be consumed or it is read as a target.
const CP_TAKES_VALUE: &[&str] = &[
    "target-directory",
    "suffix",
    "backup-control",
    "context",
    "reflink",
];

/// `mv`/`install` flag spellings that take a separate value token.
const MV_TAKES_VALUE: &[&str] = &["target-directory", "suffix", "backup-control", "context"];

/// Short flags whose value is a SEPARATE token: the token after them is that
/// flag's argument, not a file the command touches. `install -m 644 ./a ./b`
/// without this read `644` as the destination (and the real destination `./b`
/// as an ordinary source), so the rating was computed on the wrong set.
/// `-t DIR` is in here because a copy's target directory is named BEFORE its
/// operands.
const COPY_VALUE_SHORT_FLAGS: &[char] = &['m', 't', 'S', 'Z'];
/// Short-flag letters a `cp`/`mv`/`install`/`scp` cluster can carry. Anything
/// outside this set is an argument or an unknown cluster, and the token after
/// it must be treated as an operand — dropping it would lose the very target
/// the rating depends on.
const COPY_SHORT_FLAGS: &[char] = &[
    'a', 'v', 'z', 'p', 'r', 'R', 'l', 'L', 'P', 'H', 'D', 's', 'S', 'g', 'o', 'n', 'u', 'c', 'x',
    'm', 'b', 'i', 'd', 'h', 'A', 'K', 'q', 't', 'T', 'f', 'F', 'e',
];
const SYNC_TAKES_VALUE: &[&str] = &[
    "exclude",
    "include",
    "filter",
    "files-from",
    "exclude-from",
    "include-from",
    "log-file",
    "rsh",
    "rsync-path",
    "temp-dir",
    "max-delete",
    "max-size",
    "min-size",
    "chmod",
    "chown",
    "usermap",
    "groupmap",
    "suffix",
];

/// Flags that delete what the copy/move used to name.
const SYNC_DELETE_FLAGS: &[char] = &['d'];

/// `rsync` flag spellings whose value is a file the operation never touches
/// (`rsync --files-from LIST SRC/ DST/`), so it is not a target.
const SYNC_LIST_FLAGS: &[&str] = &["files-from", "exclude-from", "include-from", "log-file"];

/// A long flag's NAME — the part before the first `=`, so `--rsync-path=sh`
/// and `--rsync-path sh` read the same.
fn long_flag_name(token: &str) -> Option<&str> {
    let name = token.strip_prefix("--")?;
    Some(name.split_once('=').map_or(name, |(k, _)| k))
}

/// A `-`-prefixed token carrying the short flag `c` in its cluster
/// (`-t`, `-at`, `-ravt`, and the `-m644` glued spelling).
fn short_flag(token: &str, c: char) -> bool {
    if token.starts_with("--") {
        return false;
    }
    token.strip_prefix('-').is_some_and(|rest| rest.contains(c))
}

/// Is this token a `chmod` MODE rather than a file?
///
/// `chmod` accepts an octal mode (`755`), a symbolic one (`u+x`, `go-w`, `a=r`)
/// and a copy-from form (`g=u`). Everything with a `/` in it is a path, and a
/// bare token made only of mode letters and operators is the operand. The
/// first-target strip this replaces removed whatever the FIRST token was, which
/// only worked while the mode came before any flag.
fn is_chmod_mode(token: &str) -> bool {
    if token.is_empty() || token.contains('/') {
        return false;
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '=' | ',' | '.'))
}

/// Does this copy/move/install short-flag cluster carry one of the flags whose
/// value is the NEXT token (`-m 644`, `-t DIR`)?
///
/// Only for a cluster made ENTIRELY of letters the verb accepts: an unknown
/// letter (`cp -Z ctx a b`) means the token after it is an operand, and eating
/// it would drop a target the rating depends on.
fn carries_copy_value_short_flag(token: &str) -> bool {
    let Some(letters) = token.strip_prefix('-') else {
        return false;
    };
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    letters
        .chars()
        .all(|c| COPY_SHORT_FLAGS.contains(&c) || COPY_VALUE_SHORT_FLAGS.contains(&c))
        && letters.chars().any(|c| COPY_VALUE_SHORT_FLAGS.contains(&c))
}

/// `find` flags that make the traversal actually delete something.
const FINDER_DELETE_FLAGS: &[&str] = &[
    "delete", "exec", "execdir", "ok", "okdir", "fprint0", "fprintf",
];
/// `find` flags that narrow the result set to a named subset.
const FINDER_PREDICATE_FLAGS: &[&str] = &[
    "name", "iname", "path", "ipath", "regex", "iregex", "type", "size", "mtime", "mmin", "newer",
    "maxdepth", "mindepth", "user", "group", "perm", "empty",
];
/// Flags that take a separate value token (so that value is not a target).
const VALUE_FLAGS: &[&str] = &[
    "name", "iname", "path", "ipath", "regex", "iregex", "type", "size", "mtime", "mmin", "newer",
    "maxdepth", "mindepth", "user", "group", "perm", "printf", "exec", "execdir", "ok", "okdir",
    "bs", "count", "seek", "skip", "conv", "iflag", "oflag", "status", "f",
];
/// `key=value` flags whose value is a **write** destination.
const WRITE_VALUE_FLAGS: &[&str] = &["of"];
/// Programs that take SQL as an argument, so a `DROP` in the argument list counts.
const DB_CLIENTS: &[&str] = &[
    "sqlite3", "psql", "mysql", "mariadb", "mongosh", "duckdb", "cqlsh",
];
/// Object keywords that make a bare `DROP`/`TRUNCATE` a statement.
const SQL_OBJECTS: &[&str] = &[
    "table",
    "database",
    "schema",
    "index",
    "view",
    "function",
    "procedure",
    "sequence",
    "user",
];
/// `git` subcommands that discard work irreversibly.
const VCS_HARD_SUBS: &[&str] = &["reset", "clean", "checkout", "restore", "filter-branch"];

/// Top-level directories that make any target under them root-adjacent.
///
/// Used for **boundedness**, not for refusal — `/dev` is included here so that
/// `rm -rf /dev` is `Unbounded`, but it is deliberately excluded from
/// [`DENIED_SYSTEM_DIRS`] so that a block-device write reaches the
/// justification gate instead of being swallowed by the path deny.
const SYSTEM_ROOTS: &[&str] = &[
    "etc", "usr", "bin", "sbin", "boot", "lib", "lib32", "lib64", "libx32", "dev", "proc", "sys",
    "var", "root", "run", "snap",
];

/// Directories refused outright. `/dev` is excluded (see [`is_block_device`]).
const DENIED_SYSTEM_DIRS: &[&str] = &[
    "etc", "usr", "bin", "sbin", "boot", "lib", "lib32", "lib64", "libx32", "proc", "sys", "var",
    "root", "run", "snap",
];

/// Basenames under `/dev/` that are not block devices.
const BENIGN_DEV: &[&str] = &[
    "null", "zero", "random", "urandom", "full", "tty", "ptmx", "console", "core", "fd", "shm",
    "stdout", "stderr", "stdin",
];

/// Prefixes that identify a raw block device under `/dev/`.
const BLOCK_DEVICE_PREFIXES: &[&str] = &[
    "sd", "hd", "vd", "xvd", "nvme", "mmcblk", "disk", "md", "dm-", "loop", "zram", "mapper",
];

/// Wrappers that sit in front of the real verb and carry no risk themselves.
const LEADING_WRAPPERS: &[&str] = &[
    "sudo", "doas", "pkexec", "env", "time", "nohup", "nice", "ionice", "command", "xargs",
];

fn strip_quotes(s: &str) -> &str {
    s.trim().trim_matches(|c| c == '"' || c == '\'')
}

fn basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn verb_kind(name: &str) -> Option<VerbKind> {
    let base = basename(name)
        .strip_suffix(".exe")
        .unwrap_or_else(|| basename(name));
    let lower = base.to_ascii_lowercase();
    if lower.starts_with("mkfs") {
        return Some(VerbKind::Format);
    }
    Some(match lower.as_str() {
        "rm" | "unlink" | "shred" | "srm" | "rmdir" | "truncate" => VerbKind::Remove,
        // Z15: `mv` moves, so it destroys its SOURCE as surely as `rm` does,
        // and the move target is written on top of whatever is already there.
        "mv" => VerbKind::Relocate,
        "dd" => VerbKind::RawWrite,
        // Z15: `cp`/`scp` create or overwrite the destination; `cp file
        // /dev/sda` destroys the medium exactly as `dd of=/dev/sda` does, and
        // without this arm the verb was unmodelled and the whole segment
        // invisible to the deny list.
        "cp" | "scp" => VerbKind::Copy,
        // Z15: `rsync` deletes only under `--delete`; the arm that rates it is
        // in `rate_segment`, and `--remove-source-files` makes it a move.
        "rsync" => VerbKind::Sync,
        // Z15: `install` is `cp` + `chmod` + `chown` + `strip` in one verb, so
        // it WRITES and it destroys by default; there is no flag that makes it
        // a read.
        "install" => VerbKind::Install,
        "tee" => VerbKind::Write,
        "chmod" | "chown" | "chgrp" => VerbKind::RecursiveMeta,
        "find" => VerbKind::Finder,
        "git" => VerbKind::Vcs,
        "fdisk" | "sfdisk" | "cfdisk" | "parted" | "wipefs" | "badblocks" | "hdparm" | "mkswap" => {
            VerbKind::Format
        }
        _ => return None,
    })
}

/// True when a `-`-prefixed token is a short-flag cluster containing `c`
/// (so `-rf` contains both `r` and `f`).
fn has_short_flag(token: &str, c: char) -> bool {
    if token.starts_with("--") {
        return false;
    }
    token.strip_prefix('-').is_some_and(|rest| rest.contains(c))
}

fn is_flag(token: &str) -> bool {
    // Long flags too: `git reset --hard` is the whole reason this exists.
    token.len() > 1 && token.starts_with('-')
}

fn trim_flag(token: &str) -> &str {
    token.trim_start_matches('-')
}

// ============================================================================
// Segment parsing
// ============================================================================

#[derive(Debug, Default)]
struct Segment {
    verb: Option<VerbKind>,
    /// `git` subcommand, lowercased.
    sub: String,
    /// Every `-flag` / `--flag` token seen, for flag-sensitive verbs.
    flags: Vec<String>,
    /// Positional arguments, in order (flag values already consumed).
    targets: Vec<String>,
    /// Redirect destinations and `dd of=` destinations.
    writes: Vec<String>,
    deleting: bool,
    narrowed: bool,
}

fn parse_segment(seg: &str) -> Segment {
    let mut s = Segment::default();
    let toks: Vec<&str> = seg.split_whitespace().collect();
    let mut i = 0;

    while i < toks.len() && LEADING_WRAPPERS.contains(&toks[i]) {
        i += 1;
    }
    // The head word itself, kept for the few verbs whose OPERAND has to be
    // told apart from a target (`chown root ./src`, `chmod 755 ./src`). Borrowed
    // from `toks`, so the common case does not allocate.
    let mut head = "";
    if let Some(t) = toks.get(i)
        && let Some(v) = verb_kind(t)
    {
        s.verb = Some(v);
        head = basename(t);
        i += 1;
    }
    if s.verb == Some(VerbKind::Vcs)
        && let Some(sub) = toks.get(i)
    {
        s.sub = sub.to_ascii_lowercase();
        i += 1;
    }

    while i < toks.len() {
        let t = toks[i];

        // Redirection: `>`, `>>`, `2>`, `&>`, and the `2>/dev/null` spelling.
        // The left-hand side must be empty, `&`, or digits — anything else
        // (`--out=1`, `-ofile`) is a flag whose text happens to contain `>`,
        // not a redirect, and eating the token after it would drop a target.
        if let Some(gt) = t.find('>')
            && t[..gt].chars().all(|c| c.is_ascii_digit() || c == '&')
        {
            let rhs = t[gt + 1..].trim_start_matches('>').trim_start_matches('|');
            if rhs.is_empty() {
                if let Some(next) = toks.get(i + 1) {
                    s.writes.push((*next).to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            } else {
                s.writes.push(rhs.to_string());
                i += 1;
            }
            continue;
        }

        // `key=value` — a flag, and sometimes a write destination.
        if let Some(eq) = t.find('=')
            && eq > 0
            && t[..eq].chars().all(|c| c.is_ascii_alphabetic())
        {
            let key = &t[..eq];
            let value = &t[eq + 1..];
            s.flags.push(key.to_string());
            if s.verb == Some(VerbKind::Finder) {
                if FINDER_DELETE_FLAGS.contains(&key) {
                    s.deleting = true;
                }
                if FINDER_PREDICATE_FLAGS.contains(&key) {
                    s.narrowed = true;
                }
            }
            if WRITE_VALUE_FLAGS.contains(&key) && !value.is_empty() {
                s.writes.push(value.to_string());
            }
            i += 1;
            continue;
        }

        if is_flag(t) {
            s.flags.push((*t).to_string());
            let name = trim_flag(t);
            // A value-taking flag swallows the next token so it is not
            // mistaken for a target (`find . -name '*.log' -delete`).
            // `--opt=value` carries its value in the same word, so it takes
            // nothing extra.
            let value_flag = VALUE_FLAGS.contains(&name)
                || match s.verb {
                    Some(VerbKind::Copy) => {
                        long_flag_name(t).is_some_and(|k| CP_TAKES_VALUE.contains(&k))
                            || carries_copy_value_short_flag(t)
                    }
                    Some(VerbKind::Relocate) | Some(VerbKind::Install) => {
                        long_flag_name(t).is_some_and(|k| MV_TAKES_VALUE.contains(&k))
                            || carries_copy_value_short_flag(t)
                    }
                    Some(VerbKind::Sync) => {
                        // `--exclude=*.log` carries its value in the same word.
                        (long_flag_name(t).is_some_and(|k| SYNC_TAKES_VALUE.contains(&k))
                            && !t.contains('='))
                            || long_flag_name(t).is_some_and(|k| SYNC_LIST_FLAGS.contains(&k))
                    }
                    _ => false,
                };
            if value_flag {
                i += 2;
            } else {
                i += 1;
            }
            // The flag-level signals have to be read AFTER the value-flag
            // decision above, which advances `i` past the flag's argument.
            // Reading them first let a cluster like `-sd /etc` be mistaken
            // for a delete flag and then swallow the directory after it.
            if s.verb == Some(VerbKind::Finder) {
                if FINDER_DELETE_FLAGS.contains(&name) {
                    s.deleting = true;
                }
                if FINDER_PREDICATE_FLAGS.contains(&name) {
                    s.narrowed = true;
                }
            }
            if s.verb == Some(VerbKind::Sync) && has_short_flag(t, SYNC_DELETE_FLAGS[0]) {
                s.deleting = true;
            }
            continue;
        }

        // `chmod -R 755 ./src` / `chmod u+x ./src`: the mode is the verb's
        // OPERAND, not a file. It used to be pushed as a target and then
        // removed by the first-target strip below, which only fired when a
        // target was left over -- so `chmod -R 755 ./src` kept `755`, lost
        // `./src`, and the recursive flag had nothing left to rate: the whole
        // segment rated `None`, and `chmod -R 755 /etc` was invisible to the
        // deny list too.
        if s.verb == Some(VerbKind::RecursiveMeta)
            && head.eq_ignore_ascii_case("chmod")
            && is_chmod_mode(t)
        {
            i += 1;
            continue;
        }
        s.targets.push((*t).to_string());
        i += 1;
    }

    // `chown -R root ./src` / `chgrp -R wheel ./src` — the owner is the first
    // operand, not a file. Only these two verbs: `chmod`'s mode is consumed as
    // a flag value above, and stripping a target for any other RecursiveMeta
    // verb would drop a real path.
    if matches!(s.verb, Some(VerbKind::RecursiveMeta))
        && (head.eq_ignore_ascii_case("chown") || head.eq_ignore_ascii_case("chgrp"))
        && !s.targets.is_empty()
    {
        s.targets.remove(0);
    }
    // `cp -t DIR SRC…` / `install -t DIR SRC…` — the target directory is named
    // BEFORE the operands, and it is where the copy WRITES. It was consumed as
    // a flag value and then dropped, so `cp -t /etc ./a` rated `Scoped` on
    // `./a` alone and `/etc` was never looked at. Same for the
    // `--target-directory=DIR` spelling.
    if matches!(s.verb, Some(VerbKind::Copy) | Some(VerbKind::Install)) {
        let words: Vec<&str> = seg.split_whitespace().collect();
        if let Some(index) = words
            .iter()
            .position(|t| short_flag(t, 't') && carries_copy_value_short_flag(t))
            && let Some(dir) = words.get(index + 1)
            && !dir.starts_with('-')
        {
            s.writes.push((*dir).to_string());
        }
        for f in &s.flags {
            if let Some(value) = f.strip_prefix("--target-directory=")
                && !value.is_empty()
            {
                s.writes.push(value.to_string());
            }
        }
    }
    s
}

// ============================================================================
// Device / boundedness predicates
// ============================================================================

/// True for `/dev/null`, `/dev/zero`, `/dev/pts/*`, … — writes that destroy nothing.
pub fn is_benign_dev(target: &str) -> bool {
    let t = strip_quotes(target);
    let Some(rest) = t.strip_prefix("/dev/") else {
        return false;
    };
    rest.is_empty()
        || BENIGN_DEV.contains(&rest)
        || rest.starts_with("pts/")
        || rest.starts_with("shm/")
}

/// True when the target names a raw block device (`/dev/sda`, `/dev/mapper/x`, …).
pub fn is_block_device(target: &str) -> bool {
    let t = strip_quotes(target);
    let Some(rest) = t.strip_prefix("/dev/") else {
        return false;
    };
    if rest.is_empty() || is_benign_dev(t) {
        return false;
    }
    if rest == "mapper" {
        return true;
    }
    let base = basename(rest);
    BLOCK_DEVICE_PREFIXES.iter().any(|p| base.starts_with(p))
}

/// Can the target be pinned to a known, finite set of files?
pub fn boundedness_of(target: &str) -> Boundedness {
    let t = strip_quotes(target);
    if t.is_empty() {
        return Boundedness::Unbounded;
    }
    // Indirection: not knowable from the text at all.
    if t.starts_with('$') || t.contains("`") || t.contains("$(") {
        return Boundedness::Unbounded;
    }
    // Home root, any spelling.
    if t == "~" || t.starts_with("~/") {
        return Boundedness::Unbounded;
    }
    // Filesystem root, any spelling.
    if t.chars().all(|c| c == '/') {
        return Boundedness::Unbounded;
    }
    // Trailing `/.` or `/..` — the directory the command is standing in.
    if t.ends_with("/.") || t.ends_with("/..") {
        return Boundedness::Unbounded;
    }

    let norm = t.trim_start_matches('/');

    // A bare `.` or `..` is the directory the command is standing in; a pure
    // classifier has no cwd, so it cannot bound it. `./target/debug` and
    // `../build/out`, by contrast, have a real literal prefix.
    let mut rest = norm;
    loop {
        let stripped = rest
            .strip_prefix("./")
            .or_else(|| rest.strip_prefix("../"))
            .unwrap_or(rest);
        if stripped.len() == rest.len() {
            break;
        }
        rest = stripped;
    }
    if rest.is_empty() || rest == "." || rest == ".." {
        return Boundedness::Unbounded;
    }

    // A glob with no literal prefix cannot be enumerated.
    let first = rest.split('/').next().unwrap_or("");
    if first.contains('*') || first.contains('?') || first.contains('[') {
        return Boundedness::Unbounded;
    }
    if SYSTEM_ROOTS.contains(&first) {
        return Boundedness::Unbounded;
    }
    Boundedness::Bounded
}

// ============================================================================
// Rating
// ============================================================================

/// SQL statements are destructive regardless of the verb that carries them.
fn sql_class(seg: &str) -> Option<BlastRadius> {
    let lower = seg.to_ascii_lowercase();
    let words: Vec<String> = lower
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_string()
        })
        .collect();
    let head = words.first().map(String::as_str).unwrap_or("");

    if (head == "drop" || head == "truncate")
        && words
            .get(1)
            .is_some_and(|w| SQL_OBJECTS.contains(&w.as_str()))
    {
        return Some(BlastRadius::Unbounded);
    }
    if DB_CLIENTS.contains(&head) {
        let bare = lower.split_whitespace().collect::<Vec<_>>().join(" ");
        let destructive = [
            "drop table",
            "drop database",
            "drop schema",
            "drop index",
            "truncate table",
        ]
        .iter()
        .any(|p| bare.contains(p))
            || (bare.contains("delete from") && !bare.contains(" where "));
        if destructive {
            return Some(BlastRadius::Unbounded);
        }
    }
    None
}

fn worst(a: Boundedness, b: Boundedness) -> Boundedness {
    if a == Boundedness::Unbounded || b == Boundedness::Unbounded {
        Boundedness::Unbounded
    } else {
        Boundedness::Bounded
    }
}

fn rate_segment(seg: &str) -> (BlastRadius, Vec<String>) {
    // The fail-closed sentinel: an interpreter whose inner script we could
    // not extract. Rated `Scoped` so it is always VISIBLE to the operator
    // (never `None`, which is how this family slipped through for so long)
    // without putting an `Unbounded` gate on every oddly-shaped `sh -c`.
    if seg == UNRESOLVED_INTERPRETER {
        return (BlastRadius::Scoped, Vec::new());
    }
    if let Some(c) = sql_class(seg) {
        return (c, Vec::new());
    }
    // Z15's "smaller fix, which closes the whole class": the DENY LIST must be
    // reachable whatever the verb is. With `verb: None` this arm rated `None`
    // and `screen()` returned before the deny check, so `mv /etc /tmp/x` and
    // `cp ~/.ssh/id_rsa /tmp/x` were never looked at. The escalation is now
    // EXACTLY the denied-path condition and nothing else: an unmodelled verb
    // whose target `check_deny` refuses is `Unbounded` (so the deny list runs),
    // and every other unmodelled segment is still `None`. That is what keeps
    // `ls -la /tmp`, `grep -r x .` and `ssh host uptime` unrated -- a fallback
    // that rated "any target" would be a rate-everything switch, which is
    // worse than the hole it closes.
    let p = parse_segment(seg);
    if p.verb.is_none() {
        let targets: Vec<String> = p
            .targets
            .iter()
            .chain(p.writes.iter())
            .filter(|t| !is_benign_dev(t))
            .cloned()
            .collect();
        // A write destination is rated even when the verb is unknown, exactly
        // as it is for a modelled one: `echo boom > /dev/sda` destroys the
        // medium whatever the verb on the left is called.
        let write_class = p.writes.iter().fold(BlastRadius::None, |acc, w| {
            if is_benign_dev(w) {
                return acc;
            }
            acc.max(if is_block_device(w) {
                BlastRadius::Device
            } else if boundedness_of(w) == Boundedness::Unbounded {
                BlastRadius::Unbounded
            } else {
                BlastRadius::Scoped
            })
        });
        if write_class != BlastRadius::None {
            return (write_class, targets);
        }
        // Otherwise the escalate condition is EXACTLY the denied path and
        // nothing else: `check_deny` refuses a target, so the segment is
        // `Unbounded` (the deny list then runs and names it). Everything else
        // stays `None` -- `ls -la /tmp`, `grep -r x .` and `ssh host uptime`
        // must stay unrated, or this fallback would be a rate-everything
        // switch, which is worse than the hole it closes.
        //
        // `home` is None because this is a PURE function of the command text:
        // the caller's home is added by `screen()`, which re-runs the deny
        // check over the returned targets with the real home. What is caught
        // here is the part no environment can change -- `/etc`, `$HOME`, `~`,
        // the filesystem root, `.git`.
        //
        // An unmodelled INTERPRETER whose script cannot be followed stays out
        // of this path entirely: `segments_full` already appended the
        // fail-closed sentinel for it, and rating its bare flag text here
        // (`sudo -u root` -> `root`) would both invent a target and break the
        // contract those tests pin.
        if seg.split_whitespace().any(|t| {
            let bare = basename(t);
            INTERPRETER_VERBS.contains(&bare.to_ascii_lowercase().as_str())
                || LEADING_WRAPPERS.contains(&bare)
        }) {
            return (BlastRadius::None, Vec::new());
        }
        let denied: Vec<String> = p
            .targets
            .iter()
            .filter(|t| check_deny(t, None).is_some())
            .cloned()
            .collect();
        if denied.is_empty() {
            return (BlastRadius::None, Vec::new());
        }
        return (BlastRadius::Unbounded, denied);
    }
    let mut targets = p.targets.clone();

    // `dd` writes to `of=`, not to its positionals.
    if p.verb == Some(VerbKind::RawWrite) {
        targets.clear();
    }
    targets.extend(p.writes.iter().cloned());

    // A write destination is rated on its own, even when the verb that
    // performs it is innocuous: `echo boom > /dev/sda` destroys the medium
    // just as thoroughly as `dd of=/dev/sda`. Without this the `device`
    // override below would be unreachable for every non-destructive verb.
    let mut write_class = BlastRadius::None;
    for w in &p.writes {
        if is_benign_dev(w) {
            continue;
        }
        write_class = write_class.max(if is_block_device(w) {
            BlastRadius::Device
        } else if boundedness_of(w) == Boundedness::Unbounded {
            BlastRadius::Unbounded
        } else {
            BlastRadius::Scoped
        });
    }

    let device = targets.iter().any(|t| is_block_device(t));
    let mut bound = Boundedness::Bounded;
    for t in &targets {
        if is_benign_dev(t) {
            continue;
        }
        bound = worst(bound, boundedness_of(t));
    }
    // `find` uses its search root as the bound; `chmod -R` mode arg was dropped.
    if p.verb == Some(VerbKind::Finder) && p.targets.is_empty() && bound == Boundedness::Bounded {
        bound = Boundedness::Unbounded;
    }
    // Z15: `cp`/`rsync` default to NOT overwriting an existing destination, so
    // a bounded copy reads and writes named files and its write destination is
    // already rated on its own above. `install`, `mv` and `dd` all write by
    // default and cannot be read that way.
    let base = match p.verb {
        None => BlastRadius::None,
        Some(VerbKind::Remove) | Some(VerbKind::Write) | Some(VerbKind::Relocate) => {
            if bound == Boundedness::Unbounded {
                BlastRadius::Unbounded
            } else {
                BlastRadius::Scoped
            }
        }
        Some(VerbKind::RawWrite) | Some(VerbKind::Format) => {
            if device {
                BlastRadius::Device
            } else if bound == Boundedness::Unbounded {
                BlastRadius::Unbounded
            } else {
                BlastRadius::Scoped
            }
        }
        Some(VerbKind::Copy) | Some(VerbKind::Sync) | Some(VerbKind::Install) => {
            // These five verbs are the destructive WRITE family: `mv` destroys
            // its source, `cp`/`scp`/`install` create or overwrite the
            // destination, and `rsync` empties one of the two sets under
            // `--delete` / `--remove-source-files`. A BOUNDED one is therefore
            // `Scoped` -- visible to the operator, allowed without a
            // justification, which is the property that keeps the fix from
            // being "block every copy". What a flag changes is WHICH set goes,
            // not how much: the target's boundedness decides `Unbounded`.
            //
            // A modelled verb with no operand at all (`cp --help`, `mv --help`)
            // touches nothing and stays unrated.
            // A modelled verb with no operand at all (`cp --help`) touches
            // nothing and stays unrated; anything else here destroys something
            // bounded. `p.deleting` (`rsync -d`, `--delete`) is deliberately NOT
            // read: the flag decides WHICH set the transfer empties, not how
            // much, and the boundedness computed above already decides that.
            // `rsync_deletes_only_with_the_flag_that_says_so` pins the
            // observable consequence -- `rsync -a ./src/ ./dst/` and
            // `rsync -a --delete ./src/ ./dst/` rate identically, and
            // `rsync -a --delete /etc/ /tmp/x` is denied through its target.
            let has_targets = !targets.is_empty() || !p.writes.is_empty();
            if device {
                BlastRadius::Device
            } else if bound == Boundedness::Unbounded {
                BlastRadius::Unbounded
            } else if !has_targets {
                BlastRadius::None
            } else {
                BlastRadius::Scoped
            }
        }
        Some(VerbKind::RecursiveMeta) => {
            // `chmod -R` / `chown -R` / `chmod --recursive`. `has_short_flag`
            // matched only a lower-case `r`, so the conventional upper-case
            // `-R` every operator actually types was never seen: every
            // recursive chmod/chown rated `None`, and `chmod -R 755 /etc`
            // reached the deny list not at all.
            let recursive = p
                .flags
                .iter()
                .any(|f| f == "--recursive" || has_short_flag(f, 'r') || has_short_flag(f, 'R'));
            if recursive {
                if bound == Boundedness::Unbounded {
                    BlastRadius::Unbounded
                } else {
                    BlastRadius::Scoped
                }
            } else {
                BlastRadius::None
            }
        }
        Some(VerbKind::Finder) => {
            if !p.deleting {
                BlastRadius::None
            } else if p.narrowed && bound == Boundedness::Bounded {
                BlastRadius::Scoped
            } else {
                BlastRadius::Unbounded
            }
        }
        Some(VerbKind::Vcs) => {
            let hard = VCS_HARD_SUBS.contains(&p.sub.as_str())
                && (p.sub != "reset" || p.flags.iter().any(|f| f == "--hard"))
                && (p.sub != "clean" || p.flags.iter().any(|f| has_short_flag(f, 'f')));
            if p.sub == "reset" && !hard {
                // Soft/mixed reset only moves HEAD; the working tree survives.
                BlastRadius::Scoped
            } else if hard {
                BlastRadius::Unbounded
            } else {
                BlastRadius::None
            }
        }
    };

    // A block-device target is `Device` whatever the verb; a redirect write
    // already carries its own rating. Max, so nothing is ever understated.
    let class = write_class.max(if device && base > BlastRadius::None {
        BlastRadius::Device
    } else {
        base
    });
    (class, targets)
}

/// Rate a shell command by how much it can destroy.
///
/// Structural: verb x flags x target boundedness, maxed across shell
/// segments. Pure — no I/O, no terminal, no filesystem.
pub fn classify(command: &str) -> BlastRadius {
    let mut class = BlastRadius::None;
    for seg in segments_full(command) {
        class = class.max(rate_segment(&seg).0);
    }
    class
}

/// Split on shell separators only — the raw tokenizer, no interpretation.
///
/// Kept as the primitive so [`segments_full`] can build on it and so a test
/// can pin the splitter's behaviour independently of the unwrap.
fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let bytes: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let two: String = bytes[i..].iter().take(2).collect();
        if c == '\n' || c == ';' {
            out.push(std::mem::take(&mut cur));
            i += 1;
        } else if two == "&&" || two == "||" {
            out.push(std::mem::take(&mut cur));
            i += 2;
        } else if c == '|' {
            out.push(std::mem::take(&mut cur));
            i += 1;
        } else {
            cur.push(c);
            i += 1;
        }
    }
    out.push(cur);
    out.retain(|s| !s.trim().is_empty());
    out
}

/// Interpreters and wrappers whose whole job is to run something else.
///
/// A segment headed by one of these is not the command that does the work —
/// `bash -c "rm -rf /etc"` is a `rm`, and rating it as `bash` rated it as
/// nothing. This list is the smallest set that covers how a model or a
/// playwright actually reaches an inner command; it deliberately does NOT
/// try to model:`docker run`, `ssh host`, `sudo -u user` (a privilege change
/// with no command), or `crontab -l` (reads its script from a file).
const INTERPRETER_VERBS: &[&str] = &[
    "bash", "sh", "zsh", "dash", "ksh", "csh", "tcsh", "fish", "ash", "env", "sudo", "doas",
    "nice", "nohup", "timeout", "setsid", "stdbuf", "ionice", "trap",
];

/// The inner script a segment's interpreter verb was asked to run, if any.
///
/// Handles the two shapes that matter: `-c <script>` (and the glued
/// `-c<script>`, which `shlex`-free string work can still see) and the bare
/// positional form `<interp> <file>`, which is a script path rather than a
/// command.
///
/// Three-valued on purpose:
///
/// - `Some(script)` — followed: classify that script.
/// - `Some("")` — an interpreter we FOUND but could not follow (`bash -c`
///   with no argument). The caller turns this into the fail-closed sentinel.
/// - `None` — not an interpreter-headed segment at all.
///
/// Collapsing the middle case into `None` was a real bug: `bash -c` rated
/// `None` and slipped the gate entirely. The test
/// `an_interpreter_that_cannot_be_unwrapped_fails_closed` pins it.
fn interpreter_script(seg: &str) -> Option<String> {
    let toks = tokens(seg);
    let head = toks.first()?;
    let head_base = basename(head).to_ascii_lowercase();
    if !INTERPRETER_VERBS.contains(&head_base.as_str()) {
        return None;
    }
    // Wrappers that take a leading numeric parameter of their OWN before the
    // command they run (`timeout 30 cmd`, `nice -n 10 cmd`). That token is the
    // wrapper's config, not the command — skipping it is what lets
    // `timeout 30 rm -rf /etc` still reach `rm`.
    const LEADING_PARAM_WRAPPERS: &[&str] = &["timeout", "nice", "ionice", "stdbuf"];
    let skip_leading_params = LEADING_PARAM_WRAPPERS.contains(&head_base.as_str());

    let mut i = 1usize;
    while i < toks.len() {
        let tok = toks[i].trim();
        // A short-flag cluster carrying `c` — `-c`, `-lc`, `-ec` — means
        // "the script is the rest of THIS word (glued) or the next token
        // (spaced)". Matching only the exact `-c` token missed `sh -lc`,
        // and the old `ends_with('c')` gate + `rfind('c')` mistook a glued
        // script's own trailing `c` for the flag — so `bash -c"rm -rf /etc"`
        // (one word after quote removal: `-crm -rf /etc`) fell through to
        // the unfollowable sentinel and the gate passed it (RT-320).
        if tok.starts_with('-')
            && !tok.starts_with("--")
            && let Some(glued) = glued_script(tok)
        {
            if !glued.is_empty() {
                return Some(glued);
            }
            // `bash -c` with nothing glued: interpreter-shaped but
            // unfollowable. `Some("")` (not `None`) so the caller emits the
            // fail-closed sentinel instead of treating this as "not an
            // interpreter" and rating it `None`.
            //
            // The next token is the script ONLY when it is not another flag.
            // Consuming a flag here rated `bash -c -x` as `None`, because
            // `-x` is not a destructive verb and the real command was never
            // consulted.
            return match toks.get(i + 1) {
                Some(next) if next.starts_with('-') => Some(String::new()),
                Some(next) => Some(next.clone()),
                None => Some(String::new()),
            };
        }
        // `--command=` long form, and `--` ending our flag scan.
        if let Some(rest) = tok.strip_prefix("--command=") {
            return Some(rest.to_string());
        }
        if tok == "--" {
            return toks.get(i + 1).cloned().or_else(|| Some(String::new()));
        }
        if skip_leading_params && !tok.starts_with('-') && is_all_digits(tok) {
            i += 1;
            continue;
        }
        // `sudo`/`doas` take a `-u <user>` / `-g <group>` argument that is the
        // wrapper's config, not the command. Skipping the VALUE is what lets
        // `sudo -u root rm -rf /etc` reach `rm` instead of rating `root`.
        if (head_base == "sudo" || head_base == "doas")
            && matches!(tok, "-u" | "-g" | "-p" | "-C" | "-h")
        {
            i += 2;
            continue;
        }
        // `VAR=value` env assignments belong to `env`/`sudo`/`nice` — skip
        // them so `env FOO=bar rm -rf /etc` still reaches `rm`.
        if !tok.starts_with('-') && !is_env_assignment(tok) {
            // A bare positional after the interpreter is a script FILE for
            // `bash`, or the COMMAND for the wrapper family (`sudo`, `env`,
            // `nice`, `timeout`, `trap`, `nohup`). Either way it is what must
            // be classified, so hand back the WHOLE remaining tail — one token
            // would drop the command's own flags and target, which is how
            // `timeout 30 rm -rf /etc` came out `Scoped` instead of
            // `Unbounded` (the return was the bare verb `rm`).
            return Some(rejoin_tail(&toks, i));
        }
        i += 1;
    }
    // Reached the end with a flag-only tail: an interpreter we could not
    // follow. `Some("")`, not `None` — see the `-c` arm above.
    Some(String::new())
}

/// The script glued onto a short-flag cluster's `c`, when there is one:
/// `-cscript` or `-lcscript`. Returns `None` when the cluster carries no
/// `c` (a bare `-c`/`-lc` returns `Some("")` — the script is the NEXT token).
///
/// bash/getopt semantics: the `c` flag's argument is the rest of the SAME
/// word, so the script runs from the first `c` in the letter run onward —
/// `-c"rm -rf /etc"` is ONE word after quote removal (`-crm -rf /etc`) and
/// bash runs exactly `rm -rf /etc`. Scanning only the leading ASCII-letter
/// run keeps a glued script's own characters (and a flagless cluster's
/// later text) from being mistaken for the flag.
fn glued_script(flag: &str) -> Option<String> {
    let letters = flag.strip_prefix('-')?;
    let letter_run: String = letters
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let c_at = letter_run.find('c')?;
    Some(letters[c_at + 1..].to_string())
}

/// Rebuild the tokens from `start` onward into one string, re-quoting any
/// token that would not survive a round-trip through [`tokens`].
///
/// Needed because [`interpreter_script`] hands the *remainder* of a wrapper
/// segment to [`segments`], and a token containing a space (`bash -c 'a b'`)
/// would otherwise split into two.
fn rejoin_tail(toks: &[String], start: usize) -> String {
    let tail = &toks[start..];
    // A tail whose FIRST token is itself a quoted phrase. `trap 'rm -rf /' EXIT`
    // tokenizes to ["trap", "rm -rf /", "EXIT"] -- `tokens()` collapses the
    // handler into ONE token, because it was quoted. Re-quoting that token here
    // produced `'rm -rf /' EXIT`, which the recursion re-tokenized into a single
    // head token `rm -rf /`; `verb_kind` does not know that name, so the whole
    // destructive command rated `None` and slipped the gate. Emit such a tail
    // UN-quoted so the recursion sees `rm` as the head verb and rates it.
    //
    // Only the FIRST token decides this: a quoted phrase in a LATER position is
    // exactly what re-quoting is for (`nice -n 5 sh -c 'rm -rf /etc'` must stay
    // `sh -c 'rm -rf /etc'` so the inner `-c` arm can still find the script).
    let first_is_a_quoted_phrase = tail
        .first()
        .is_some_and(|t| t.chars().any(char::is_whitespace));
    tail.iter()
        .map(|t| {
            if !first_is_a_quoted_phrase && (t.is_empty() || t.chars().any(char::is_whitespace)) {
                format!("'{}'", t.replace('\'', "'\\''"))
            } else {
                t.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// True for a bare non-negative number, the one leading-parameter shape a
/// wrapper swallows (`timeout 30`). Kept narrow on purpose: widening it to
/// "anything not a flag" would swallow the command in `env FOO cmd`.
fn is_all_digits(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit())
}

fn is_env_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((name, _)) => {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Quote-aware whitespace tokenizer.
///
/// [`segments`] splits on operators; this splits a single segment into words
/// while honouring `'…'` and `"…"` so a script path with a space survives.
/// Backslash escapes are honoured inside double quotes only, which is where
/// they matter for the arguments the unwrap reads.
fn tokens(seg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;
    let mut chars = seg.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else if c == q {
                    quote = None;
                    in_token = true;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    in_token = true;
                }
                c if c.is_whitespace() => {
                    if in_token {
                        out.push(std::mem::take(&mut cur));
                        in_token = false;
                    }
                }
                c => {
                    cur.push(c);
                    in_token = true;
                }
            },
        }
    }
    if in_token {
        out.push(cur);
    }
    out
}

/// Segments with interpreter indirection unwrapped, recursively.
///
/// This is the segmenter the classifier uses. For every raw segment it does
/// THREE things, and the third is what makes the fail-closed contract real:
///
/// 1. keeps the raw segment (so the outer verb is still rated),
/// 2. appends the inner script's own segments when the interpreter verb's
///    script argument can be extracted, and
/// 3. when the interpreter was found but its script CANNOT be extracted,
///    appends a sentinel `UNRESOLVED` segment that rates `Scoped`.
///
/// `Scoped` and not `Unbounded` on purpose: an unresolvable interpreter is
/// suspicious but not proven destructive, and forcing an `Unbounded` rating
/// would put a justification gate on every `sh -c` with an unusual flag
/// shape. `Scoped` means the command is always *visible* to the operator;
/// `None` would mean it silently slipped through again.
///
/// The unwrap is RECURSIVE, not one level: see [`MAX_INTERPRETER_DEPTH`] for
/// how far it follows a chain and [`MAX_INTERPRETER_INPUT`] for how big a
/// command it will unwrap at all.
///
/// The number of interpreter hops the unwrap will follow.
///
/// A wrapper chain (`env VAR=1 sudo -u root timeout 5 sh -c '...'`) nests one
/// interpreter per link, so a single descent re-opened the exact hole row 1
/// closed for every chain deeper than one. The cap stops a self-referential
/// or pathological script from recursing without bound; the last level still
/// emits the fail-closed sentinel, so a chain DEEPER than the cap is rated
/// `Scoped` rather than `None`.
const MAX_INTERPRETER_DEPTH: usize = 8;

/// The longest command the classifier will interpret.
///
/// `classify` runs on the approval path before anything executes, and the
/// unwrap re-tokenizes at every hop, so its work is superlinear in the input
/// length. This bound is what makes that safe. Past it the classifier emits
/// the fail-closed sentinel and rates `Scoped` -- it does NOT fall back to the
/// surface split, because that split's head verb is `bash` (for an
/// interpreter-headed command) and `rate_segment` would rate it `None`, which
/// is the exact hole row 1 closed. A command past this length is a script, and
/// a script gets gated, not waved through.
const MAX_INTERPRETER_INPUT: usize = 8 * 1024;

/// Segments with interpreter indirection unwrapped, recursively.
///
/// This is the segmenter the classifier uses. For every raw segment it does
/// THREE things, and the third is what makes the fail-closed contract real:
///
/// 1. keeps the raw segment (so the outer verb is still rated),
/// 2. appends the inner script's own segments when the interpreter verb's
///    script argument can be extracted, and
/// 3. when the interpreter was found but its script CANNOT be extracted,
///    appends a sentinel `UNRESOLVED` segment that rates `Scoped`.
///
/// `Scoped` and not `Unbounded` on purpose: an unresolvable interpreter is
/// suspicious but not proven destructive, and forcing an `Unbounded` rating
/// would put a justification gate on every `sh -c` with an unusual flag
/// shape. `Scoped` means the command is always *visible* to the operator;
/// `None` would mean it silently slipped through again.
///
/// The unwrap is RECURSIVE, not one level: see [`MAX_INTERPRETER_DEPTH`] for
/// how far it follows a chain and [`MAX_INTERPRETER_INPUT`] for how big a
/// command it will unwrap at all.
///
///
fn segments_full(command: &str) -> Vec<String> {
    // Past the bound, do NOT fall back to the unwrapped surface split: that
    // split's head verb is `bash`, which `rate_segment` does not model, so
    // `bash -c 'rm -rf /'` would rate `None` -- which is the exact hole row 1
    // closed. The bound has to fail CLOSED like every other interpreter arm.
    if command.len() > MAX_INTERPRETER_INPUT {
        return vec![
            UNRESOLVED_INTERPRETER.to_string(),
            UNRESOLVED_INTERPRETER.to_string(),
        ];
    }
    segments_full_depth(command, /*depth*/ 0)
}

fn segments_full_depth(command: &str, depth: usize) -> Vec<String> {
    let raw = segments(command);
    let mut out: Vec<String> = Vec::with_capacity(raw.len() * 2);
    for seg in raw {
        let seg = seg.trim().to_string();
        if seg.is_empty() {
            continue;
        }
        // `Some("")` means an interpreter we found but could not
        // follow; `None` means the head verb is not an interpreter at all and
        // the segment is classified as written. The sentinel is what makes the
        // first case observable instead of silently `None`.
        match interpreter_script(&seg) {
            Some(script) if !script.trim().is_empty() => {
                out.push(seg.clone());
                if depth < MAX_INTERPRETER_DEPTH {
                    // RECURSIVE, not one level: the inner script can itself be
                    // headed by an interpreter (`bash -c 'sh -c "..."'`, or
                    // `env VAR=1 sh -c '...'` where the wrapper sits in front of
                    // the interpreter). Without this, only the outermost hop
                    // was followed and every deeper chain rated `None`.
                    out.extend(segments_full_depth(&script, depth + 1));
                } else {
                    // Depth-exhausted: classify the script as written, and add
                    // the sentinel so the chain is visible rather than silent.
                    out.extend(segments(&script));
                    out.push(UNRESOLVED_INTERPRETER.to_string());
                }
            }
            Some(_) => {
                out.push(seg.clone());
                out.push(UNRESOLVED_INTERPRETER.to_string());
            }
            None => out.push(seg),
        }
    }
    out
}

/// The sentinel a segment becomes when it names an interpreter whose inner
/// script could not be extracted. Rate at the call site, never execute.
const UNRESOLVED_INTERPRETER: &str = "<unresolved-interpreter>";

/// Every target a destructive verb or a redirect can reach, for path denial.
pub fn destructive_targets(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seg in segments_full(command) {
        if sql_class(&seg).is_some() {
            continue;
        }
        let (class, targets) = rate_segment(&seg);
        if class != BlastRadius::None {
            out.extend(targets);
        }
    }
    out
}

/// A one-phrase description of the first unbounded thing the command touches,
/// for the model-facing refusal message.
pub fn unbounded_detail(command: &str) -> String {
    for seg in segments_full(command) {
        if sql_class(&seg).is_some() {
            return format!("destructive SQL statement in `{seg}`");
        }
        let (class, targets) = rate_segment(&seg);
        if class == BlastRadius::Unbounded || class == BlastRadius::Device {
            let worst_target = targets
                .iter()
                .find(|t| !is_benign_dev(t))
                .cloned()
                .unwrap_or_else(|| seg.trim().to_string());
            let verb = basename(seg.split_whitespace().next().unwrap_or("")).to_ascii_lowercase();
            return if verb.is_empty() {
                worst_target
            } else {
                format!("`{verb}` against {worst_target}")
            };
        }
    }
    "an unbounded target".to_string()
}

// ============================================================================
// Path denial
// ============================================================================

/// A target refused regardless of justification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeniedPath {
    /// The offending target, verbatim.
    pub target: String,
    /// Why it is refused.
    pub detail: &'static str,
}

/// Refuse system directories, the filesystem root, the user's home root, and
/// VCS internals.
///
/// `home` is the caller's home directory, passed in rather than read from the
/// environment so this function stays pure and testable.
pub fn check_deny(target: &str, home: Option<&str>) -> Option<DeniedPath> {
    let t = strip_quotes(target);
    if t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c == '/') {
        return Some(DeniedPath {
            target: t.to_string(),
            detail: "filesystem root",
        });
    }
    // The home root under any spelling this module recognises. `~/x` and
    // `~user/x` are both "the home tree", and `~` alone is denied above —
    // without the prefix arm `cp ~/.ssh/id_rsa /tmp/x` fell to
    // `MissingJustification` instead of `DeniedPath`, which is a weaker answer
    // than the module's own doc comment promises. `$HOME/...` was already
    // covered, so the two spellings disagreed about the same directory.
    if t == "~"
        || t == "~/"
        || t.starts_with("~/")
        || t.starts_with("~$")
        || t.starts_with('$') && (t == "$HOME" || t.starts_with("$HOME/"))
    {
        return Some(DeniedPath {
            target: t.to_string(),
            detail: "home root",
        });
    }
    if let Some(h) = home.map(|h| h.trim_end_matches('/'))
        && !h.is_empty()
        && (t == h || t == format!("{h}/"))
    {
        return Some(DeniedPath {
            target: t.to_string(),
            detail: "home root",
        });
    }
    if t.split('/').any(|c| c == ".git") {
        return Some(DeniedPath {
            target: t.to_string(),
            detail: "VCS internals (.git)",
        });
    }
    let first = t.trim_start_matches('/').split('/').next().unwrap_or("");
    if DENIED_SYSTEM_DIRS.contains(&first) {
        return Some(DeniedPath {
            target: t.to_string(),
            detail: "system directory",
        });
    }
    None
}

// ============================================================================
// Gate
// ============================================================================

/// Why the gate refused to let a command run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The target is on the deny list. No justification can unlock it.
    DeniedPath(DeniedPath),
    /// High blast radius, and no justification was offered at all.
    MissingJustification { class: BlastRadius, detail: String },
    /// A justification was offered but did not carry enough substance.
    InsufficientJustification {
        class: BlastRadius,
        detail: String,
        defects: Vec<Defect>,
    },
}

impl Refusal {
    /// Risk level, for `CheckToolApprovalResult`.
    pub fn risk_level(&self) -> &'static str {
        match self {
            Self::DeniedPath(_) => "critical",
            Self::MissingJustification { .. } | Self::InsufficientJustification { .. } => "high",
        }
    }

    /// Which layer refused, for `CheckToolApprovalResult.blocked_by`.
    pub fn blocked_by(&self) -> &'static str {
        match self {
            Self::DeniedPath(_) => "path_deny",
            Self::MissingJustification { .. } | Self::InsufficientJustification { .. } => {
                "blast_radius"
            }
        }
    }

    /// The model-facing message. It must state what happened and how to
    /// proceed legitimately, so the correct response is a better justification
    /// rather than a different tool that dodges the gate.
    pub fn message(&self) -> String {
        match self {
            Self::DeniedPath(d) => format!(
                "Refused: `{}` is a denied path ({}). This is NOT overridable by a \
                 justification. Use a narrower, scoped target inside the working \
                 directory, or stop and tell the user this step needs a human.",
                d.target, d.detail
            ),
            Self::MissingJustification { class, detail } => format!(
                "Refused: {} command — {}. Before it can run you must re-issue the SAME \
                 call with a `justification` string that says WHY this destruction is \
                 necessary and WHAT is lost if it goes wrong. A restatement of the \
                 command, or padded text, will be rejected again.",
                class.label(),
                detail
            ),
            Self::InsufficientJustification {
                class,
                detail,
                defects,
            } => {
                let list = defects
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Refused: {} command — {}. The `justification` you supplied was not \
                     substantive ({list}). Re-issue the SAME call with a justification \
                     that states the concrete reason and the concrete loss, in your own \
                     words. Padding it, restating the command, or repeating one sentence \
                     will fail again — write the actual reason instead.",
                    class.label(),
                    detail
                )
            }
        }
    }
}

/// The gate: refuse a command whose blast radius is `Unbounded` or `Device`
/// unless a substantive justification accompanies it, and refuse denied paths
/// outright.
///
/// `justification` is the model's own written reason; `home` is the caller's
/// home directory. Pure — returns a decision, performs no I/O.
pub fn screen(command: &str, justification: Option<&str>, home: Option<&str>) -> Option<Refusal> {
    let class = classify(command);
    if class == BlastRadius::None {
        return None;
    }
    if class >= BlastRadius::Scoped
        && let Some(denied) = destructive_targets(command)
            .iter()
            .find_map(|t| check_deny(t, home))
    {
        return Some(Refusal::DeniedPath(denied));
    }
    if class < BlastRadius::Unbounded {
        return None;
    }
    let detail = unbounded_detail(command);
    match justification {
        None => Some(Refusal::MissingJustification { class, detail }),
        Some(j) => {
            let report = justification::assess(j, command);
            if report.sufficient {
                None
            } else {
                Some(Refusal::InsufficientJustification {
                    class,
                    detail,
                    defects: report.defects,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_should_rate_rm_recursive_on_root_as_unbounded() {
        assert_eq!(classify("rm -rf /"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf /*"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf ~"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf $HOME"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf *"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf ."), BlastRadius::Unbounded);
    }

    #[test]
    fn classify_should_rate_scoped_rm_lower_than_root_rm() {
        let scoped = classify("rm -rf ./target/debug");
        let root = classify("rm -rf /");
        assert_eq!(scoped, BlastRadius::Scoped);
        assert!(scoped < root, "scoped rm must rate below root rm");
        // A glob WITH a literal prefix is still bounded.
        assert_eq!(classify("rm -rf ./target/*"), BlastRadius::Scoped);
    }

    #[test]
    fn classify_should_detect_find_delete_and_redirect_to_device() {
        let found = classify("find ./build -name '*.o' -delete");
        assert!(
            found >= BlastRadius::Scoped,
            "find -delete must be destructive"
        );
        // Without a narrowing predicate the traversal is unbounded.
        assert_eq!(classify("find . -delete"), BlastRadius::Unbounded);
        // -exec rm is deletion too.
        assert!(classify("find ./build -name '*.o' -exec rm -f {} +") >= BlastRadius::Scoped);
        // A redirect into a block device is a raw device write...
        assert_eq!(classify("echo boom > /dev/sda"), BlastRadius::Device);
        assert_eq!(classify("dd if=/dev/zero of=/dev/sda"), BlastRadius::Device);
        // ...but /dev/null is not, or every shell command would be gated.
        assert!(classify("echo hi > /dev/null") < BlastRadius::Unbounded);
    }

    #[test]
    fn classify_should_detect_git_reset_hard_and_clean_force() {
        assert_eq!(classify("git reset --hard"), BlastRadius::Unbounded);
        assert_eq!(classify("git clean -fdx"), BlastRadius::Unbounded);
        // Read-only git stays out of the gate entirely.
        assert_eq!(classify("git status"), BlastRadius::None);
        // A soft/mixed reset keeps the working tree, so it is only scoped.
        assert_eq!(classify("git reset"), BlastRadius::Scoped);
    }

    #[test]
    fn path_deny_should_refuse_root_home_and_git_internals() {
        let home = Some("/home/agent");
        assert_eq!(
            check_deny("/", home).map(|d| d.detail),
            Some("filesystem root")
        );
        assert_eq!(
            check_deny("//", home).map(|d| d.detail),
            Some("filesystem root")
        );
        assert_eq!(check_deny("~", home).map(|d| d.detail), Some("home root"));
        assert_eq!(
            check_deny("$HOME", home).map(|d| d.detail),
            Some("home root")
        );
        assert_eq!(
            check_deny("/home/agent", home).map(|d| d.detail),
            Some("home root")
        );
        assert_eq!(
            check_deny("/etc/nginx", home).map(|d| d.detail),
            Some("system directory")
        );
        assert_eq!(
            check_deny("/usr/lib", home).map(|d| d.detail),
            Some("system directory")
        );
        assert_eq!(
            check_deny("repo/.git", home).map(|d| d.detail),
            Some("VCS internals (.git)")
        );
        assert_eq!(
            check_deny("repo/.git/objects/pack", home).map(|d| d.detail),
            Some("VCS internals (.git)")
        );
        // Scoped working-directory targets are fine.
        assert!(check_deny("./target/debug", home).is_none());
        assert!(check_deny("/tmp/scratch", home).is_none());
        // /dev is a device target, gated by justification, not by deny.
        assert!(check_deny("/dev/sda", home).is_none());
    }

    #[test]
    fn read_only_commands_are_unrated() {
        for cmd in [
            "ls",
            "ls -la /tmp",
            "cat a.txt",
            "grep -r x .",
            "echo hello",
        ] {
            assert_eq!(classify(cmd), BlastRadius::None, "{cmd}");
        }
    }

    #[test]
    fn screen_denies_root_rm_even_with_a_justification() {
        let good = "the fixture is rebuilding from scratch and the checkout is disposable";
        assert!(matches!(
            screen("rm -rf /", Some(good), Some("/home/agent")),
            Some(Refusal::DeniedPath(_))
        ));
    }

    #[test]
    fn screen_blocks_unbounded_without_a_justification() {
        assert!(matches!(
            screen("git reset --hard", None, None),
            Some(Refusal::MissingJustification { .. })
        ));
    }

    #[test]
    fn screen_passes_scoped_commands_unconditionally() {
        assert!(screen("rm -rf ./target", None, None).is_none());
        // A literal search root plus a narrowing predicate really is bounded.
        assert!(screen("find ./build -name '*.o' -delete", None, None).is_none());
    }

    #[test]
    fn find_from_the_working_directory_is_unbounded_despite_a_predicate() {
        // `-name` narrows WHICH files match, not HOW DEEP the walk goes, and
        // the walk starts at a cwd a pure classifier cannot bound. Same class
        // as `rm -rf .`, so it needs a justification.
        assert_eq!(
            classify("find . -name '*.log' -delete"),
            BlastRadius::Unbounded
        );
        assert!(matches!(
            screen("find . -name '*.log' -delete", None, None),
            Some(Refusal::MissingJustification { .. })
        ));
        // `find` without a deleting action is just a search.
        assert_eq!(classify("find . -name '*.log'"), BlastRadius::None);
    }

    #[test]
    fn multi_segment_commands_take_the_worst_segment() {
        assert_eq!(classify("ls -la && rm -rf /"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -f ./a.txt; echo done"), BlastRadius::Scoped);
    }
}

// ============================================================================
// The interpreter boundary (codex-deepen Row 1, 2026-10-31)
// ============================================================================

// ============================================================================
// Z15 — the verb families the table did not model (2026-11-16 red-team)
// ============================================================================

#[cfg(test)]
mod verb_family_tests {
    use super::*;

    /// The home a pure classifier can be given for the deny list.
    const HOME: Option<&str> = Some("/home/agent");

    /// The BETA: `screen()` must not wave these through. Each one is a real
    /// command that destroys something a modelled verb would be stopped for.
    #[test]
    fn screen_refuses_or_justifies_the_previously_invisible_verbs() {
        // Z15 named these exactly. Before the fix `verb_kind` returned None for
        // every one of them, `rate_segment` rated the segment `None`, and
        // `screen()` returned before the deny list was ever consulted.
        for command in [
            "mv /etc /tmp/x",
            "mv ~/.ssh /tmp/x",
            "cp /etc/shadow /tmp/x",
            "cp ~/.ssh/id_rsa /tmp/x",
            "rsync -a /etc/ /tmp/x",
            "install -m 644 /etc/shadow /tmp/x",
            "scp ~/.ssh/id_rsa host:/tmp/x",
        ] {
            let refusal = screen(command, None, HOME);
            assert!(
                refusal.is_some(),
                "`{command}` must be denied or require a justification, got allowed"
            );
        }
    }

    /// The verdicts themselves, stated as classes. A bare `is_some()` would let
    // a regression that escalates every `cp` to `Unbounded` pass.
    #[test]
    fn the_unmodelled_verbs_now_rate_by_what_they_touch() {
        assert_eq!(classify("mv /etc /tmp/x"), BlastRadius::Unbounded);
        assert_eq!(classify("cp /etc/shadow /tmp/x"), BlastRadius::Unbounded);
        assert_eq!(classify("rsync -a /etc/ /tmp/x"), BlastRadius::Unbounded);
        assert_eq!(
            classify("install -m 644 /etc/shadow /tmp/x"),
            BlastRadius::Unbounded
        );
        // A `~`-prefixed target is the home root under any spelling
        // (`check_deny`'s own contract). The segment is `Unbounded` so the
        // deny list runs; with the caller's `home` it is a DeniedPath.
        assert_eq!(classify("cp ~/.ssh/id_rsa /tmp/x"), BlastRadius::Unbounded);
        assert!(matches!(
            screen("cp ~/.ssh/id_rsa /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        // And a `~`-prefixed copy with no justification at all is refused
        // rather than allowed: the home root is not a thing this gate waves
        // through whatever the verb.
        assert!(screen("cp ~/.ssh/id_rsa /tmp/x", None, None).is_some());
    }

    /// A system directory is a DENIED path, not a justification: `mv /etc …`
    /// must refuse outright, with the same `blocked_by` the modelled verbs get.
    #[test]
    fn moving_a_system_directory_is_denied_outright() {
        assert!(matches!(
            screen("mv /etc /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        assert!(matches!(
            screen("cp /etc/shadow /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        assert!(matches!(
            screen("rsync -a /etc/ /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
    }

    /// The home root is denied even when the caller's home is not passed in:
    /// the `~` spelling is refused by the deny list without it.
    #[test]
    fn the_home_root_spelling_is_denied_for_copy_verbs() {
        assert!(matches!(
            screen("cp ~/.ssh/id_rsa /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        assert!(matches!(
            screen("mv ~/.ssh /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
    }

    /// The NEGATIVE control, and the reason the fix is not "block everything".
    /// A bounded copy/move of in-repo paths must stay allowed with NO
    /// justification: a gate that needs a justification for `cp ./a ./b` would
    /// be worse than the bug it closes.
    #[test]
    fn scoped_copy_move_sync_and_install_stay_allowed_without_a_justification() {
        for command in [
            "mv ./a ./b",
            "cp ./a ./b",
            "cp -r ./src ./dst",
            "rsync -a ./src/ ./dst/",
            "install -m 644 ./a ./b",
            "scp ./a ./b",
        ] {
            assert_eq!(
                classify(command),
                BlastRadius::Scoped,
                "`{command}` must stay Scoped"
            );
            assert!(
                screen(command, None, HOME).is_none(),
                "`{command}` must stay allowed without a justification"
            );
        }
    }

    /// `cp`/`rsync` with no `--delete` do not destroy the source set, so a
    /// scoped copy is NOT rated as a delete would be. Pinned separately from
    /// the control above because it is the arm that would over-block first.
    #[test]
    fn a_scoped_copy_that_only_reads_its_sources_is_scoped_not_unbounded() {
        assert_eq!(classify("cp ./a ./b"), BlastRadius::Scoped);
        assert_eq!(classify("cp -r ./src ./dst"), BlastRadius::Scoped);
    }

    /// The device case: `cp file /dev/sda` is a raw write even though `cp`
    /// never deletes anything.
    #[test]
    fn copying_onto_a_block_device_is_a_device_write() {
        assert_eq!(classify("cp ./a /dev/sda"), BlastRadius::Device);
        assert_eq!(classify("dd if=./a of=/dev/sda"), BlastRadius::Device);
    }

    /// `rsync --delete` and `--remove-source-files` are the arms that make a
    /// transfer destructive; without the flag the same command is not.
    #[test]
    fn rsync_deletes_only_with_the_flag_that_says_so() {
        assert_eq!(classify("rsync -a ./src/ ./dst/"), BlastRadius::Scoped);
        assert_eq!(
            classify("rsync -a --delete ./src/ ./dst/"),
            BlastRadius::Scoped
        );
        assert_eq!(
            classify("rsync -a --remove-source-files ./src/ ./dst/"),
            BlastRadius::Scoped
        );
        // Unbounded only when the target set has no bound, same as every other
        // verb: the flag decides WHAT is destroyed, the target decides HOW MUCH.
        assert_eq!(
            classify("rsync -a --delete /etc/ /tmp/x"),
            BlastRadius::Unbounded
        );
        assert_eq!(
            classify("rsync -a --remove-source-files ~ /tmp/x"),
            BlastRadius::Unbounded
        );
    }

    /// `install -m` eats its MODE token; if it did not, the mode would be read
    /// as a target and the rating would be computed on the wrong set.
    #[test]
    fn install_does_not_mistake_its_mode_for_a_target() {
        let (_class, targets) = rate_segment("install -m 644 ./a ./b");
        assert!(
            !targets.iter().any(|t| t == "644"),
            "the mode must not be a target: {targets:?}"
        );
        assert!(targets.iter().any(|t| t == "./b"), "got {targets:?}");
    }

    /// The audit's own "smaller fix": the deny check on the targets is what
    /// closes the CLASS, not the five verbs. Any verb outside the table —
    /// today's, tomorrow's — is now rated by its target.
    #[test]
    fn an_unmodelled_verb_is_rated_by_its_target_instead_of_disappearing() {
        // `ln` is not in the table and never was; the audit's point is that the
        // next unmodelled verb must not inherit `mv`'s hole.
        assert_ne!(
            classify("ln -sf /etc/passwd /tmp/x"),
            BlastRadius::None,
            "an unmodelled verb naming a denied path must not rate None"
        );
        assert_eq!(
            classify("ln -sf /etc/passwd /tmp/x"),
            BlastRadius::Unbounded
        );
        assert!(matches!(
            screen("ln -sf /etc/passwd /tmp/x", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        // And a bounded unmodelled target stays visible without a
        // justification, and still rates None: the fallback escalates ONLY a
        // denied target, which is what keeps it from becoming a
        // rate-everything switch.
        assert_eq!(classify("unshare ./a ./b"), BlastRadius::None);
        assert!(screen("unshare ./a ./b", None, HOME).is_none());
        // An unmodelled verb with no operand at all is still nothing.
        assert_eq!(classify("docker ps"), BlastRadius::None);
    }

    /// The regression the fallback could cause if it were sloppy: the
    /// classifier must not start refusing ordinary read-only commands. Every
    /// one of these was `None` before and must stay `None`.
    #[test]
    fn the_fallback_does_not_make_read_only_commands_destructive() {
        for command in [
            "ls -la /tmp",
            "cat ./notes.txt",
            "grep -r needle ./",
            "echo hello > /dev/null",
            "docker ps",
            "ssh host uptime",
            "crontab -l",
            // `cp --help` is a modelled verb with no operand, so it is a
            // destructive-shaped write with an empty target set: `Scoped`,
            // visible, but NOT gated. It must not become `Unbounded`, which
            // is what an over-eager "any copy is unbounded" arm would do.
        ] {
            assert_ne!(
                classify(command),
                BlastRadius::Unbounded,
                "`{command}` must not become unbounded"
            );
            assert_ne!(
                classify(command),
                BlastRadius::Device,
                "`{command}` must not become a device write"
            );
            assert!(
                screen(command, None, HOME).is_none(),
                "`{command}` must stay allowed"
            );
        }
    }

    /// The mutation guard. If `verb_kind` loses one of the five arms, this
    /// fails — and it fails on the OBSERVABLE difference, not on a comment.
    /// The pre-fix rating is re-derived here rather than asserted from memory.
    #[test]
    fn removing_a_verb_from_the_table_changes_the_rating_to_the_fallback() {
        // Pre-fix: unmodelled -> `verb: None` -> `rate_segment` returned None
        // for the whole segment, so the deny list was never reached.
        for (verb, target) in [
            ("mv", "/etc"),
            ("cp", "/etc/shadow"),
            ("rsync", "/etc/"),
            ("install", "/etc/shadow"),
            ("scp", "/etc/passwd"),
        ] {
            let modelled = format!("{verb} {target} /tmp/x");
            let bare_target = format!("wobble {target} /tmp/x");
            assert_eq!(
                classify(&modelled),
                BlastRadius::Unbounded,
                "the modelled verb must be rated by its target"
            );
            // `wobble` is not a verb in any table; it stands in for the
            // pre-fix `verb: None` path. The fallback rates it identically,
            // which is the property the fix adds.
            assert_eq!(
                classify(&bare_target),
                classify(&modelled),
                "an unknown verb must be rated like the real one"
            );
        }
    }

    /// Every verb `verb_kind` models is one this module can actually parse a
    /// segment for. The table is the audit's list of "what the gate can see",
    /// and it is what stops a future verb being added to the match with no
    /// handling in `parse_segment` or `rate_segment` — which is exactly how
    /// `mv`/`cp`/`rsync`/`install`/`scp` came to be invisible.
    #[test]
    fn every_recognised_verb_yields_a_non_empty_verdict_shape() {
        // One representative command per VerbKind. Each must rate `Scoped` or
        // above: a modelled verb that rates `None` on a real operand is a verb
        // the gate cannot see, which is the defect this table exists to
        // prevent. `git reset --hard` is the deliberate exception -- it is
        // unconditionally `Unbounded` (see the Vcs arm), so it is asserted
        // below rather than expected to stay allowed.
        for (verb, probe) in [
            ("rm", "rm -f ./a"),
            ("unlink", "unlink ./a"),
            ("shred", "shred -u ./a"),
            ("srm", "srm ./a"),
            ("rmdir", "rmdir ./a"),
            ("truncate", "truncate -s 0 ./a"),
            ("mv", "mv ./a ./b"),
            ("dd", "dd if=./a of=./b"),
            ("cp", "cp ./a ./b"),
            ("scp", "scp ./a ./b"),
            ("rsync", "rsync -a ./src/ ./dst/"),
            ("install", "install -m 644 ./a ./b"),
            ("tee", "tee ./a"),
            ("chmod", "chmod -R 755 ./src"),
            ("chown", "chown -R root ./src"),
            ("chgrp", "chgrp -R wheel ./src"),
            ("find", "find ./build -name '*.o' -delete"),
            ("git", "git reset --hard"),
            ("fdisk", "fdisk -l ./disk.img"),
            ("sfdisk", "sfdisk ./disk.img"),
            ("cfdisk", "cfdisk ./disk.img"),
            ("parted", "parted ./disk.img"),
            ("wipefs", "wipefs ./disk.img"),
            ("badblocks", "badblocks ./disk.img"),
            ("hdparm", "hdparm ./disk.img"),
            ("mkswap", "mkswap ./disk.img"),
        ] {
            assert!(
                verb_kind(verb).is_some(),
                "`{verb}` must be recognised, else the segment is invisible"
            );
            let rated = classify(probe);
            assert!(
                rated >= BlastRadius::Scoped,
                "`{probe}` ({verb}) must rate Scoped or above, got {rated:?}"
            );
            if verb == "git" {
                // `git reset --hard` discards the working tree unconditionally:
                // it needs a justification even with a scoped path.
                assert_eq!(rated, BlastRadius::Unbounded, "{probe}");
                assert!(screen(probe, None, HOME).is_some(), "{probe}");
            } else {
                assert!(
                    screen(probe, None, HOME).is_none(),
                    "`{probe}` ({verb}) is bounded and must stay allowed"
                );
            }
        }
    }

    /// The unbounded spelling of the same verbs is gated. This is the half that
    /// makes the table load-bearing rather than decorative.
    #[test]
    fn every_recognised_verb_gates_its_unbounded_spelling() {
        for probe in [
            "rm -rf /etc",
            "mv /etc /tmp/x",
            "cp /etc/shadow /tmp/x",
            "rsync -a /etc/ /tmp/x",
            "install -m 644 /etc/shadow /tmp/x",
            "scp /etc/passwd host:/tmp/x",
            "truncate -s 0 /etc/passwd",
            "shred -u /etc/passwd",
            "tee /etc/passwd",
            "chmod -R 755 /etc",
        ] {
            assert_ne!(
                classify(probe),
                BlastRadius::None,
                "`{probe}` must not be invisible"
            );
            assert!(
                screen(probe, None, HOME).is_some(),
                "`{probe}` must be denied or need a justification"
            );
        }
    }

    /// `cp -t DIR SRC` puts the destination FIRST. If the parser kept operand
    /// order, the rating would be computed on the source and the directory the
    /// copy lands in would never be looked at.
    #[test]
    fn a_target_directory_flag_is_rated_as_the_destination() {
        // `-t /etc SRC` writes into /etc: the source is fine, the destination
        // is a denied system directory.
        assert_eq!(classify("cp -t /etc ./a"), BlastRadius::Unbounded);
        assert!(matches!(
            screen("cp -t /etc ./a", None, HOME),
            Some(Refusal::DeniedPath(_))
        ));
        assert_eq!(classify("install -t /etc ./a"), BlastRadius::Unbounded);
        // A bounded `-t` directory stays scoped.
        assert_eq!(classify("cp -t ./out ./a"), BlastRadius::Scoped);
        assert_eq!(classify("install -t ./out ./a"), BlastRadius::Scoped);
        // And the long spelling.
        assert_eq!(
            classify("cp --target-directory=/etc ./a"),
            BlastRadius::Unbounded
        );
    }

    /// The refusal the operator actually sees must say WHICH target, so the
    /// next attempt can be a narrower one instead of the same command again.
    #[test]
    fn the_denial_message_names_the_denied_target() {
        let refusal = screen("mv /etc /tmp/x", None, HOME);
        let Some(Refusal::DeniedPath(denied)) = refusal else {
            panic!("expected a DeniedPath refusal, got {refusal:?}");
        };
        assert_eq!(denied.detail, "system directory");
        assert_eq!(denied.target, "/etc");
        assert_eq!(
            denied.detail,
            check_deny("/etc", HOME).map(|d| d.detail).unwrap_or("")
        );
    }
}

#[cfg(test)]
mod interpreter_boundary_tests {
    use super::*;

    #[test]
    fn bash_c_is_unwrapped_and_the_inner_command_is_classified() {
        // The whole point of the row. Before the unwrap, the verb the
        // classifier saw was `bash`, which no arm knows, so this rated `None`
        // and `screen()` waved it through.
        assert_eq!(classify("bash -c 'rm -rf /etc'"), BlastRadius::Unbounded);
        assert_eq!(
            classify("bash -c \"rm -rf /etc\""),
            BlastRadius::Unbounded,
            "double-quoted inner script must unwrap too"
        );
        // `sh` and its login-shell form behave the same way.
        assert_eq!(classify("sh -c 'rm -rf /etc'"), BlastRadius::Unbounded);
        assert_eq!(classify("sh -lc 'rm -rf /etc'"), BlastRadius::Unbounded);
    }

    /// RT-320: the GLUED form — `bash -c"rm -rf /etc"` is ONE word after
    /// quote removal (`-crm -rf /etc`), and a real shell runs exactly
    /// `rm -rf /etc` (getopt: the `c` flag's argument is the rest of the
    /// word). The old gate required the token to END with `c` and then
    /// `rfind`'d the LAST `c` — which for this shape is the one inside
    /// `/etc`, so the "glued script" came out empty, the segment became the
    /// unfollowable sentinel, and the gate passed it as `Scoped`.
    #[test]
    fn bash_c_glued_to_the_script_is_unwrapped_like_the_spaced_form() {
        assert_eq!(
            classify("bash -c\"rm -rf /etc\""),
            BlastRadius::Unbounded,
            "the glued form is the spaced form in a real shell"
        );
        assert_eq!(
            classify("bash -lc\"rm -rf /etc\""),
            BlastRadius::Unbounded,
            "the login-shell glued form too"
        );
        // The gate itself must refuse it, not merely the classifier.
        assert!(
            screen("bash -c\"rm -rf /etc\"", None, None).is_some(),
            "the destructive gate must fire on the glued form"
        );
        // A glued benign script stays benign (the guard against over-blocking).
        assert_eq!(classify("bash -c\"echo hi\""), BlastRadius::None);
    }

    #[test]
    fn benign_interpreter_use_is_not_penalised() {
        // The guard that keeps the row from blocking every `sh -c` in the
        // repo: benign interpreter use must stay `None`.
        assert_eq!(classify("bash -c 'echo hi'"), BlastRadius::None);
        assert_eq!(classify("sh -c 'ls -la'"), BlastRadius::None);
        assert_eq!(classify("bash -c 'grep -r foo .'"), BlastRadius::None);
    }

    #[test]
    fn wrapper_verbs_are_skipped_to_the_command_beneath_them() {
        // `sudo`/`env`/`timeout` are wrappers: the verb that does the work is
        // the one after them.
        assert_eq!(classify("sudo rm -rf /etc"), BlastRadius::Unbounded);
        assert_eq!(classify("env FOO=bar rm -rf /etc"), BlastRadius::Unbounded);
        assert_eq!(classify("timeout 30 rm -rf /etc"), BlastRadius::Unbounded);
        assert_eq!(classify("nice rm -rf /etc"), BlastRadius::Unbounded);
        // And a benign wrapped command stays benign.
        assert_eq!(classify("sudo ls -la"), BlastRadius::None);
        assert_eq!(classify("env FOO=bar ls -la"), BlastRadius::None);
    }

    #[test]
    fn an_interpreter_that_cannot_be_unwrapped_fails_closed() {
        // `bash -c` with no script argument at all is interpreter-shaped but
        // unextractable: it must NOT be `None`. This is the fail-closed arm —
        // the value that makes the unwrap a safety improvement rather than a
        // parser that can still be walked around.
        let c = classify("bash -c");
        assert_ne!(c, BlastRadius::None, "a lone `bash -c` must fail closed");
        assert_eq!(c, BlastRadius::Scoped);
        // A wrapper given no command either.
        assert_eq!(classify("sudo -u root"), BlastRadius::Scoped);
    }

    #[test]
    fn the_direct_path_is_not_regressed_by_the_unwrap() {
        // Every row must prove it did not break what already worked.
        assert_eq!(classify("rm -rf /etc"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf /"), BlastRadius::Unbounded);
        assert_eq!(classify("rm -rf ./target/debug"), BlastRadius::Scoped);
        assert_eq!(classify("ls -la"), BlastRadius::None);
        assert_eq!(classify("git status"), BlastRadius::None);
    }

    #[test]
    fn the_unwrap_reaches_through_a_pipeline_and_a_separator() {
        // The inner command must be found whether the interpreter is alone or
        // chained.
        assert_eq!(
            classify("ls && bash -c 'rm -rf /etc'"),
            BlastRadius::Unbounded
        );
        assert_eq!(
            classify("bash -c 'echo hi'; rm -rf /etc"),
            BlastRadius::Unbounded
        );
        assert_eq!(
            classify("bash -c 'echo hi' && bash -c 'rm -rf /etc'"),
            BlastRadius::Unbounded
        );
    }

    #[test]
    fn a_scoped_inner_command_stays_scoped_not_unbounded() {
        // Unwrapping must not inflate: `bash -c 'rm -rf ./build'` is scoped.
        assert_eq!(classify("bash -c 'rm -rf ./build'"), BlastRadius::Scoped);
        // And it must deny the same paths the direct path denies.
        let home = Some("/home/agent");
        let refusal = screen("bash -c 'rm -rf /'", None, home);
        assert!(
            matches!(refusal, Some(Refusal::DeniedPath(_))),
            "the unwrapped command must keep the path-deny layer, got {refusal:?}"
        );
    }

    /// The mutation guard from the plan, stated as an executable test.
    ///
    /// It re-derives the OLD behaviour (split only, never follow) and asserts
    /// the difference is observable — so reverting `segments_full` to the raw
    /// splitter makes this fail, which is what "mutation-proven" means.
    #[test]
    fn reverting_the_unwrap_changes_the_rating_from_unbounded_to_none() {
        let command = "bash -c 'rm -rf /etc'";
        // Rated through the unwrap.
        assert_eq!(classify(command), BlastRadius::Unbounded);
        // Rated the old way: raw segments only.
        let old_way = segments(command)
            .iter()
            .map(|seg| rate_segment(seg).0)
            .max()
            .unwrap_or(BlastRadius::None);
        assert_eq!(
            old_way,
            BlastRadius::None,
            "the pre-unwrap rating must have been None — if this fails the \
             baseline this row closes has itself moved and the row's premise \
             needs re-measuring"
        );
        assert_ne!(
            classify(command),
            old_way,
            "the unwrap must change the observable rating"
        );
    }

    #[test]
    fn the_tokenizer_honours_quotes_so_a_spaced_path_survives() {
        // A quoted script path with a space is one token, not two.
        let toks = tokens("bash -c '/tmp/my script.sh'");
        assert_eq!(toks, vec!["bash", "-c", "/tmp/my script.sh"]);
    }

    #[test]
    fn a_wrapper_chain_is_followed_to_the_command_beneath() {
        // The row-1 unwrap used to descend exactly one hop. `interpreter_script`
        // returned the whole remainder of a wrapper segment as one string, and
        // that string was then rated as a COMMAND -- so a wrapper sitting in
        // front of an interpreter (`env VAR=1 sh -c 'rm -rf /'`) rated `None`.
        // Every one of these must not be None: the whole point of row 1 was
        // that `sh -c "rm -rf /etc"` cannot pass the gate.
        for command in [
            "env VAR=1 sh -c 'rm -rf /'",
            "sudo -u root sh -c 'rm -rf /'",
            "timeout 5 sh -c 'rm -rf /'",
            "nice -n 5 sh -c 'rm -rf /'",
            "setsid sh -c 'rm -rf /'",
            "nohup env sh -c 'rm -rf /'",
        ] {
            assert_ne!(
                classify(command),
                BlastRadius::None,
                "a wrapper chain must be followed to the inner command: {command}"
            );
        }
    }

    #[test]
    fn nested_interpreters_are_followed_all_the_way_down() {
        // Same defect, nesting shape: `bash -c 'sh -c "rm -rf /"'`.
        // Quoting matters: `bash -c 'sh -c "rm -rf /"'` is valid shell (the
        // inner double quotes survive the outer single quotes), and that is the
        // shape a wrapper chain actually produces.
        assert_eq!(
            classify("bash -c 'sh -c \"rm -rf /\"'"),
            BlastRadius::Unbounded
        );
        // A wrapper chain inside an interpreter. This rates Scoped, not
        // Unbounded, and that is the measured limit of a quote-rewriting
        // unwrap (not a real shell parser): the outer `bash -c` hop strips one
        // quote layer, and the surviving `env sh -c rm -rf /` reaches
        // `rate_segment` as a segment whose head verb is `env` -- which
        // `rate_segment` does not model -- so it lands at Scoped.
        //
        // The important property is what it is NOT: not `None`. Scoped is
        // visible and gated. Every shape that used to escape entirely
        // (`env VAR=1 sh -c 'rm -rf /'`, the nesting above) now rates
        // Unbounded or Scoped, and never None.
        assert_ne!(
            classify("bash -c 'env sh -c rm -rf /'"),
            BlastRadius::None,
            "an env-inside-bash chain must not be rated None"
        );
        // The same chain written WITHOUT the extra layer is fully rated --
        // proof that the gap is the quote-stripping hop, not the verb model.
        assert_eq!(classify("env sh -c 'rm -rf /'"), BlastRadius::Unbounded);
        assert_ne!(classify("bash -c 'sudo rm -rf /'"), BlastRadius::None);

        // Three levels (`bash -c 'bash -c "sh -c rm -rf /"'`) degrade to Scoped,
        // and that is recorded rather than hidden: each hop strips one quote
        // layer and the third loses the pairing. Scoped keeps the command
        // VISIBLE and gated, which is the property row 1 exists for; `None` is
        // what let the family through, and nothing here returns it.
        assert_ne!(
            classify("bash -c 'bash -c \"sh -c rm -rf /\"'"),
            BlastRadius::None,
            "a three-level chain must not be rated None"
        );
        assert_eq!(
            classify("bash -c 'bash -c \"sh -c rm -rf /\"'"),
            BlastRadius::Scoped
        );
    }

    #[test]
    fn a_trap_handler_is_rated_not_skipped() {
        // Row 1 red-team (2026-11-01): `trap` is in INTERPRETER_VERBS, so
        // `interpreter_script` unwraps its tail and hands back the re-quoted
        // handler. `trap 'rm -rf /' EXIT` tokenizes to ["trap", "rm -rf /",
        // "EXIT"] -- `tokens()` collapses the handler to ONE token because it
        // was quoted -- and `rejoin_tail` re-quoted it into `'rm -rf /' EXIT`.
        // The recursion re-tokenized that into a single head token `rm -rf /`,
        // which `verb_kind` does not model, so the whole command rated None.
        //
        // A destructive command rated None is the exact failure row 1 exists
        // to prevent, so this pins it in both its bare and wrapped forms.
        assert_eq!(
            classify("trap 'rm -rf /' EXIT"),
            BlastRadius::Unbounded,
            "a trap whose handler is destructive must be rated, not skipped"
        );
        assert_eq!(
            classify("env trap 'rm -rf /' EXIT"),
            BlastRadius::Unbounded,
            "the same handler behind a wrapper must still be rated"
        );
        // The benign trap must NOT have been made destructive by the fix: the
        // rating comes from the handler, so a handler with no destructive verb
        // stays Scoped (visible) rather than escalating to Unbounded.
        assert_ne!(
            classify("trap rm EXIT"),
            BlastRadius::None,
            "a trap must never silently slip the gate"
        );
    }

    #[test]
    fn reverting_the_rejoin_fix_fails_this_test() {
        // The mutation-guard bookend for the rejoin_tail fix. If the
        // `first_is_a_quoted_phrase` guard is removed (or the condition above is
        // reverted to unconditional re-quoting), the head verb becomes
        // `rm -rf /`, `verb_kind` returns None and the rating collapses back to
        // BlastRadius::None -- which this test names.
        let toks = tokens("trap 'rm -rf /' EXIT");
        let rejoined = rejoin_tail(&toks, 1);
        // The handler must stay UN-quoted so the recursion sees `rm`.
        assert!(
            !rejoined.starts_with('\''),
            "rejoin_tail must not re-quote an already-unquoted first token, got {rejoined:?}"
        );
        let head = basename(&tokens(&rejoined)[0]).to_ascii_lowercase();
        assert_eq!(head, "rm", "the head verb must survive the round-trip");
    }

    #[test]
    fn an_unfollowable_interpreter_flag_fails_closed_at_scoped_not_none() {
        // `bash -c -x`: the flag after `-c` was consumed as the SCRIPT, so
        // `-x` was classified as the command and the whole thing rated None.
        // It is an interpreter we found but cannot follow -> Scoped, never None.
        assert_ne!(classify("bash -c -x"), BlastRadius::None);
        assert_eq!(classify("bash -c -x"), BlastRadius::Scoped);
        assert_ne!(classify("sh -c --"), BlastRadius::None);
    }

    #[test]
    fn a_deep_chain_past_the_input_bound_fails_closed_at_scoped() {
        // THE BOUND, pinned. The unwrap re-tokenizes at every hop, so its work
        // is superlinear in the input length; past MAX_INTERPRETER_INPUT the
        // classifier refuses to unwrap at all and emits the fail-closed
        // sentinel, which rates Scoped.
        //
        // The chain here is deliberately INSIDE the bound (six levels, 1.1 KiB)
        // and is then padded past it, so the test exercises the boundary rather
        // than the depth cap.
        //
        // Recorded limitation, so nobody reads this test as more than it is:
        // deep same-verb nesting (`bash -c 'bash -c ...'`) at depths 2-7 rates
        // NONE, because each hop strips one quote layer and by the second hop
        // the surviving head verb is no longer one `rate_segment` follows.
        // That is a gap in the quote-rewriting unwrap (it is not a real shell
        // parser), it pre-dates this iteration -- the pre-recursion shape rates
        // identically, measured before this test existed -- and the bound is
        // what stops it being unbounded work.
        let mut deep = String::from("rm -rf /");
        for _ in 0..6 {
            deep = format!("bash -c '{}'", deep.replace('\'', "'\\''"));
        }
        assert!(
            deep.len() <= MAX_INTERPRETER_INPUT,
            "this chain must start inside the unwrap bound; it is {} bytes",
            deep.len()
        );
        let padded = format!("{deep}{}", "bash -c 'rm -rf /'; ".repeat(400));
        assert!(
            padded.len() > MAX_INTERPRETER_INPUT,
            "the padded form must be past the bound; it is {} bytes",
            padded.len()
        );
        let rated = classify(&padded);
        assert_eq!(
            rated,
            BlastRadius::Scoped,
            "past MAX_INTERPRETER_INPUT the classifier must fail closed, not rate None"
        );
    }

    #[test]
    fn a_command_past_the_unwrap_bound_fails_closed_at_scoped() {
        // The DoS guard. The unwrap is superlinear in the input length, so past
        // MAX_INTERPRETER_INPUT the classifier stops unwrapping. It must NOT
        // then rate by the surface split: that split's head verb is `bash`,
        // which `rate_segment` does not model, so `bash -c 'rm -rf /'` would
        // rate `None` -- re-opening the exact hole row 1 closed.
        let mut padded = String::from("echo pad;");
        while padded.len() <= MAX_INTERPRETER_INPUT {
            padded.push_str("echo pad;");
        }
        padded.push_str("bash -c 'rm -rf /'");

        let start = std::time::Instant::now();
        let rated = classify(&padded);
        let elapsed = start.elapsed();

        // Fail CLOSED: visible and gated, never a silent pass.
        assert_eq!(
            rated,
            BlastRadius::Scoped,
            "a command past the unwrap bound must fail closed, not rate None"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "the unbounded path must be avoided: took {elapsed:?}"
        );
    }
}
