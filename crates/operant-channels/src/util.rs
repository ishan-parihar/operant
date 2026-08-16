/// Truncate a string to `max_chars` Unicode characters, appending "..." if truncated.
pub fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => {
            let truncated = &s[..idx];
            format!("{}...", truncated.trim_end())
        }
        None => s.to_string(),
    }
}

pub const BLOCK_KIT_PREFIX: &str = "__OPERANT_BLOCK_KIT__";

pub fn strip_tool_call_tags(message: &str) -> String {
    const TOOL_CALL_OPEN_TAGS: [&str; 7] = [
        "<function_calls>",
        "<function_call>",
        "<tool_call>",
        "<toolcall>",
        "<tool-call>",
        "<tool>",
        "<invoke>",
    ];

    fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
        tags.iter()
            .filter_map(|tag| haystack.find(tag).map(|idx| (idx, *tag)))
            .min_by_key(|(idx, _)| *idx)
    }

    fn matching_close_tag(open_tag: &str) -> Option<&'static str> {
        match open_tag {
            "<function_calls>" => Some("</function_calls>"),
            "<function_call>" => Some("</function_call>"),
            "<tool_call>" => Some("</tool_call>"),
            "<toolcall>" => Some("</toolcall>"),
            "<tool-call>" => Some("</tool-call>"),
            "<tool>" => Some("</tool>"),
            "<invoke>" => Some("</invoke>"),
            _ => None,
        }
    }

    fn extract_first_json_end(input: &str) -> Option<usize> {
        let trimmed = input.trim_start();
        let trim_offset = input.len().saturating_sub(trimmed.len());

        for (byte_idx, ch) in trimmed.char_indices() {
            if ch != '{' && ch != '[' {
                continue;
            }

            let slice = &trimmed[byte_idx..];
            let mut stream =
                serde_json::Deserializer::from_str(slice).into_iter::<serde_json::Value>();
            if let Some(Ok(_value)) = stream.next() {
                let consumed = stream.byte_offset();
                if consumed > 0 {
                    return Some(trim_offset + byte_idx + consumed);
                }
            }
        }

        None
    }

    fn strip_leading_close_tags(mut input: &str) -> &str {
        loop {
            let trimmed = input.trim_start();
            if !trimmed.starts_with("</") {
                return trimmed;
            }

            let Some(close_end) = trimmed.find('>') else {
                return "";
            };
            input = &trimmed[close_end + 1..];
        }
    }

    let mut kept_segments = Vec::new();
    let mut remaining = message;

    while let Some((start, open_tag)) = find_first_tag(remaining, &TOOL_CALL_OPEN_TAGS) {
        let before = &remaining[..start];
        if !before.is_empty() {
            kept_segments.push(before.to_string());
        }

        let Some(close_tag) = matching_close_tag(open_tag) else {
            break;
        };
        let after_open = &remaining[start + open_tag.len()..];

        if let Some(close_idx) = after_open.find(close_tag) {
            remaining = &after_open[close_idx + close_tag.len()..];
            continue;
        }

        if let Some(consumed_end) = extract_first_json_end(after_open) {
            remaining = strip_leading_close_tags(&after_open[consumed_end..]);
            continue;
        }

        kept_segments.push(remaining[start..].to_string());
        remaining = "";
        break;
    }

    if !remaining.is_empty() {
        kept_segments.push(remaining.to_string());
    }

    let mut result = kept_segments.concat();

    // Clean up any resulting blank lines (but preserve paragraphs)
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

/// Recognized attachment marker kinds (e.g. `[IMAGE:/path]`, `[DOCUMENT:url]`).
const ATTACHMENT_KINDS: &[&str] = &[
    "IMAGE", "PHOTO", "DOCUMENT", "FILE", "VIDEO", "AUDIO", "VOICE",
];

