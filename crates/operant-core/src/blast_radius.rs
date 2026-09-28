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
//! an interpreter (`sh -c`, `python -c`), a variable expanded at runtime, a
//! binary whose behavior is not in [`VerbKind`]. It raises the cost of a
//! careless `rm -rf /`; it does not make destructive commands impossible.
//! Real isolation lives in `operant-runtime`'s sandbox backends (landlock,
//! bubblewrap, firejail, docker, sandbox-exec — `SandboxBackend`).
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
    /// `dd` — raw block copy; `of=` names the destination.
    RawWrite,
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
        "dd" => VerbKind::RawWrite,
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
    if let Some(t) = toks.get(i)
        && let Some(v) = verb_kind(t)
    {
        s.verb = Some(v);
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
        if let Some(gt) = t.find('>') {
            let lhs = &t[..gt];
            let redirect_ish =
                lhs.is_empty() || lhs == "&" || lhs.chars().all(|c| c.is_ascii_digit());
            if redirect_ish {
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
            if s.verb == Some(VerbKind::Finder) {
                if FINDER_DELETE_FLAGS.contains(&name) {
                    s.deleting = true;
                }
                if FINDER_PREDICATE_FLAGS.contains(&name) {
                    s.narrowed = true;
                }
            }
            // A value-taking flag swallows the next token so it is not
            // mistaken for a target (`find . -name '*.log' -delete`).
            if VALUE_FLAGS.contains(&name) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        s.targets.push((*t).to_string());
        i += 1;
    }

    // `chmod -R 755 ./src` — the mode/owner is not a target.
    if s.verb == Some(VerbKind::RecursiveMeta) && !s.targets.is_empty() {
        s.targets.remove(0);
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
    if let Some(c) = sql_class(seg) {
        return (c, Vec::new());
    }
    let p = parse_segment(seg);
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

    let base = match p.verb {
        None => BlastRadius::None,
        Some(VerbKind::Remove) | Some(VerbKind::Write) => {
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
        Some(VerbKind::RecursiveMeta) => {
            let recursive = p
                .flags
                .iter()
                .any(|f| has_short_flag(f, 'r') || f == "--recursive");
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
    for seg in segments(command) {
        class = class.max(rate_segment(&seg).0);
    }
    class
}

/// Split on shell separators. Indirection through a script or an interpreter
/// is deliberately not modelled — see the module docs.
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

/// Every target a destructive verb or a redirect can reach, for path denial.
pub fn destructive_targets(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seg in segments(command) {
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
    for seg in segments(command) {
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
    if t == "~" || t == "~/" || t.starts_with('$') && (t == "$HOME" || t.starts_with("$HOME/")) {
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
