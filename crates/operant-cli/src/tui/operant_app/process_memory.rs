// Vendored from jcode (crates/operant-base/src/process_memory.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only ProcessMemorySnapshot (:32) and snapshot_with_source (:149), which the
// markdown/mermaid memory hooks read. See operant_app/mod.rs for scope.
// [port-decision] leaf port (extended scope round): snapshot_with_source's whole
// reachable chain, verbatim from the same file — imports (:1,5,6,11), consts
// (:13), OsProcessMemoryInfo (:49), AllocatorInfo/AllocatorStats
// (:78,:90), AllocatorProfilingInfo/AllocatorTuningInfo (:100,:106),
// ProcessMemoryHistoryEntry/MEMORY_HISTORY/memory_history (:126,:133,:135),
// allocator_info (:199, exact cfg split kept), glibc_malloc_stats
// (:237,:289 — jemalloc cfg variant dropped; `jemalloc` feature exists only in
// jcode's own crate, so upstream's jemalloc arms are cfg-stripped here
// identically), record_snapshot (:673), read_linux_memory_info (:693),
// parse_proc_status_value_bytes/parse_proc_status_count (:895,:901).
use crate::tui::operant_app::logging;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_HISTORY_SAMPLES: usize = 512;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessMemorySnapshot {
    pub rss_bytes: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
    /// Number of OS threads (`Threads:` in `/proc/self/status`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_count: Option<u64>,
    /// Main thread stack size (`VmStk:` in `/proc/self/status`). Auxiliary
    /// thread stacks live in anonymous mappings and are not included here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<OsProcessMemoryInfo>,
    pub allocator: AllocatorInfo,
}