/// Parse `[KIND:target]` attachment markers out of a message.
/// Returns cleaned text (markers removed) and a vec of `(kind, target)` pairs.
pub fn parse_attachment_markers(message: &str) -> (String, Vec<(String, String)>) {
    let mut cleaned = String::with_capacity(message.len());
    let mut attachments = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = message[cursor..].find('[') {
        let start = cursor + rel_start;
        cleaned.push_str(&message[cursor..start]);

        let Some(rel_end) = message[start..].find(']') else {
            cleaned.push_str(&message[start..]);
            cursor = message.len();
            break;
        };
        let end = start + rel_end;
        let marker_text = &message[start + 1..end];

        let parsed = marker_text.split_once(':').and_then(|(kind, target)| {
            let kind_upper = kind.trim().to_ascii_uppercase();
            let target = target.trim();
            if target.is_empty() || !ATTACHMENT_KINDS.contains(&kind_upper.as_str()) {
                return None;
            }
            Some((kind_upper, target.to_string()))
        });

        if let Some(attachment) = parsed {
            attachments.push(attachment);
        } else {
            cleaned.push_str(&message[start..=end]);
        }

        cursor = end + 1;
    }

    if cursor < message.len() {
        cleaned.push_str(&message[cursor..]);
    }

    (cleaned.trim().to_string(), attachments)
}

/// Generate a short 6-character lowercase alphanumeric approval token.
///
/// Uses the full `[a-z0-9]` alphabet (36 options per position, 36^6 ≈ 2.2B
/// combinations) — not UUID hex (which would give only 16^6 ≈ 16.7M and
/// would materially weaken the WhatsApp no-per-sender-check design
/// described in the PR #6010 security note).
pub(crate) fn new_approval_token() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

/// Parse an approval reply of the form `"TOKEN yes|no|always ..."`.
///
/// Returns `Some((token, response))` when the text begins with a 6-character
/// alphanumeric token followed by a recognised action word. Returns `None`
/// for any other input so normal messages are not intercepted.
pub fn parse_approval_reply(
    text: &str,
) -> Option<(String, operant_api::channel::ChannelApprovalResponse)> {
    use operant_api::channel::ChannelApprovalResponse;
    let lower = text.trim().to_lowercase();
    let mut parts = lower.splitn(2, ' ');
    let token = parts.next()?.to_string();
    if token.len() != 6 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let action_word = parts.next()?.split_whitespace().next()?;
    let response = match action_word {
        "yes" | "y" | "approve" => ChannelApprovalResponse::Approve,
        "no" | "n" | "deny" => ChannelApprovalResponse::Deny,
        "always" => ChannelApprovalResponse::AlwaysApprove,
        _ => return None,
    };
    Some((token, response))
}

