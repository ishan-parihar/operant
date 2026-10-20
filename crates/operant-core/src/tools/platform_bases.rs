//! The API base each messaging platform is reached at.
//!
//! The gateway's ADAPTERS read their base from the config
//! (`[gateway] telegram_api_base` and friends), so a deployment that must
//! reach a platform through a proxy — or point at a compatible endpoint —
//! can. The agent's TOOLS did not: they hard-coded the public API URLs,
//! so a deployment that reached Telegram through the gateway could not
//! reach it through the agent's `send_message`.
//!
//! One owner, read from the same runtime config the adapters read, so the
//! two cannot disagree.
//!
//! Each base has an environment override, matching the precedent the SMS
//! adapter set (`TWILIO_API_BASE`): the environment wins, so a container
//! can point a tool at a proxy without rewriting the config file.
//!
//! The bases are trimmed of a trailing `/` because the callers append
//! their own path (`/bot{token}/sendMessage`), and a doubled slash is a
//! different URL to most servers.

use crate::config::runtime_config;

/// The base URL for the Telegram Bot API.
///
/// Overridden by `TELEGRAM_API_BASE`.
pub fn telegram_api_base() -> String {
    let base = std::env::var("TELEGRAM_API_BASE")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| runtime_config().gateway.telegram_api_base);
    base.trim_end_matches('/').to_string()
}

/// The base URL for the Discord API.
///
/// Overridden by `DISCORD_API_BASE`.
pub fn discord_api_base() -> String {
    let base = std::env::var("DISCORD_API_BASE")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| runtime_config().gateway.discord_api_base);
    base.trim_end_matches('/').to_string()
}

/// The base URL for the Slack API.
///
/// Overridden by `SLACK_API_BASE`.
pub fn slack_api_base() -> String {
    let base = std::env::var("SLACK_API_BASE")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| runtime_config().gateway.slack_api_base);
    base.trim_end_matches('/').to_string()
}

/// The base URL for the WhatsApp Graph API.
///
/// Overridden by `WHATSAPP_API_BASE`.
pub fn whatsapp_api_base() -> String {
    let base = std::env::var("WHATSAPP_API_BASE")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| runtime_config().gateway.whatsapp_api_base);
    base.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bases come from the CONFIGURED values, and a trailing slash is
    /// trimmed — the callers append their own path, and a doubled slash is
    /// a different URL to most servers.
    ///
    /// The agent's TOOLS used to hard-code the public API URLs, so a
    /// deployment that reached a platform through the gateway could not
    /// reach it through the agent's `send_message`. The adapters read the
    /// config; the tools did not, and the two could disagree about where
    /// a platform is.
    #[test]
    fn the_bases_come_from_the_config_and_carry_no_trailing_slash() {
        // The DEFAULTS are what the config carries when nothing is
        // overridden — the point is that the accessor reads the config
        // rather than a literal in this module.
        assert_eq!(telegram_api_base(), "https://api.telegram.org");
        assert_eq!(discord_api_base(), "https://discord.com/api/v10");
        assert_eq!(slack_api_base(), "https://slack.com/api");
        assert_eq!(whatsapp_api_base(), "https://graph.facebook.com");
    }

    /// The environment override wins over the config, and a trailing slash
    /// is trimmed so the caller's path is the only separator.
    ///
    /// The environment wins because a container must be able to point a
    /// tool at a proxy without rewriting the config file — the precedent
    /// the SMS adapter set with `TWILIO_API_BASE`.
    #[test]
    fn the_environment_overrides_the_config_and_the_slash_is_trimmed() {
        // The env is process-global, so the mutation is serialised against
        // the other env-mutating tests.
        let _lock = crate::platform::test_env_lock_blocking();
        let previous = std::env::var("TELEGRAM_API_BASE").ok();
        // SAFETY: the env is process-global; the lock above serialises
        // this test against every other test that mutates it.
        unsafe {
            std::env::set_var("TELEGRAM_API_BASE", "https://proxy.example/telegram/");
        }
        let base = telegram_api_base();
        assert_eq!(
            base, "https://proxy.example/telegram",
            "the override must win and the trailing slash must be trimmed"
        );
        let url = format!("{base}/bot{token}/sendMessage", token = "T");
        assert_eq!(url, "https://proxy.example/telegram/botT/sendMessage");
        // The scheme's own `//` is not a doubled separator — only a
        // doubled one AFTER the authority is.
        assert!(
            !url["https://".len()..].contains("//"),
            "the path must not contain a doubled slash: {url}"
        );
        // SAFETY: the lock above serialises this restore.
        unsafe {
            match previous {
                Some(v) => std::env::set_var("TELEGRAM_API_BASE", v),
                None => std::env::remove_var("TELEGRAM_API_BASE"),
            }
        }
    }
}