#[cfg(target_os = "linux")]
pub fn snapshot_with_source(source: impl Into<String>) -> ProcessMemorySnapshot {
    let source = source.into();
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        logging::warn(&format!(
            "process memory snapshot source={source} missing /proc/self/status; using defaults"
        ));
        let snapshot = ProcessMemorySnapshot::default();
        record_snapshot(source, snapshot.clone());
        return snapshot;
    };

    let snapshot = ProcessMemorySnapshot {
        rss_bytes: parse_proc_status_value_bytes(&status, "VmRSS:"),
        peak_rss_bytes: parse_proc_status_value_bytes(&status, "VmHWM:"),
        virtual_bytes: parse_proc_status_value_bytes(&status, "VmSize:"),
        thread_count: parse_proc_status_count(&status, "Threads:"),
        main_stack_bytes: parse_proc_status_value_bytes(&status, "VmStk:"),
        os: read_linux_memory_info(&status),
        allocator: allocator_info(),
    };
    logging::debug(&format!(
        "process memory snapshot source={source} rss={:?} peak_rss={:?} virtual={:?} allocator={}",
        snapshot.rss_bytes,
        snapshot.peak_rss_bytes,
        snapshot.virtual_bytes,
        snapshot.allocator.name
    ));
    record_snapshot(source, snapshot.clone());
    snapshot
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OsProcessMemoryInfo {
    pub pss_bytes: Option<u64>,
    /// Proportional set size of anonymous mappings (`Pss_Anon:` in
    /// smaps_rollup): heap + thread stacks + other private anon memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_anon_bytes: Option<u64>,
    /// Proportional set size of file-backed mappings (`Pss_File:`): mostly
    /// the executable text/rodata and shared libraries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_file_bytes: Option<u64>,
    /// Proportional set size of shmem mappings (`Pss_Shmem:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_shmem_bytes: Option<u64>,
    /// Bytes backed by transparent huge pages (`AnonHugePages:`); a subset of
    /// anon memory that amplifies allocator retention (one live allocation
    /// pins a whole 2MB page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anon_huge_pages_bytes: Option<u64>,
    pub rss_anon_bytes: Option<u64>,
    pub rss_file_bytes: Option<u64>,
    pub rss_shmem_bytes: Option<u64>,
    pub private_clean_bytes: Option<u64>,
    pub private_dirty_bytes: Option<u64>,
    pub shared_clean_bytes: Option<u64>,
    pub shared_dirty_bytes: Option<u64>,
    pub swap_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllocatorInfo {
    pub name: &'static str,
    pub stats_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<AllocatorStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tuning: Option<AllocatorTuningInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profiling: Option<AllocatorProfilingInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorStats {
    pub allocated_bytes: Option<u64>,
    pub active_bytes: Option<u64>,
    pub metadata_bytes: Option<u64>,
    pub resident_bytes: Option<u64>,
    pub mapped_bytes: Option<u64>,
    pub retained_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorProfilingInfo {
    pub available: bool,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorTuningInfo {
    pub available: bool,
    pub background_thread: Option<bool>,
    pub max_background_threads: Option<u64>,
    pub arena_count: Option<u64>,
    pub initialized_arenas: Option<u64>,
    pub dirty_decay_ms: Option<i64>,
    pub muzzy_decay_ms: Option<i64>,
    pub retain: Option<bool>,
    pub tcache_enabled: Option<bool>,
    pub tcache_max_bytes: Option<u64>,
}

impl Default for AllocatorInfo {
    fn default() -> Self {
        allocator_info()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessMemoryHistoryEntry {
    pub timestamp_ms: u128,
    pub source: String,
    pub snapshot: ProcessMemorySnapshot,
}

static MEMORY_HISTORY: OnceLock<Mutex<VecDeque<ProcessMemoryHistoryEntry>>> = OnceLock::new();

fn memory_history() -> &'static Mutex<VecDeque<ProcessMemoryHistoryEntry>> {
    MEMORY_HISTORY.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_HISTORY_SAMPLES)))
}

pub fn allocator_info() -> AllocatorInfo {
    #[cfg(any())] // [port-decision] upstream jemalloc feature not declared in operant
    {
        let stats = jemalloc_stats();
        let profiling = jemalloc_profiling_info();
        AllocatorInfo {
            name: "jemalloc",
            stats_available: stats.is_some(),
            stats,
            tuning: jemalloc_tuning_info(),
            profiling,
        }
    }

    // [port-decision] (jemalloc branch above cfg-any() gated; this branch is unconditional)
    {
        let stats = glibc_malloc_stats();
        AllocatorInfo {
            name: "system",
            stats_available: stats.is_some(),
            stats,
            tuning: None,
            profiling: None,
        }
    }
}

/// Read glibc malloc statistics via `mallinfo2` (glibc >= 2.33).
///
/// This does not attribute memory to app structures, but it splits process
/// heap into "live" (bytes the app currently holds) and "retained" (bytes
/// freed by the app but kept by the allocator), which is the distinction that
/// matters when diagnosing unattributed RSS.
///
/// `mallinfo2` is resolved with `dlsym` instead of linked directly: release
/// binaries are built against a glibc 2.17 (manylinux2014) baseline where the
/// symbol does not exist, so a direct call fails to link. At runtime on a
/// modern glibc the lookup succeeds and stats work as before; on an old glibc
/// this returns `None` and callers already treat stats as unavailable.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn glibc_malloc_stats() -> Option<AllocatorStats> {
    // Mirrors glibc's `struct mallinfo2` (all fields `size_t`).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Mallinfo2 {
        arena: libc::size_t,
        ordblks: libc::size_t,
        smblks: libc::size_t,
        hblks: libc::size_t,
        hblkhd: libc::size_t,
        usmblks: libc::size_t,
        fsmblks: libc::size_t,
        uordblks: libc::size_t,
        fordblks: libc::size_t,
        keepcost: libc::size_t,
    }
    type Mallinfo2Fn = unsafe extern "C" fn() -> Mallinfo2;

    static MALLINFO2: std::sync::OnceLock<Option<Mallinfo2Fn>> = std::sync::OnceLock::new();
    let mallinfo2 = (*MALLINFO2.get_or_init(|| {
        // Safety: dlsym with a NUL-terminated literal; the default namespace
        // (RTLD_DEFAULT) searches the already-loaded glibc.
        let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"mallinfo2".as_ptr()) };
        if sym.is_null() {
            None
        } else {
            // Safety: glibc's mallinfo2 has exactly this signature.
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, Mallinfo2Fn>(sym) })
        }
    }))?;

    // Totals are summed across all arenas by modern glibc.
    // uordblks: in-use arena bytes; fordblks: freed-but-retained arena bytes;
    // hblkhd: mmap-backed allocation bytes; arena: total sbrk/mmap arena size.
    let info = unsafe { mallinfo2() };
    let live = (info.uordblks as u64).saturating_add(info.hblkhd as u64);
    let mapped = (info.arena as u64).saturating_add(info.hblkhd as u64);
    Some(AllocatorStats {
        allocated_bytes: Some(live),
        active_bytes: Some(info.uordblks as u64),
        metadata_bytes: None,
        resident_bytes: None,
        mapped_bytes: Some(mapped),
        retained_bytes: Some(info.fordblks as u64),
    })
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn glibc_malloc_stats() -> Option<AllocatorStats> {
    None
}