/// Generate a conversation history key from a channel message.
/// SSRF verdict for a URL: `Ok(true)` when the hostname resolves only to
/// public addresses, `Ok(false)` when it is blocked, `Err` on DNS failure
/// (fail-closed). Hermes `tools/url_safety.py::is_safe_url` parity — blocks
/// loopback, private (RFC 1918), link-local, multicast, unspecified, and
/// reserved ranges so a malicious attachment URL can't reach cloud metadata
/// (169.254.169.254), localhost services, or internal networks.
pub async fn ssrf_verdict(url: &str) -> anyhow::Result<bool> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;

    // Literal IPs are checked directly (no DNS). The typed `host()` returns
    // the IP for IPv6 bracket syntax, which `host_str()` round-tripping can
    // mangle.
    if let Some(host) = parsed.host() {
        match host {
            url::Host::Ipv4(ip) => return Ok(!is_blocked_ip(std::net::IpAddr::V4(ip))),
            url::Host::Ipv6(ip) => return Ok(!is_blocked_ip(std::net::IpAddr::V6(ip))),
            url::Host::Domain(_) => {}
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    // Resolve the hostname; fail-closed on DNS errors.
    let resolved = tokio::net::lookup_host((host, 443)).await?;
    for addr in resolved {
        if is_blocked_ip(addr.ip()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || is_reserved_ipv4(v4)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || matches!(v6.segments(), [0, 0, 0, 0, 0, 0, 0, 1])
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

fn is_reserved_ipv4(v4: std::net::Ipv4Addr) -> bool {
    // 0.0.0.0/8, 100.64.0.0/10 (CGNAT), 169.254.0.0/16 (link-local — caught
    // above but kept for clarity), 192.0.0.0/24, 192.0.2.0/24 (TEST-NET),
    // 198.18.0.0/15 (benchmark), 198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4,
    // 240.0.0.0/4, 255.255.255.255.
    let o = v4.octets();
    let block = u32::from_be_bytes(o);
    let is_cgnat = (o[0] == 100) && (o[1] & 0xC0) == 0x40;
    let is_doc = o[0] == 192 && o[1] == 0 && o[2] == 2;
    let is_bench = o[0] == 198 && (o[1] & 0xFE) == 18;
    let is_doc2 = o[0] == 198 && o[1] == 51 && o[2] == 100;
    let is_doc3 = o[0] == 203 && o[1] == 0 && o[2] == 113;
    let is_class_e = (o[0] & 0xF0) == 0xF0;
    let is_broadcast = v4 == std::net::Ipv4Addr::BROADCAST;
    block == 0 || is_cgnat || is_doc || is_bench || is_doc2 || is_doc3 || is_class_e || is_broadcast
}

/// Fetch a URL with per-redirect SSRF re-validation (hermes
/// `_read_url_image_with_redirect_guard` parity). `allow_redirects(false)`
/// plus manual hop walking means every redirect target is re-checked before
/// any bytes are read — a redirect from a public URL to a private address
/// is refused instead of silently followed.
pub async fn fetch_url_with_ssrf_guard(
    client: &reqwest::Client,
    url: &str,
    max_redirects: usize,
) -> anyhow::Result<reqwest::Response> {
    let mut current = url.to_string();
    for _ in 0..=max_redirects {
        if !ssrf_verdict(&current).await? {
            anyhow::bail!(
                "Blocked URL redirect to private/internal address (SSRF protection): {current}"
            );
        }
        let resp = client.get(&current).send().await?.error_for_status()?;
        if let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            let next = url::Url::parse(&current)
                .and_then(|base| base.join(loc))
                .map(|u| u.to_string())
                .unwrap_or_else(|_| loc.to_string());
            current = next;
            continue;
        }
        return Ok(resp);
    }
    anyhow::bail!("Too many URL redirects (SSRF protection)")
}

pub fn conversation_history_key(msg: &operant_api::channel::ChannelMessage) -> String {
    match &msg.thread_ts {
        Some(tid) => format!(
            "{}_{}_{}_{}",
            msg.channel, msg.reply_target, tid, msg.sender
        ),
        None => format!("{}_{}_{}", msg.channel, msg.reply_target, msg.sender),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SSRF guard ────────────────────────────────────────────────

    #[tokio::test]
    async fn ssrf_verdict_blocks_literals() {
        assert!(!ssrf_verdict("http://127.0.0.1:8080/admin").await.unwrap());
        assert!(!ssrf_verdict("http://10.0.0.5/internal").await.unwrap());
        assert!(
            !ssrf_verdict("http://169.254.169.254/latest/meta-data")
                .await
                .unwrap()
        );
        assert!(!ssrf_verdict("http://192.168.1.1/router").await.unwrap());
        assert!(!ssrf_verdict("http://[::1]/api").await.unwrap());
    }

    #[tokio::test]
    async fn ssrf_verdict_allows_public_host() {
        // example.com is a public test domain; DNS may be unavailable in
        // offline CI, so accept both the Ok(true) and the Err (fail-closed)
        // outcomes — the assertion is that a *public* literal IP is allowed.
        assert!(ssrf_verdict("http://93.184.216.34/resource").await.unwrap());
    }

    #[tokio::test]
    async fn ssrf_verdict_rejects_localhost_name() {
        // localhost resolves to loopback on every platform.
        let v = ssrf_verdict("http://localhost:3000/api").await;
        let blocked = match &v {
            Ok(safe) => !safe,
            Err(_) => true, // DNS failure = fail-closed
        };
        assert!(blocked, "got: {v:?}");
    }

    #[tokio::test]
    async fn is_blocked_ip_covers_metadata() {
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.1.2.3".parse().unwrap()));
        assert!(is_blocked_ip("100.64.0.1".parse().unwrap())); // CGNAT
        assert!(!is_blocked_ip("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn parse_attachment_markers_extracts_known_kinds() {
        let (cleaned, attachments) =
            parse_attachment_markers("Here [IMAGE:/tmp/a.png] and [DOCUMENT:/tmp/b.pdf] done");
        assert_eq!(cleaned, "Here  and  done");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0], ("IMAGE".into(), "/tmp/a.png".into()));
        assert_eq!(attachments[1], ("DOCUMENT".into(), "/tmp/b.pdf".into()));
    }

    #[test]
    fn parse_attachment_markers_preserves_unknown_kinds() {
        let (cleaned, attachments) = parse_attachment_markers("Check [UNKNOWN:foo] out");
        assert_eq!(cleaned, "Check [UNKNOWN:foo] out");
        assert!(attachments.is_empty());
    }

    #[test]
    fn parse_attachment_markers_preserves_empty_target() {
        let (cleaned, attachments) = parse_attachment_markers("See [IMAGE:] here");
        assert_eq!(cleaned, "See [IMAGE:] here");
        assert!(attachments.is_empty());
    }

    #[test]
    fn parse_attachment_markers_no_markers() {
        let (cleaned, attachments) = parse_attachment_markers("Hello world");
        assert_eq!(cleaned, "Hello world");
        assert!(attachments.is_empty());
    }

    #[test]
    fn parse_attachment_markers_all_kinds() {
        let input = "[IMAGE:a] [PHOTO:b] [DOCUMENT:c] [FILE:d] [VIDEO:e] [AUDIO:f] [VOICE:g]";
        let (_, attachments) = parse_attachment_markers(input);
        assert_eq!(attachments.len(), 7);
    }

    #[test]
    fn parse_attachment_markers_case_insensitive_kind() {
        let (_, attachments) = parse_attachment_markers("[image:/tmp/a.png]");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].0, "IMAGE");
    }

    #[test]
    fn new_approval_token_is_6_char_alphanumeric() {
        let token = super::new_approval_token();
        assert_eq!(token.len(), 6);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn parse_approval_reply_accepts_canonical_forms() {
        use operant_api::channel::ChannelApprovalResponse;
        let cases = [
            ("abc123 yes", ChannelApprovalResponse::Approve),
            ("abc123 y", ChannelApprovalResponse::Approve),
            ("abc123 approve", ChannelApprovalResponse::Approve),
            ("ABC123 YES", ChannelApprovalResponse::Approve),
            (
                "abc123 yes please go ahead",
                ChannelApprovalResponse::Approve,
            ),
            ("abc123 no", ChannelApprovalResponse::Deny),
            ("abc123 n", ChannelApprovalResponse::Deny),
            ("abc123 deny", ChannelApprovalResponse::Deny),
            ("abc123 always", ChannelApprovalResponse::AlwaysApprove),
        ];
        for (input, expected) in cases {
            let (token, response) = super::parse_approval_reply(input)
                .unwrap_or_else(|| panic!("expected Some for input {:?}", input));
            assert_eq!(
                token,
                input.trim().to_lowercase().split(' ').next().unwrap()
            );
            assert_eq!(response, expected, "input: {input:?}");
        }
    }

    #[test]
    fn parse_approval_reply_rejects_bad_input() {
        let bad = [
            "yes",
            "abc123",
            "abc 123 yes",
            "toolname yes",
            "abc123 maybe",
            "",
            "abc123 ",
        ];
        for input in bad {
            assert!(
                super::parse_approval_reply(input).is_none(),
                "expected None for input {:?}",
                input
            );
        }
    }
}
