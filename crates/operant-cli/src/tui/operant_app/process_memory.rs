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

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn glibc_malloc_stats() -> Option<AllocatorStats> {
    None
}
