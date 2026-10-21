//! `helpers` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

/// Used by `#[serde(skip_serializing_if)]` on plain `bool` fields to omit
/// them from TOML output when they carry their struct-level default (`false`).
/// Keeps fresh provider entries clean — a default-constructed
/// `ModelProviderConfig` for one provider family shouldn't write flag fields
/// that only apply to a different family.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn default_multimodal_max_images() -> usize {
    4
}

pub(crate) fn default_multimodal_max_image_size_mb() -> usize {
    5
}

pub(crate) fn normalize_comma_values(values: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        for part in value.split(',') {
            let normalized = part.trim();
            if normalized.is_empty() {
                continue;
            }
            output.push(normalized.to_string());
        }
    }
    output.sort_unstable();
    output.dedup();
    output
}

pub(crate) fn default_runtime_trace_mode() -> String {
    "none".to_string()
}

pub(crate) fn default_runtime_trace_path() -> String {
    "state/runtime-trace.jsonl".to_string()
}

pub(crate) fn default_runtime_trace_max_entries() -> usize {
    200
}

pub(crate) fn default_provider_retries() -> u32 {
    2
}

pub(crate) fn default_provider_backoff_ms() -> u64 {
    500
}

pub(crate) fn default_channel_backoff_secs() -> u64 {
    2
}

pub(crate) fn default_channel_backoff_max_secs() -> u64 {
    60
}

pub(crate) fn default_scheduler_poll_secs() -> u64 {
    15
}

pub(crate) fn default_scheduler_retries() -> u32 {
    2
}