fn record_snapshot(source: String, snapshot: ProcessMemorySnapshot) {
    let Ok(mut history) = memory_history().lock() else {
        logging::error("process memory history lock poisoned; dropping snapshot");
        return;
    };
    if history.len() >= MAX_HISTORY_SAMPLES {
        logging::debug("process memory history full; dropping oldest snapshot");
        history.pop_front();
    }
    history.push_back(ProcessMemoryHistoryEntry {
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        source,
        snapshot,
    });
}

#[cfg(target_os = "linux")]
fn read_linux_memory_info(status: &str) -> Option<OsProcessMemoryInfo> {
    let smaps = std::fs::read_to_string("/proc/self/smaps_rollup").ok();
    let info = OsProcessMemoryInfo {
        pss_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Pss:")),
        pss_anon_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Pss_Anon:")),
        pss_file_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Pss_File:")),
        pss_shmem_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Pss_Shmem:")),
        anon_huge_pages_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "AnonHugePages:")),
        rss_anon_bytes: parse_proc_status_value_bytes(status, "RssAnon:"),
        rss_file_bytes: parse_proc_status_value_bytes(status, "RssFile:"),
        rss_shmem_bytes: parse_proc_status_value_bytes(status, "RssShmem:"),
        private_clean_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Private_Clean:")),
        private_dirty_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Private_Dirty:")),
        shared_clean_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Shared_Clean:")),
        shared_dirty_bytes: smaps
            .as_deref()
            .and_then(|text| parse_proc_value_bytes(text, "Shared_Dirty:")),
        swap_bytes: parse_proc_status_value_bytes(status, "VmSwap:").or_else(|| {
            smaps
                .as_deref()
                .and_then(|text| parse_proc_value_bytes(text, "Swap:"))
        }),
    };

    if info.pss_bytes.is_none()
        && info.rss_anon_bytes.is_none()
        && info.rss_file_bytes.is_none()
        && info.rss_shmem_bytes.is_none()
        && info.private_clean_bytes.is_none()
        && info.private_dirty_bytes.is_none()
        && info.shared_clean_bytes.is_none()
        && info.shared_dirty_bytes.is_none()
        && info.swap_bytes.is_none()
    {
        return None;
    }

    Some(info)
}

#[cfg(target_os = "linux")]
fn parse_proc_status_value_bytes(status: &str, key: &str) -> Option<u64> {
    parse_proc_value_bytes(status, key)
}

/// Parse a unit-less `/proc` counter such as `Threads:\t10`.
#[cfg(target_os = "linux")]
fn parse_proc_status_count(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(key)?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(target_os = "linux")]
fn parse_proc_value_bytes(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with(key) {
            return None;
        }
        let value = trimmed.trim_start_matches(key).trim();
        let mut parts = value.split_whitespace();
        let number = parts.next()?.parse::<u64>().ok()?;
        let unit = parts.next().unwrap_or("kB");
        Some(match unit {
            "kB" | "KB" | "kb" => number.saturating_mul(1024),
            "mB" | "MB" | "mb" => number.saturating_mul(1024 * 1024),
            "gB" | "GB" | "gb" => number.saturating_mul(1024 * 1024 * 1024),
            _ => number,
        })
    })
}

// [port-source] operant-base/src/process_memory.rs:455-475 — ported verbatim.
pub fn estimate_json_bytes<T: Serialize>(value: &T) -> usize {
    #[derive(Default)]
    struct ByteCounter {
        bytes: usize,
    }

    impl std::io::Write for ByteCounter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buffer.len());
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value)
        .map(|()| counter.bytes)
        .unwrap_or(0)
}
