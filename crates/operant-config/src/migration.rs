//! Pre-deserialization table migration for nested field changes.
//!
//! The v0→v1 config migration lived here with the aggregate it migrated
//! (`schema::Config`). The aggregate is gone (RT-141: it was never loaded
//! in production), so only the table preparation survives — and only for
//! the `[channels]` shape, which is itself part of the retired surface.

/// Pre-deserialization table migration for nested field changes that
/// `#[serde(flatten)]` cannot capture (e.g. removing a field from a nested
/// struct and moving its value elsewhere).
///
/// Called on the raw `toml::Table` before it is deserialized into `V1Compat`.
pub fn prepare_table(table: &mut toml::Table) {
    // Migrate channels_config.matrix.room_id → channels_config.matrix.allowed_rooms
    for key in &["channels_config", "channels"] {
        if let Some(toml::Value::Table(channels)) = table.get_mut(*key)
            && let Some(toml::Value::Table(matrix)) = channels.get_mut("matrix")
            && let Some(toml::Value::String(room_id)) = matrix.remove("room_id")
            && !room_id.is_empty()
        {
            let rooms = matrix
                .entry("allowed_rooms")
                .or_insert_with(|| toml::Value::Array(Vec::new()));
            if let toml::Value::Array(arr) = rooms {
                let already_present = arr.iter().any(|v| v.as_str() == Some(room_id.as_str()));
                if !already_present {
                    arr.push(toml::Value::String(room_id));
                }
            }
        }
    }

    // Migrate channels.slack.channel_id → channels.slack.channel_ids
    for key in &["channels_config", "channels"] {
        if let Some(toml::Value::Table(channels)) = table.get_mut(*key)
            && let Some(toml::Value::Table(slack)) = channels.get_mut("slack")
            && let Some(toml::Value::String(channel_id)) = slack.remove("channel_id")
            && !channel_id.is_empty()
            && channel_id != "*"
        {
            let ids = slack
                .entry("channel_ids")
                .or_insert_with(|| toml::Value::Array(Vec::new()));
            if let toml::Value::Array(arr) = ids {
                let already_present = arr.iter().any(|v| v.as_str() == Some(channel_id.as_str()));
                if !already_present {
                    arr.push(toml::Value::String(channel_id));
                }
            }
        }
    }

    // Rename legacy `channels_config` key to `channels`
    if table.contains_key("channels_config")
        && !table.contains_key("channels")
        && let Some(val) = table.remove("channels_config")
    {
        table.insert("channels".to_string(), val);
    }
}
