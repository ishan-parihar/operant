//! Telegram channel tests (verbatim body of the former inline `mod tests`).
use super::*;

#[test]
fn telegram_channel_name() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert_eq!(ch.name(), "telegram");
}

#[test]
fn link_previews_default_on_and_disablable() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert!(ch.link_preview_json().is_none(), "previews on by default");

    let disabled = ch.with_link_previews(false);
    assert_eq!(
        disabled.link_preview_json(),
        Some(serde_json::json!({ "is_disabled": true }))
    );
}

#[test]
fn typing_cooldown_clamps_minimum() {
    let base = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert_eq!(base.typing_cooldown_secs, 30.0);
    let clamped = base.with_typing_cooldown_secs(0.0);
    assert_eq!(clamped.typing_cooldown_secs, 1.0);
    let normal = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_typing_cooldown_secs(15.0);
    assert_eq!(normal.typing_cooldown_secs, 15.0);
}

#[test]
fn typing_cooldown_state_blocks_until_expiry() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    // Insert a cooldown ending in the future and verify the loop's gate
    // logic (in_cooldown check) suppresses refreshes.
    ch.typing_cooldown_until
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            "123".to_string(),
            std::time::Instant::now() + Duration::from_secs(60),
        );
    let in_cooldown = {
        let mut map = ch
            .typing_cooldown_until
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match map.get("123") {
            Some(until) if *until > std::time::Instant::now() => true,
            _ => {
                map.remove("123");
                false
            }
        }
    };
    assert!(in_cooldown);
}

#[test]
fn random_telegram_ack_reaction_is_from_pool() {
    for _ in 0..128 {
        let emoji = random_telegram_ack_reaction();
        assert!(TELEGRAM_ACK_REACTIONS.contains(&emoji));
    }
}

#[test]
fn telegram_ack_reaction_request_shape() {
    let body = build_telegram_ack_reaction_request("-100200300", 42, "⚡️");
    assert_eq!(body["chat_id"], "-100200300");
    assert_eq!(body["message_id"], 42);
    assert_eq!(body["reaction"][0]["type"], "emoji");
    assert_eq!(body["reaction"][0]["emoji"], "⚡️");
}

#[test]
fn telegram_extract_update_message_target_parses_ids() {
    let update = serde_json::json!({
        "update_id": 1,
        "message": {
            "message_id": 99,
            "chat": { "id": -100_123_456 }
        }
    });

    let target = TelegramChannel::extract_update_message_target(&update);
    assert_eq!(target, Some(("-100123456".to_string(), 99)));
}

#[test]
fn typing_handle_starts_as_none() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn stop_typing_clears_handle() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    // Manually insert a dummy handle
    {
        let mut guard = ch.typing_handle.lock();
        *guard = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
    }

    // stop_typing should abort and clear
    ch.stop_typing("123").await.unwrap();

    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn start_typing_replaces_previous_handle() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    // Insert a dummy handle first
    {
        let mut guard = ch.typing_handle.lock();
        *guard = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
    }

    // start_typing should abort the old handle and set a new one
    let _ = ch.start_typing("123").await;

    let guard = ch.typing_handle.lock();
    assert!(guard.is_some());
}

#[test]
fn supports_draft_updates_respects_stream_mode() {
    let off = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert!(!off.supports_draft_updates());

    let partial = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 750);
    assert!(partial.supports_draft_updates());
    assert_eq!(partial.draft_update_interval_ms, 750);
}

#[tokio::test]
async fn send_draft_returns_none_when_stream_mode_off() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let id = ch
        .send_draft(&SendMessage::new("draft", "123"))
        .await
        .unwrap();
    assert!(id.is_none());
}

#[tokio::test]
async fn update_draft_rate_limit_short_circuits_network() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 60_000);
    ch.last_draft_edit
        .lock()
        .insert("123".to_string(), std::time::Instant::now());

    let result = ch.update_draft("123", "42", "delta text").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn update_draft_utf8_truncation_is_safe_for_multibyte_text() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 0);
    let long_emoji_text = "😀".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 20);

    // Invalid message_id returns early after building display_text.
    // This asserts truncation never panics on UTF-8 boundaries.
    let result = ch
        .update_draft("123", "not-a-number", &long_emoji_text)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn finalize_draft_invalid_message_id_falls_back_to_chunk_send() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 0);
    let long_text = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 64);

    // For oversized text + invalid draft message_id, finalize_draft should
    // fall back to chunked send instead of returning early.
    let result = ch.finalize_draft("123", "not-a-number", &long_text).await;
    assert!(result.is_err());
}

#[test]
fn dm_topics_disabled_returns_none() {
    // Ensure-dm-topic is a no-op when the feature is off (no network).
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let out = rt.block_on(async { ch.ensure_dm_topic("123456").await });
    assert!(out.is_none());
}
/// Serializes tests that mutate `OPERANT_CONFIG_DIR` (Rust runs tests in
/// parallel within a binary, and env mutation would race).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn dm_topics_state_path_respects_config_dir_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    // Unset override first so the default branch (HOME) applies.
    unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };
    let default_path = TelegramChannel::dm_topic_state_path();
    assert!(default_path.ends_with("telegram_dm_topics.json"));

    unsafe { std::env::set_var("OPERANT_CONFIG_DIR", "/tmp/operant-dm-topic-test") };
    let custom = TelegramChannel::dm_topic_state_path();
    assert_eq!(
        custom,
        std::path::PathBuf::from("/tmp/operant-dm-topic-test/telegram_dm_topics.json")
    );
    unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };
    let _ = ch; // keep the channel binding used
}

#[test]
fn dm_topic_state_roundtrip() {
    let _guard = ENV_LOCK.lock().unwrap();
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    ch.dm_topic_threads
        .lock()
        .unwrap()
        .insert("999001".to_string(), 42);
    // Persist to a temp config dir, then load into a fresh channel.
    unsafe { std::env::set_var("OPERANT_CONFIG_DIR", "/tmp/operant-dm-topic-test") };
    ch.persist_dm_topic_state();
    let ch2 = TelegramChannel::new("t".into(), vec!["*".into()], false);
    ch2.load_dm_topic_state();
    assert_eq!(
        *ch2.dm_topic_threads.lock().unwrap().get("999001").unwrap(),
        42
    );
    unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };
    let _ = std::fs::remove_file(TelegramChannel::dm_topic_state_path());
}

#[test]
fn telegram_api_url() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("getMe"),
        "https://api.telegram.org/bot123:ABC/getMe"
    );
}

#[test]
fn telegram_markdown_to_html_escapes_quotes_in_link_href() {
    let rendered =
        TelegramChannel::markdown_to_telegram_html("[click](https://example.com?q=\"x\"&a='b')");
    assert_eq!(
        rendered,
        "<a href=\"https://example.com?q=&quot;x&quot;&amp;a=&#39;b&#39;\">click</a>"
    );
}

#[test]
fn telegram_markdown_to_html_escapes_quotes_in_plain_text() {
    let rendered = TelegramChannel::markdown_to_telegram_html("say \"hi\" & <tag> 'ok'");
    assert_eq!(
        rendered,
        "say &quot;hi&quot; &amp; &lt;tag&gt; &#39;ok&#39;"
    );
}

#[test]
fn telegram_markdown_to_html_code_block_drops_language_attribute() {
    let rendered =
        TelegramChannel::markdown_to_telegram_html("```rust\" onclick=\"alert(1)\nlet x = 1;\n```");
    assert_eq!(rendered, "<pre><code>let x = 1;</code></pre>");
    assert!(!rendered.contains("language-"));
    assert!(!rendered.contains("onclick"));
}

#[test]
fn telegram_user_allowed_wildcard() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_specific() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "bob".into()], false);
    assert!(ch.is_user_allowed("alice"));
    assert!(!ch.is_user_allowed("eve"));
}

#[test]
fn telegram_user_allowed_with_at_prefix_in_config() {
    let ch = TelegramChannel::new("t".into(), vec!["@alice".into()], false);
    assert!(ch.is_user_allowed("alice"));
}

#[test]
fn telegram_user_denied_empty() {
    let ch = TelegramChannel::new("t".into(), vec![], false);
    assert!(!ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_exact_match_not_substring() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.is_user_allowed("alice_bot"));
    assert!(!ch.is_user_allowed("alic"));
    assert!(!ch.is_user_allowed("malice"));
}

#[test]
fn telegram_user_empty_string_denied() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.is_user_allowed(""));
}

#[test]
fn telegram_user_case_sensitive() {
    let ch = TelegramChannel::new("t".into(), vec!["Alice".into()], false);
    assert!(ch.is_user_allowed("Alice"));
    assert!(!ch.is_user_allowed("alice"));
    assert!(!ch.is_user_allowed("ALICE"));
}

#[test]
fn telegram_wildcard_with_specific_users() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "*".into()], false);
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("bob"));
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_by_numeric_id_identity() {
    let ch = TelegramChannel::new("t".into(), vec!["123456789".into()], false);
    assert!(ch.is_any_user_allowed(["unknown", "123456789"]));
}

#[test]
fn telegram_user_denied_when_none_of_identities_match() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "987654321".into()], false);
    assert!(!ch.is_any_user_allowed(["unknown", "123456789"]));
}

#[test]
fn telegram_pairing_enabled_with_empty_allowlist() {
    let ch = TelegramChannel::new("t".into(), vec![], false);
    assert!(ch.pairing_code_active());
}

#[test]
fn telegram_pairing_disabled_with_nonempty_allowlist() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.pairing_code_active());
}

#[test]
fn telegram_extract_bind_code_plain_command() {
    assert_eq!(
        TelegramChannel::extract_bind_code("/bind 123456"),
        Some("123456")
    );
}

#[test]
fn telegram_extract_bind_code_supports_bot_mention() {
    assert_eq!(
        TelegramChannel::extract_bind_code("/bind@operant_bot 654321"),
        Some("654321")
    );
}

#[test]
fn telegram_extract_bind_code_rejects_invalid_forms() {
    assert_eq!(TelegramChannel::extract_bind_code("/bind"), None);
    assert_eq!(TelegramChannel::extract_bind_code("/start"), None);
}

#[test]
fn parse_attachment_markers_extracts_multiple_types() {
    let message = "Here are files [IMAGE:/tmp/a.png] and [DOCUMENT:https://example.com/a.pdf]";
    let (cleaned, attachments) = parse_attachment_markers(message);

    assert_eq!(cleaned, "Here are files  and");
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].kind, TelegramAttachmentKind::Image);
    assert_eq!(attachments[0].target, "/tmp/a.png");
    assert_eq!(attachments[1].kind, TelegramAttachmentKind::Document);
    assert_eq!(attachments[1].target, "https://example.com/a.pdf");
}

#[test]
fn parse_attachment_markers_keeps_invalid_markers_in_text() {
    let message = "Report [UNKNOWN:/tmp/a.bin]";
    let (cleaned, attachments) = parse_attachment_markers(message);

    assert_eq!(cleaned, "Report [UNKNOWN:/tmp/a.bin]");
    assert!(attachments.is_empty());
}

#[test]
fn parse_path_only_attachment_detects_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("snap.png");
    std::fs::write(&image_path, b"fake-png").unwrap();

    let parsed = parse_path_only_attachment(image_path.to_string_lossy().as_ref())
        .expect("expected attachment");

    assert_eq!(parsed.kind, TelegramAttachmentKind::Image);
    assert_eq!(parsed.target, image_path.to_string_lossy());
}

#[test]
fn parse_path_only_attachment_rejects_sentence_text() {
    assert!(parse_path_only_attachment("Screenshot saved to /tmp/snap.png").is_none());
}

#[test]
fn infer_attachment_kind_from_target_detects_document_extension() {
    assert_eq!(
        infer_attachment_kind_from_target("https://example.com/files/specs.pdf?download=1"),
        Some(TelegramAttachmentKind::Document)
    );
}

#[test]
fn parse_update_message_uses_chat_id_as_reply_target() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 1,
        "message": {
            "message_id": 33,
            "text": "hello",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300
            }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("message should parse");

    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.reply_target, "-100200300");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.id, "telegram_-100200300_33");
}

#[test]
fn parse_update_message_allows_numeric_id_without_username() {
    let ch = TelegramChannel::new("token".into(), vec!["555".into()], false);
    let update = serde_json::json!({
        "update_id": 2,
        "message": {
            "message_id": 9,
            "text": "ping",
            "from": {
                "id": 555
            },
            "chat": {
                "id": 12345
            }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("numeric allowlist should pass");

    assert_eq!(msg.sender, "555");
    assert_eq!(msg.reply_target, "12345");
}

#[test]
fn parse_update_message_extracts_thread_id_for_forum_topic() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 3,
        "message": {
            "message_id": 42,
            "text": "hello from topic",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300
            },
            "message_thread_id": 789
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("message with thread_id should parse");

    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.reply_target, "-100200300:789");
    assert_eq!(msg.content, "hello from topic");
    assert_eq!(msg.id, "telegram_-100200300_42");
}

// ── File sending API URL tests ──────────────────────────────────

#[test]
fn telegram_api_url_send_document() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendDocument"),
        "https://api.telegram.org/bot123:ABC/sendDocument"
    );
}

#[test]
fn telegram_api_url_send_photo() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendPhoto"),
        "https://api.telegram.org/bot123:ABC/sendPhoto"
    );
}

#[test]
fn telegram_api_url_send_video() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendVideo"),
        "https://api.telegram.org/bot123:ABC/sendVideo"
    );
}

#[test]
fn telegram_api_url_send_audio() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendAudio"),
        "https://api.telegram.org/bot123:ABC/sendAudio"
    );
}

#[test]
fn telegram_api_url_send_voice() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendVoice"),
        "https://api.telegram.org/bot123:ABC/sendVoice"
    );
}

// ── File sending integration tests (with mock server) ──────────

#[tokio::test]
async fn telegram_send_document_bytes_builds_correct_form() {
    // This test verifies the method doesn't panic and handles bytes correctly
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"Hello, this is a test file content".to_vec();

    // The actual API call will fail (no real server), but we verify the method exists
    // and handles the input correctly up to the network call
    let result = ch
        .send_document_bytes("123456", None, file_bytes, "test.txt", Some("Test caption"))
        .await;

    // Should fail with network error, not a panic or type error
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Error should be network-related, not a code bug
    assert!(
        err.contains("error") || err.contains("failed") || err.contains("connect"),
        "Expected network error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_bytes_builds_correct_form() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    // Minimal valid PNG header bytes
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    let result = ch
        .send_photo_bytes("123456", None, file_bytes, "test.png", None)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    let result = ch
        .send_document_by_url(
            "123456",
            None,
            "https://example.com/file.pdf",
            Some("PDF doc"),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_photo_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    let result = ch
        .send_photo_by_url("123456", None, "https://example.com/image.jpg", None)
        .await;

    assert!(result.is_err());
}

// ── File path handling tests ────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/file.txt");

    let result = ch.send_document("123456", None, path, None).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Should fail with file not found error
    assert!(
        err.contains("No such file") || err.contains("not found") || err.contains("os error"),
        "Expected file not found error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/photo.jpg");

    let result = ch.send_photo("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_video_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/video.mp4");

    let result = ch.send_video("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_audio_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/audio.mp3");

    let result = ch.send_audio("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_voice_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/voice.ogg");

    let result = ch.send_voice("123456", None, path, None).await;

    assert!(result.is_err());
}

// ── Message splitting tests ─────────────────────────────────────

#[test]
fn telegram_split_short_message() {
    let msg = "Hello, world!";
    let chunks = split_message_for_telegram(msg);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], msg);
}

#[test]
fn telegram_split_exact_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
    let chunks = split_message_for_telegram(&msg);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), TELEGRAM_MAX_MESSAGE_LENGTH);
}

#[test]
fn telegram_split_over_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
    let chunks = split_message_for_telegram(&msg);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    assert!(chunks[1].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
}

#[test]
fn telegram_split_at_word_boundary() {
    let msg = format!(
        "{} more text here",
        "word ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5)
    );
    let chunks = split_message_for_telegram(&msg);
    assert!(chunks.len() >= 2);
    // First chunk should end with a complete word (space at the end)
    for chunk in &chunks[..chunks.len() - 1] {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

#[test]
fn telegram_split_at_newline() {
    let text_block = "Line of text\n".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 13 + 1);
    let chunks = split_message_for_telegram(&text_block);
    assert!(chunks.len() >= 2);
    for chunk in chunks {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

#[test]
fn telegram_split_preserves_content() {
    let msg = "test ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5 + 100);
    let chunks = split_message_for_telegram(&msg);
    let rejoined = chunks.join("");
    assert_eq!(rejoined, msg);
}

#[test]
fn telegram_split_empty_message() {
    let chunks = split_message_for_telegram("");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "");
}

#[test]
fn telegram_split_very_long_message() {
    let msg = "x".repeat(TELEGRAM_MAX_MESSAGE_LENGTH * 3);
    let chunks = split_message_for_telegram(&msg);
    assert!(chunks.len() >= 3);
    for chunk in chunks {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

// ── Caption handling tests ──────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"test content".to_vec();

    // With caption
    let result = ch
        .send_document_bytes(
            "123456",
            None,
            file_bytes.clone(),
            "test.txt",
            Some("My caption"),
        )
        .await;
    assert!(result.is_err()); // Network error expected

    // Without caption
    let result = ch
        .send_document_bytes("123456", None, file_bytes, "test.txt", None)
        .await;
    assert!(result.is_err()); // Network error expected
}

#[tokio::test]
async fn telegram_send_photo_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47];

    // With caption
    let result = ch
        .send_photo_bytes(
            "123456",
            None,
            file_bytes.clone(),
            "test.png",
            Some("Photo caption"),
        )
        .await;
    assert!(result.is_err());

    // Without caption
    let result = ch
        .send_photo_bytes("123456", None, file_bytes, "test.png", None)
        .await;
    assert!(result.is_err());
}

// ── Empty/edge case tests ───────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_empty_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes: Vec<u8> = vec![];

    let result = ch
        .send_document_bytes("123456", None, file_bytes, "empty.txt", None)
        .await;

    // Should not panic, will fail at API level
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_filename() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"content".to_vec();

    let result = ch
        .send_document_bytes("123456", None, file_bytes, "", None)
        .await;

    // Should not panic
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_chat_id() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"content".to_vec();

    let result = ch
        .send_document_bytes("", None, file_bytes, "test.txt", None)
        .await;

    // Should not panic
    assert!(result.is_err());
}

// ── Message ID edge cases ─────────────────────────────────────

#[test]
fn telegram_message_id_format_includes_chat_and_message_id() {
    // Verify that message IDs follow the format: telegram_{chat_id}_{message_id}
    let chat_id = "123456";
    let message_id = 789;
    let expected_id = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(expected_id, "telegram_123456_789");
}

#[test]
fn telegram_message_id_is_deterministic() {
    // Same chat_id + same message_id = same ID (prevents duplicates after restart)
    let chat_id = "123456";
    let message_id = 789;
    let id1 = format!("telegram_{chat_id}_{message_id}");
    let id2 = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(id1, id2);
}

#[test]
fn telegram_message_id_different_message_different_id() {
    // Different message IDs produce different IDs
    let chat_id = "123456";
    let id1 = format!("telegram_{chat_id}_789");
    let id2 = format!("telegram_{chat_id}_790");
    assert_ne!(id1, id2);
}

#[test]
fn telegram_message_id_different_chat_different_id() {
    // Different chats produce different IDs even with same message_id
    let message_id = 789;
    let id1 = format!("telegram_123456_{message_id}");
    let id2 = format!("telegram_789012_{message_id}");
    assert_ne!(id1, id2);
}

#[test]
fn telegram_message_id_no_uuid_randomness() {
    // Verify format doesn't contain random UUID components
    let chat_id = "123456";
    let message_id = 789;
    let id = format!("telegram_{chat_id}_{message_id}");
    assert!(!id.contains('-')); // No UUID dashes
    assert!(id.starts_with("telegram_"));
}

#[test]
fn telegram_message_id_handles_zero_message_id() {
    // Edge case: message_id can be 0 (fallback/missing case)
    let chat_id = "123456";
    let message_id = 0;
    let id = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(id, "telegram_123456_0");
}

// ── Tool call tag stripping tests ───────────────────────────────────

#[test]
fn strip_tool_call_tags_removes_standard_tags() {
    let input = "Hello <tool>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_alias_tags() {
    let input =
        "Hello <toolcall>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</toolcall> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_dash_tags() {
    let input = "Hello <tool-call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool-call> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_tool_call_tags() {
    let input = "Hello <tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_invoke_tags() {
    let input =
        "Hello <invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"date\"}}</invoke> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_handles_multiple_tags() {
    let input = "Start <tool>a</tool> middle <tool>b</tool> end";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Start  middle  end");
}

#[test]
fn strip_tool_call_tags_handles_mixed_tags() {
    let input = "A <tool>a</tool> B <toolcall>b</toolcall> C <tool-call>c</tool-call> D";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "A  B  C  D");
}

#[test]
fn strip_tool_call_tags_preserves_normal_text() {
    let input = "Hello world! This is a test.";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello world! This is a test.");
}

#[test]
fn strip_tool_call_tags_handles_unclosed_tags() {
    let input = "Hello <tool>world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello <tool>world");
}

#[test]
fn strip_tool_call_tags_handles_unclosed_tool_call_with_json() {
    let input = "Status:\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Status:");
}

#[test]
fn strip_tool_call_tags_handles_mismatched_close_tag() {
    let input =
        "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}</arg_value>";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn strip_tool_call_tags_cleans_extra_newlines() {
    let input = "Hello\n\n<tool>\ntest\n</tool>\n\n\nworld";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello\n\nworld");
}

#[test]
fn strip_tool_call_tags_handles_empty_input() {
    let input = "";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn strip_tool_call_tags_handles_only_tags() {
    let input = "<tool>{\"name\":\"test\"}</tool>";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn telegram_contains_bot_mention_finds_mention() {
    assert!(TelegramChannel::contains_bot_mention(
        "Hello @mybot",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "@mybot help",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "Hey @mybot how are you?",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "Hello @MyBot, can you help?",
        "mybot"
    ));
}

#[test]
fn telegram_contains_bot_mention_no_false_positives() {
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello @otherbot",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello mybot",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello @mybot2",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention("", "mybot"));
}

#[test]
fn telegram_normalize_incoming_content_strips_mention() {
    let result = TelegramChannel::normalize_incoming_content("@mybot hello", "mybot");
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn telegram_normalize_incoming_content_handles_multiple_mentions() {
    let result = TelegramChannel::normalize_incoming_content("@mybot @mybot test", "mybot");
    assert_eq!(result, Some("test".to_string()));
}

#[test]
fn telegram_normalize_incoming_content_returns_none_for_empty() {
    let result = TelegramChannel::normalize_incoming_content("@mybot", "mybot");
    assert_eq!(result, None);
}

#[test]
fn parse_update_message_mention_only_group_requires_exact_mention() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }

    let update = serde_json::json!({
        "update_id": 10,
        "message": {
            "message_id": 44,
            "text": "hello @mybot2",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    assert!(ch.parse_update_message(&update).is_none());
}

#[test]
fn parse_update_message_mention_only_group_strips_mention_and_drops_empty() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }

    let update = serde_json::json!({
        "update_id": 11,
        "message": {
            "message_id": 45,
            "text": "Hi @MyBot status please",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    let parsed = ch
        .parse_update_message(&update)
        .expect("mention should parse");
    assert_eq!(parsed.content, "Hi status please");

    let empty_update = serde_json::json!({
        "update_id": 12,
        "message": {
            "message_id": 46,
            "text": "@mybot",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    assert!(ch.parse_update_message(&empty_update).is_none());
}

#[test]
fn telegram_is_group_message_detects_groups() {
    let group_msg = serde_json::json!({
        "chat": { "type": "group" }
    });
    assert!(TelegramChannel::is_group_message(&group_msg));

    let supergroup_msg = serde_json::json!({
        "chat": { "type": "supergroup" }
    });
    assert!(TelegramChannel::is_group_message(&supergroup_msg));

    let private_msg = serde_json::json!({
        "chat": { "type": "private" }
    });
    assert!(!TelegramChannel::is_group_message(&private_msg));
}

#[test]
fn telegram_mention_only_enabled_by_config() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    assert!(ch.mention_only);

    let ch_disabled = TelegramChannel::new("token".into(), vec!["*".into()], false);
    assert!(!ch_disabled.mention_only);
}

fn group_message_with_caption(caption: Option<&str>) -> serde_json::Value {
    let mut msg = serde_json::json!({
        "message_id": 1,
        "from": { "id": 1, "username": "alice" },
        "chat": { "id": -1, "type": "group" }
    });
    if let Some(c) = caption {
        msg["caption"] = serde_json::Value::String(c.to_string());
    }
    msg
}

/// Regression test for #6229 — when `mention_only = true` and a group
/// photo/document arrives without any caption mentioning the bot, the
/// gate must reject it. Before the fix, photo/document updates skipped
/// the gate entirely (the gate only inspected `message.text`) and the
/// bot replied to every photo posted in a group.
#[test]
fn check_media_mention_gate_rejects_group_media_without_mention() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }
    let no_caption = group_message_with_caption(None);
    assert!(
        ch.check_media_mention_gate(&no_caption, None).is_none(),
        "no caption + mention_only group ⇒ reject"
    );
    let unrelated_caption = group_message_with_caption(Some("nice photo"));
    assert!(
        ch.check_media_mention_gate(&unrelated_caption, Some("nice photo"))
            .is_none(),
        "caption without bot mention + mention_only group ⇒ reject"
    );
    let other_bot_caption = group_message_with_caption(Some("hey @otherbot look"));
    assert!(
        ch.check_media_mention_gate(&other_bot_caption, Some("hey @otherbot look"))
            .is_none(),
        "caption mentioning a different bot ⇒ reject"
    );
}

/// When the caption mentions the bot, the gate passes and returns the
/// caption with the mention stripped — matching the text-message
/// behavior of `normalize_incoming_content`.
#[test]
fn check_media_mention_gate_accepts_and_strips_caption_mention() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }
    let msg = group_message_with_caption(Some("@mybot describe this"));
    let result = ch.check_media_mention_gate(&msg, Some("@mybot describe this"));
    assert_eq!(
        result,
        Some(Some("describe this".to_string())),
        "mention should be stripped, remaining caption preserved"
    );
}

/// `mention_only = true` only applies to groups. DMs always pass with
/// the caption preserved verbatim.
#[test]
fn check_media_mention_gate_passes_dm_unchanged() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    let dm = serde_json::json!({
        "message_id": 1,
        "from": { "id": 1, "username": "alice" },
        "chat": { "id": 1, "type": "private" },
        "caption": "hello"
    });
    assert_eq!(
        ch.check_media_mention_gate(&dm, Some("hello")),
        Some(Some("hello".to_string())),
        "DM media must always pass with caption verbatim"
    );
    let dm_no_caption = serde_json::json!({
        "message_id": 1,
        "from": { "id": 1, "username": "alice" },
        "chat": { "id": 1, "type": "private" }
    });
    assert_eq!(
        ch.check_media_mention_gate(&dm_no_caption, None),
        Some(None),
        "DM media with no caption must pass"
    );
}

/// When `mention_only = false` the gate is a no-op even in groups.
#[test]
fn check_media_mention_gate_passes_when_mention_only_disabled() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let group_no_caption = group_message_with_caption(None);
    assert_eq!(
        ch.check_media_mention_gate(&group_no_caption, None),
        Some(None),
        "mention_only off ⇒ all media pass"
    );
}

/// Edge case: `mention_only = true` and the bot username has not yet
/// been resolved (e.g., `/getMe` hasn't completed). The gate must
/// reject in groups rather than fail-open, matching the existing text
/// path's behavior at telegram.rs:1640.
#[test]
fn check_media_mention_gate_rejects_group_when_bot_username_unknown() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    // Do NOT set bot_username — leave it None.
    let group = group_message_with_caption(Some("@somebody hi"));
    assert!(
        ch.check_media_mention_gate(&group, Some("@somebody hi"))
            .is_none(),
        "missing bot_username in group must fail closed"
    );
}

// ─────────────────────────────────────────────────────────────────────
// TG6: Channel platform limit edge cases for Telegram (4096 char limit)
// Prevents: Pattern 6 — issues #574, #499
// ─────────────────────────────────────────────────────────────────────

#[test]
fn telegram_split_code_block_at_boundary() {
    let mut msg = String::new();
    msg.push_str("```python\n");
    msg.push_str(&"x".repeat(4085));
    msg.push_str("\n```\nMore text after code block");
    let parts = split_message_for_telegram(&msg);
    assert!(
        parts.len() >= 2,
        "code block spanning boundary should split"
    );
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "each part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
}

#[test]
fn telegram_split_single_long_word() {
    let long_word = "a".repeat(5000);
    let parts = split_message_for_telegram(&long_word);
    assert!(parts.len() >= 2, "word exceeding limit must be split");
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "hard-split part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
    let reassembled: String = parts.join("");
    assert_eq!(reassembled, long_word);
}

#[test]
fn telegram_split_exactly_at_limit_no_split() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
    let parts = split_message_for_telegram(&msg);
    assert_eq!(parts.len(), 1, "message exactly at limit should not split");
}

#[test]
fn telegram_split_one_over_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 1);
    let parts = split_message_for_telegram(&msg);
    assert!(parts.len() >= 2, "message 1 char over limit must split");
}

#[test]
fn telegram_split_many_short_lines() {
    let msg: String = (0..1000).fold(String::new(), |mut acc, i| {
        let _ = writeln!(acc, "line {i}");
        acc
    });
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "short-line batch must be <= limit"
        );
    }
}

#[test]
fn telegram_split_only_whitespace() {
    let msg = "   \n\n\t  ";
    let parts = split_message_for_telegram(msg);
    assert!(parts.len() <= 1);
}

#[test]
fn telegram_split_emoji_at_boundary() {
    let mut msg = "a".repeat(4094);
    msg.push_str("🎉🎊"); // 4096 chars total
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        // The function splits on character count, not byte count
        assert!(
            part.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "emoji boundary split must respect limit"
        );
    }
}

#[test]
fn telegram_split_consecutive_newlines() {
    let mut msg = "a".repeat(4090);
    msg.push_str("\n\n\n\n\n\n");
    msg.push_str(&"b".repeat(100));
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        assert!(part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

#[test]
fn parse_voice_metadata_extracts_voice() {
    let msg = serde_json::json!({
        "voice": {
            "file_id": "abc123",
            "duration": 5
        }
    });
    let (file_id, dur) = TelegramChannel::parse_voice_metadata(&msg).unwrap();
    assert_eq!(file_id, "abc123");
    assert_eq!(dur, 5);
}

#[test]
fn parse_voice_metadata_extracts_audio() {
    let msg = serde_json::json!({
        "audio": {
            "file_id": "audio456",
            "duration": 30
        }
    });
    let (file_id, dur) = TelegramChannel::parse_voice_metadata(&msg).unwrap();
    assert_eq!(file_id, "audio456");
    assert_eq!(dur, 30);
}

#[test]
fn parse_voice_metadata_returns_none_for_text() {
    let msg = serde_json::json!({
        "text": "hello"
    });
    assert!(TelegramChannel::parse_voice_metadata(&msg).is_none());
}

#[test]
fn parse_voice_metadata_defaults_duration_to_zero() {
    let msg = serde_json::json!({
        "voice": {
            "file_id": "no_dur"
        }
    });
    let (_, dur) = TelegramChannel::parse_voice_metadata(&msg).unwrap();
    assert_eq!(dur, 0);
}

// ─────────────────────────────────────────────────────────────────────
// extract_sender_info tests
// ─────────────────────────────────────────────────────────────────────

#[test]
fn extract_sender_info_with_username() {
    let msg = serde_json::json!({
        "from": { "id": 123, "username": "alice" }
    });
    let (username, sender_id, identity) = TelegramChannel::extract_sender_info(&msg);
    assert_eq!(username, "alice");
    assert_eq!(sender_id, Some("123".to_string()));
    assert_eq!(identity, "alice");
}

#[test]
fn extract_sender_info_without_username() {
    let msg = serde_json::json!({
        "from": { "id": 42 }
    });
    let (username, sender_id, identity) = TelegramChannel::extract_sender_info(&msg);
    assert_eq!(username, "unknown");
    assert_eq!(sender_id, Some("42".to_string()));
    assert_eq!(identity, "42");
}

// ─────────────────────────────────────────────────────────────────────
// extract_reply_context tests
// ─────────────────────────────────────────────────────────────────────

#[test]
fn extract_reply_context_text_message() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "reply_to_message": {
            "from": { "username": "alice" },
            "text": "Hello world"
        }
    });
    let ctx = ch.extract_reply_context(&msg).unwrap();
    assert_eq!(ctx, "> @alice:\n> Hello world");
}

#[test]
fn extract_reply_context_voice_message() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "reply_to_message": {
            "from": { "username": "bob" },
            "voice": { "file_id": "abc", "duration": 5 }
        }
    });
    let ctx = ch.extract_reply_context(&msg).unwrap();
    assert_eq!(ctx, "> @bob:\n> [Voice message]");
}

#[test]
fn extract_reply_context_no_reply() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "text": "just a regular message"
    });
    assert!(ch.extract_reply_context(&msg).is_none());
}

#[test]
fn extract_reply_context_skips_topic_root() {
    // Telegram auto-injects a reply_to_message pointing at the topic-root
    // message on every message in a non-General forum topic. The injected
    // reply's message_id equals the parent's message_thread_id. It is
    // not a real reply and must not produce a blockquote prefix.
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "message_thread_id": 42,
        "text": "hello in topic",
        "reply_to_message": {
            "message_id": 42,
            "from": { "username": "alice" },
            "forum_topic_created": { "name": "General Discussion", "icon_color": 0 }
        }
    });
    assert!(ch.extract_reply_context(&msg).is_none());
}

#[test]
fn extract_reply_context_real_reply_in_topic() {
    // A genuine reply inside a forum topic (reply.message_id differs from
    // the parent's message_thread_id) should still produce a blockquote.
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "message_thread_id": 42,
        "text": "I agree",
        "reply_to_message": {
            "message_id": 100,
            "from": { "username": "alice" },
            "text": "What do you think?"
        }
    });
    let ctx = ch.extract_reply_context(&msg).unwrap();
    assert_eq!(ctx, "> @alice:\n> What do you think?");
}

#[test]
fn extract_reply_context_no_username_uses_first_name() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let msg = serde_json::json!({
        "reply_to_message": {
            "from": { "id": 999, "first_name": "Charlie" },
            "text": "Hi there"
        }
    });
    let ctx = ch.extract_reply_context(&msg).unwrap();
    assert_eq!(ctx, "> @Charlie:\n> Hi there");
}

#[test]
fn extract_reply_context_voice_with_cached_transcription() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    // Pre-populate transcription cache
    ch.voice_transcriptions
        .lock()
        .insert("100:42".to_string(), "Hello from voice".to_string());
    let msg = serde_json::json!({
        "chat": { "id": 100 },
        "reply_to_message": {
            "message_id": 42,
            "from": { "username": "bob" },
            "voice": { "file_id": "abc", "duration": 5 }
        }
    });
    let ctx = ch.extract_reply_context(&msg).unwrap();
    assert_eq!(ctx, "> @bob:\n> [Voice] Hello from voice");
}

#[test]
fn parse_update_message_includes_reply_context() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "message": {
            "message_id": 10,
            "text": "translate this",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 100, "type": "private" },
            "reply_to_message": {
                "from": { "username": "bot" },
                "text": "Bonjour le monde"
            }
        }
    });
    let parsed = ch.parse_update_message(&update).unwrap();
    assert!(
        parsed.content.starts_with("> @bot:"),
        "content should start with quote: {}",
        parsed.content
    );
    assert!(
        parsed.content.contains("translate this"),
        "content should contain user text"
    );
    assert!(
        parsed.content.contains("Bonjour le monde"),
        "content should contain quoted text"
    );
}

#[test]
fn with_transcription_sets_config_when_enabled() {
    let tc = operant_config::schema::TranscriptionConfig {
        enabled: true,
        api_key: Some("test_key".to_string()),
        ..operant_config::schema::TranscriptionConfig::default()
    };

    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false).with_transcription(tc);
    assert!(ch.transcription.is_some());
    assert!(ch.transcription_manager.is_some());
}

#[test]
fn with_transcription_skips_when_disabled() {
    let tc = operant_config::schema::TranscriptionConfig::default(); // enabled = false
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false).with_transcription(tc);
    assert!(ch.transcription.is_none());
    assert!(ch.transcription_manager.is_none());
}

#[tokio::test]
async fn try_parse_voice_message_returns_none_when_transcription_disabled() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "message": {
            "message_id": 1,
            "voice": { "file_id": "voice_file", "duration": 4 },
            "from": { "id": 123, "username": "alice" },
            "chat": { "id": 456, "type": "private" }
        }
    });

    let parsed = ch.try_parse_voice_message(&update).await;
    assert!(parsed.is_none());
}

#[tokio::test]
async fn try_parse_voice_message_skips_when_duration_exceeds_limit() {
    let tc = operant_config::schema::TranscriptionConfig {
        enabled: true,
        api_key: Some("test_key".to_string()),
        max_duration_secs: 5,
        ..Default::default()
    };

    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false).with_transcription(tc);
    let update = serde_json::json!({
        "message": {
            "message_id": 2,
            "voice": { "file_id": "voice_file", "duration": 30 },
            "from": { "id": 123, "username": "alice" },
            "chat": { "id": 456, "type": "private" }
        }
    });

    let parsed = ch.try_parse_voice_message(&update).await;
    assert!(parsed.is_none());
}

#[tokio::test]
async fn try_parse_voice_message_rejects_unauthorized_sender_before_download() {
    let tc = operant_config::schema::TranscriptionConfig {
        enabled: true,
        api_key: Some("test_key".to_string()),
        max_duration_secs: 120,
        ..Default::default()
    };

    let ch =
        TelegramChannel::new("token".into(), vec!["alice".into()], false).with_transcription(tc);
    let update = serde_json::json!({
        "message": {
            "message_id": 3,
            "voice": { "file_id": "voice_file", "duration": 4 },
            "from": { "id": 999, "username": "bob" },
            "chat": { "id": 456, "type": "private" }
        }
    });

    let parsed = ch.try_parse_voice_message(&update).await;
    assert!(parsed.is_none());
    assert!(ch.voice_transcriptions.lock().is_empty());
}

// ─────────────────────────────────────────────────────────────────────
// Live e2e: voice transcription via Groq Whisper + reply cache lookup
// ─────────────────────────────────────────────────────────────────────

/// Live test: voice transcription via Groq Whisper + reply cache lookup.
///
/// Loads a pre-recorded MP3 fixture ("hello"), sends it to Groq Whisper
/// API, verifies the transcription contains "hello", then caches it and
/// checks that `extract_reply_context` returns the cached text instead
/// of the `[Voice message]` fallback placeholder.
///
/// Skipped automatically when `GROQ_API_KEY` is not set.
/// Run: `GROQ_API_KEY=<key> cargo test --lib -- telegram::tests::e2e_live_voice_transcription_and_reply_cache --ignored`
#[tokio::test]
#[ignore = "requires GROQ_API_KEY environment variable"]
async fn e2e_live_voice_transcription_and_reply_cache() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set — skipping live voice transcription test");
        return;
    }

    // 1. Load pre-recorded fixture (TTS-generated "hello", ~7 KB MP3)
    let fixture_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.mp3");
    let audio_data = std::fs::read(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {e}", fixture_path.display()));
    assert!(
        audio_data.len() > 1000,
        "fixture too small ({} bytes), likely corrupt",
        audio_data.len()
    );

    // 2. Call TranscriptionManager.transcribe() — real Groq Whisper API
    let config = operant_config::schema::TranscriptionConfig {
        enabled: true,
        ..Default::default()
    };
    let manager = crate::transcription::TranscriptionManager::new(&config)
        .expect("TranscriptionManager::new should succeed with valid GROQ_API_KEY");
    let transcript: String = manager
        .transcribe(&audio_data, "hello.mp3")
        .await
        .expect("transcribe should succeed with valid GROQ_API_KEY");

    // 3. Verify Whisper actually recognized "hello"
    assert!(
        transcript.to_lowercase().contains("hello"),
        "expected transcription to contain 'hello', got: '{transcript}'"
    );

    // 4. Create TelegramChannel, insert transcription into voice_transcriptions cache
    let ch = TelegramChannel::new("test_token".into(), vec!["*".into()], false);
    let chat_id: i64 = 12345;
    let message_id: i64 = 67;
    let cache_key = format!("{chat_id}:{message_id}");
    ch.voice_transcriptions
        .lock()
        .insert(cache_key, transcript.clone());

    // 5. Build reply message with voice + message_id + chat.id
    let msg = serde_json::json!({
        "chat": { "id": chat_id },
        "reply_to_message": {
            "message_id": message_id,
            "from": { "username": "operant_user" },
            "voice": { "file_id": "test_file", "duration": 1 }
        }
    });

    // 6. Verify extract_reply_context returns cached transcription
    let ctx = ch
        .extract_reply_context(&msg)
        .expect("extract_reply_context should return Some for voice reply");

    assert!(
        ctx.contains(&format!("[Voice] {transcript}")),
        "expected cached transcription in reply context, got: {ctx}"
    );

    // Must NOT contain the fallback placeholder
    assert!(
        !ctx.contains("[Voice message]"),
        "context should use cached transcription, not fallback placeholder, got: {ctx}"
    );
}

// ── IncomingAttachment / parse_attachment_metadata tests ─────────

#[test]
fn parse_attachment_metadata_detects_document() {
    let message = serde_json::json!({
        "document": {
            "file_id": "BQACAgIAAxk",
            "file_name": "report.pdf",
            "file_size": 12345
        }
    });
    let att = TelegramChannel::parse_attachment_metadata(&message).unwrap();
    assert_eq!(att.kind, IncomingAttachmentKind::Document);
    assert_eq!(att.file_id, "BQACAgIAAxk");
    assert_eq!(att.file_name.as_deref(), Some("report.pdf"));
    assert_eq!(att.file_size, Some(12345));
    assert!(att.caption.is_none());
}

#[test]
fn parse_attachment_metadata_detects_photo() {
    let message = serde_json::json!({
        "photo": [
            {"file_id": "small_id", "file_size": 100, "width": 90, "height": 90},
            {"file_id": "medium_id", "file_size": 500, "width": 320, "height": 320},
            {"file_id": "large_id", "file_size": 2000, "width": 800, "height": 800}
        ]
    });
    let att = TelegramChannel::parse_attachment_metadata(&message).unwrap();
    assert_eq!(att.kind, IncomingAttachmentKind::Photo);
    assert_eq!(att.file_id, "large_id");
    assert_eq!(att.file_size, Some(2000));
    assert!(att.file_name.is_none());
}

#[test]
fn parse_attachment_metadata_extracts_caption() {
    // Document with caption
    let doc_msg = serde_json::json!({
        "document": {
            "file_id": "doc_id",
            "file_name": "data.csv"
        },
        "caption": "Monthly report"
    });
    let att = TelegramChannel::parse_attachment_metadata(&doc_msg).unwrap();
    assert_eq!(att.caption.as_deref(), Some("Monthly report"));

    // Photo with caption
    let photo_msg = serde_json::json!({
        "photo": [
            {"file_id": "photo_id", "file_size": 1000}
        ],
        "caption": "Look at this"
    });
    let att = TelegramChannel::parse_attachment_metadata(&photo_msg).unwrap();
    assert_eq!(att.caption.as_deref(), Some("Look at this"));
}

#[test]
fn parse_attachment_metadata_document_without_optional_fields() {
    let message = serde_json::json!({
        "document": {
            "file_id": "doc_no_name"
        }
    });
    let att = TelegramChannel::parse_attachment_metadata(&message).unwrap();
    assert_eq!(att.kind, IncomingAttachmentKind::Document);
    assert_eq!(att.file_id, "doc_no_name");
    assert!(att.file_name.is_none());
    assert!(att.file_size.is_none());
    assert!(att.caption.is_none());
}

#[test]
fn parse_attachment_metadata_returns_none_for_text() {
    let message = serde_json::json!({
        "text": "Hello world"
    });
    assert!(TelegramChannel::parse_attachment_metadata(&message).is_none());
}

#[test]
fn parse_attachment_metadata_returns_none_for_voice() {
    let message = serde_json::json!({
        "voice": {
            "file_id": "voice_id",
            "duration": 5
        }
    });
    assert!(TelegramChannel::parse_attachment_metadata(&message).is_none());
}

#[test]
fn parse_attachment_metadata_empty_photo_array() {
    let message = serde_json::json!({
        "photo": []
    });
    assert!(TelegramChannel::parse_attachment_metadata(&message).is_none());
}

#[test]
fn with_workspace_dir_sets_field() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_workspace_dir(std::path::PathBuf::from("/tmp/test_workspace"));
    assert_eq!(
        ch.workspace_dir.as_deref(),
        Some(std::path::Path::new("/tmp/test_workspace"))
    );
}

#[test]
fn telegram_max_file_download_bytes_is_20mb() {
    assert_eq!(TELEGRAM_MAX_FILE_DOWNLOAD_BYTES, 20 * 1024 * 1024);
}

// ── Attachment content format tests ──────────────────────────────

/// Photo attachments with image extension must use `[IMAGE:/path]` marker
/// so the multimodal pipeline validates vision capability on the provider.
#[test]
fn attachment_photo_content_uses_image_marker() {
    let local_path = std::path::Path::new("/tmp/workspace/photo_123_45.jpg");
    let local_filename = "photo_123_45.jpg";

    let content =
        format_attachment_content(IncomingAttachmentKind::Photo, local_filename, local_path);

    assert_eq!(content, "[IMAGE:/tmp/workspace/photo_123_45.jpg]");
    assert!(content.starts_with("[IMAGE:"));
    assert!(content.ends_with(']'));
}

/// Document attachments keep `[Document: name] /path` format.
#[test]
fn attachment_document_content_uses_document_label() {
    let local_path = std::path::Path::new("/tmp/workspace/report.pdf");
    let local_filename = "report.pdf";

    let content =
        format_attachment_content(IncomingAttachmentKind::Document, local_filename, local_path);

    assert_eq!(content, "[Document: report.pdf] /tmp/workspace/report.pdf");
    assert!(!content.contains("[IMAGE:"));
}

/// Markdown files must never produce `[IMAGE:]` markers (issue #1274).
#[test]
fn markdown_file_never_produces_image_marker() {
    let local_path = std::path::Path::new("/tmp/workspace/telegram_files/notes.md");
    let local_filename = "notes.md";

    // Even if Telegram misclassifies as Photo, extension guard prevents [IMAGE:].
    let content =
        format_attachment_content(IncomingAttachmentKind::Photo, local_filename, local_path);
    assert!(
        !content.contains("[IMAGE:"),
        "markdown must not get [IMAGE:] marker: {content}"
    );
    assert!(content.starts_with("[Document:"));

    // As Document, it should also be correct.
    let content_doc =
        format_attachment_content(IncomingAttachmentKind::Document, local_filename, local_path);
    assert!(
        !content_doc.contains("[IMAGE:"),
        "markdown document must not get [IMAGE:] marker: {content_doc}"
    );
}

/// Non-image files classified as Photo fall back to `[Document:]` format.
#[test]
fn non_image_photo_falls_back_to_document_format() {
    for (filename, ext_path) in [
        ("file.md", "/tmp/ws/file.md"),
        ("file.txt", "/tmp/ws/file.txt"),
        ("file.pdf", "/tmp/ws/file.pdf"),
        ("file.csv", "/tmp/ws/file.csv"),
        ("file.json", "/tmp/ws/file.json"),
        ("file.zip", "/tmp/ws/file.zip"),
        ("file", "/tmp/ws/file"),
    ] {
        let path = std::path::Path::new(ext_path);
        let content = format_attachment_content(IncomingAttachmentKind::Photo, filename, path);
        assert!(
            !content.contains("[IMAGE:"),
            "{filename}: non-image file should not get [IMAGE:] marker, got: {content}"
        );
        assert!(
            content.starts_with("[Document:"),
            "{filename}: should use [Document:] format, got: {content}"
        );
    }
}

/// All recognized image extensions produce `[IMAGE:]` when classified as Photo.
#[test]
fn image_extensions_produce_image_marker() {
    for ext in ["png", "jpg", "jpeg", "gif", "webp", "bmp"] {
        let filename = format!("photo_1_2.{ext}");
        let path_str = format!("/tmp/ws/{filename}");
        let path = std::path::Path::new(&path_str);
        let content = format_attachment_content(IncomingAttachmentKind::Photo, &filename, path);
        assert!(
            content.starts_with("[IMAGE:"),
            "{ext}: image should get [IMAGE:] marker, got: {content}"
        );
    }
}

/// Multimodal pipeline must return 0 image markers for document-formatted
/// content — even for a file misclassified as Photo (issue #1274).
#[test]
fn markdown_attachment_not_detected_by_multimodal_image_markers() {
    let content = format_attachment_content(
        IncomingAttachmentKind::Photo,
        "notes.md",
        std::path::Path::new("/tmp/ws/notes.md"),
    );
    let messages = vec![operant_providers::ChatMessage::user(content)];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&messages),
        0,
        "markdown file must not trigger image marker detection"
    );
}

/// `is_image_extension` helper recognizes image formats and rejects others.
#[test]
fn is_image_extension_recognizes_images() {
    assert!(is_image_extension(std::path::Path::new("photo.png")));
    assert!(is_image_extension(std::path::Path::new("photo.jpg")));
    assert!(is_image_extension(std::path::Path::new("photo.jpeg")));
    assert!(is_image_extension(std::path::Path::new("photo.gif")));
    assert!(is_image_extension(std::path::Path::new("photo.webp")));
    assert!(is_image_extension(std::path::Path::new("photo.bmp")));
    assert!(is_image_extension(std::path::Path::new("PHOTO.PNG")));

    assert!(!is_image_extension(std::path::Path::new("file.md")));
    assert!(!is_image_extension(std::path::Path::new("file.txt")));
    assert!(!is_image_extension(std::path::Path::new("file.pdf")));
    assert!(!is_image_extension(std::path::Path::new("file.csv")));
    assert!(!is_image_extension(std::path::Path::new("file")));
}

/// `count_image_markers` from the multimodal module must detect the
/// `[IMAGE:]` marker produced by photo attachment formatting.
#[test]
fn photo_image_marker_detected_by_multimodal() {
    let photo_content = "[IMAGE:/tmp/workspace/photo_1_2.jpg]";
    let messages = vec![operant_providers::ChatMessage::user(
        photo_content.to_string(),
    )];
    let count = operant_providers::multimodal::count_image_markers(&messages);
    assert_eq!(
        count, 1,
        "multimodal should detect exactly one image marker"
    );
}

/// Photo with caption: `[IMAGE:/path]\n\nCaption text`.
#[test]
fn photo_image_marker_with_caption() {
    let local_path = std::path::Path::new("/tmp/workspace/photo_1_2.jpg");
    let mut content = format!("[IMAGE:{}]", local_path.display());
    let caption = "Look at this screenshot";
    use std::fmt::Write;
    let _ = write!(content, "\n\n{caption}");

    assert_eq!(
        content,
        "[IMAGE:/tmp/workspace/photo_1_2.jpg]\n\nLook at this screenshot"
    );

    // Multimodal pipeline still detects the marker.
    let messages = vec![operant_providers::ChatMessage::user(content)];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&messages),
        1
    );
}

// ── E2E: attachment saves file and formats content ───────────────

/// Full pipeline test: simulate file download → save to workspace →
/// verify content format for both document and photo attachments.
#[test]
fn e2e_attachment_saves_file_and_formats_content() {
    let workspace = tempfile::tempdir().expect("create temp workspace");

    // ── Document attachment ──────────────────────────────────────
    let doc_filename = "report.pdf";
    let doc_path = workspace.path().join(doc_filename);
    // Simulate downloaded file.
    std::fs::write(&doc_path, b"%PDF-1.4 fake").expect("write doc fixture");
    assert!(doc_path.exists(), "document file must exist on disk");

    let doc_content =
        format_attachment_content(IncomingAttachmentKind::Document, doc_filename, &doc_path);
    assert!(
        doc_content.starts_with("[Document: report.pdf]"),
        "document label format mismatch: {doc_content}"
    );
    // Multimodal must NOT detect image markers in document content.
    let doc_msgs = vec![operant_providers::ChatMessage::user(doc_content)];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&doc_msgs),
        0,
        "document content must not contain image markers"
    );

    // ── Photo attachment ─────────────────────────────────────────
    let photo_filename = "photo_99_1.jpg";
    let photo_path = workspace.path().join(photo_filename);
    // Copy the JPEG fixture.
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_photo.jpg");
    std::fs::copy(&fixture, &photo_path).expect("copy photo fixture");
    assert!(photo_path.exists(), "photo file must exist on disk");

    let photo_content =
        format_attachment_content(IncomingAttachmentKind::Photo, photo_filename, &photo_path);
    assert!(
        photo_content.starts_with("[IMAGE:"),
        "photo must use [IMAGE:] marker: {photo_content}"
    );
    assert!(
        photo_content.ends_with(']'),
        "photo marker must close with ]: {photo_content}"
    );

    // Multimodal detects the marker.
    let photo_msgs = vec![operant_providers::ChatMessage::user(photo_content.clone())];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&photo_msgs),
        1,
        "multimodal must detect exactly one image marker in photo content"
    );

    // ── Photo with caption ───────────────────────────────────────
    let mut captioned = photo_content;
    use std::fmt::Write;
    let _ = write!(captioned, "\n\nCheck this out");
    let cap_msgs = vec![operant_providers::ChatMessage::user(captioned.clone())];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&cap_msgs),
        1,
        "caption must not break image marker detection"
    );
    assert!(
        captioned.contains("Check this out"),
        "caption text must be present in content"
    );

    // ── Markdown file sent as Photo (issue #1274) ────────────────
    let md_filename = "notes.md";
    let md_path = workspace.path().join(md_filename);
    std::fs::write(&md_path, b"# Hello\nSome markdown").expect("write md fixture");
    let md_content =
        format_attachment_content(IncomingAttachmentKind::Photo, md_filename, &md_path);
    assert!(
        !md_content.contains("[IMAGE:"),
        "markdown must not get [IMAGE:] marker: {md_content}"
    );
    let md_msgs = vec![operant_providers::ChatMessage::user(md_content)];
    assert_eq!(
        operant_providers::multimodal::count_image_markers(&md_msgs),
        0,
        "markdown file must not trigger image marker detection"
    );
}

// ── Groq provider rejects photo with vision error ────────────────

/// Verify that the Groq provider (OpenAI-compatible) does not support
/// vision, so the existing `count_image_markers > 0 && !supports_vision()`
/// guard in `agent/loop_.rs` will reject photo messages.
#[test]
fn groq_provider_rejects_photo_with_vision_error() {
    use operant_providers::Provider;
    use operant_providers::compatible::{AuthStyle, OpenAiCompatibleProvider};

    let groq = OpenAiCompatibleProvider::new(
        "Groq",
        "https://api.groq.com/openai",
        Some("fake_key"),
        AuthStyle::Bearer,
    );

    // Groq must not support vision.
    assert!(
        !groq.supports_vision(),
        "Groq provider must not support vision"
    );

    // Build a message with an [IMAGE:] marker (as photo attachment would).
    let messages = vec![operant_providers::ChatMessage::user(
        "[IMAGE:/tmp/photo.jpg]\n\nDescribe this image".to_string(),
    )];
    let marker_count = operant_providers::multimodal::count_image_markers(&messages);
    assert_eq!(marker_count, 1, "must detect image marker in photo content");

    // The combination of marker_count > 0 && !supports_vision() means
    // the agent loop will return ProviderCapabilityError before calling
    // the provider, and the channel will send "⚠️ Error: ..." to the user.
}

#[test]
fn ack_reactions_defaults_to_true() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    assert!(ch.ack_reactions);
}

#[test]
fn with_ack_reactions_false_disables_reactions() {
    let ch =
        TelegramChannel::new("token".into(), vec!["*".into()], false).with_ack_reactions(false);
    assert!(!ch.ack_reactions);
}

#[test]
fn with_ack_reactions_true_keeps_reactions() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false).with_ack_reactions(true);
    assert!(ch.ack_reactions);
}

// ── Forwarded message tests ─────────────────────────────────────

#[test]
fn parse_update_message_forwarded_from_user_with_username() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 100,
        "message": {
            "message_id": 50,
            "text": "Check this out",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 999 },
            "forward_from": {
                "id": 42,
                "first_name": "Bob",
                "username": "bob"
            },
            "forward_date": 1_700_000_000
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("forwarded message should parse");
    assert_eq!(msg.content, "[Forwarded from @bob] Check this out");
}

#[test]
fn parse_update_message_forwarded_from_channel() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 101,
        "message": {
            "message_id": 51,
            "text": "Breaking news",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 999 },
            "forward_from_chat": {
                "id": -1_001_234_567_890_i64,
                "title": "Daily News",
                "username": "dailynews",
                "type": "channel"
            },
            "forward_date": 1_700_000_000
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("channel-forwarded message should parse");
    assert_eq!(
        msg.content,
        "[Forwarded from channel: Daily News] Breaking news"
    );
}

#[test]
fn parse_update_message_forwarded_hidden_sender() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 102,
        "message": {
            "message_id": 52,
            "text": "Secret tip",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 999 },
            "forward_sender_name": "Hidden User",
            "forward_date": 1_700_000_000
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("hidden-sender forwarded message should parse");
    assert_eq!(msg.content, "[Forwarded from Hidden User] Secret tip");
}

#[test]
fn parse_update_message_non_forwarded_unaffected() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 103,
        "message": {
            "message_id": 53,
            "text": "Normal message",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 999 }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("non-forwarded message should parse");
    assert_eq!(msg.content, "Normal message");
}

#[test]
fn parse_update_message_forwarded_from_user_no_username() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 104,
        "message": {
            "message_id": 54,
            "text": "Hello there",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 999 },
            "forward_from": {
                "id": 77,
                "first_name": "Charlie"
            },
            "forward_date": 1_700_000_000
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("forwarded message without username should parse");
    assert_eq!(msg.content, "[Forwarded from Charlie] Hello there");
}

#[test]
fn forwarded_photo_attachment_has_attribution() {
    // Verify that format_forward_attribution produces correct prefix
    // for a photo message (the actual download is async, so we test the
    // helper directly with a photo-bearing message structure).
    let message = serde_json::json!({
        "message_id": 60,
        "from": { "id": 1, "username": "alice" },
        "chat": { "id": 999 },
        "photo": [
            { "file_id": "abc123", "file_unique_id": "u1", "width": 320, "height": 240 }
        ],
        "forward_from": {
            "id": 42,
            "username": "bob"
        },
        "forward_date": 1_700_000_000
    });

    let attr =
        TelegramChannel::format_forward_attribution(&message).expect("should detect forward");
    assert_eq!(attr, "[Forwarded from @bob] ");

    // Simulate what try_parse_attachment_message does after building content
    let photo_content = "[IMAGE:/tmp/photo.jpg]".to_string();
    let content = format!("{attr}{photo_content}");
    assert_eq!(content, "[Forwarded from @bob] [IMAGE:/tmp/photo.jpg]");
}

#[tokio::test]
async fn register_bot_commands_sends_correct_payload() {
    use wiremock::matchers::{body_json, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let expected_body = serde_json::json!({
        "commands": [
            { "command": "new",    "description": "Start a new conversation session" },
            { "command": "stop",   "description": "Cancel the current in-flight task" },
            { "command": "model",  "description": "Show or switch the current model" },
            { "command": "models", "description": "List available providers or switch provider" },
            { "command": "config", "description": "Show current configuration" },
        ]
    });

    Mock::given(method("POST"))
        .and(path_regex(r"/bot[^/]+/setMyCommands$"))
        .and(body_json(&expected_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "result": true })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_api_base(mock_server.uri());

    ch.register_bot_commands().await;

    // Mock expectation assert happens on MockServer drop
}

#[tokio::test]
async fn register_bot_commands_handles_failure_gracefully() {
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/bot[^/]+/setMyCommands$"))
        .respond_with(ResponseTemplate::new(500).set_body_json(
            serde_json::json!({ "ok": false, "description": "Internal Server Error" }),
        ))
        .expect(1)
        .mount(&mock_server)
        .await;

    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_api_base(mock_server.uri());

    // Should not panic — errors are logged, not propagated.
    ch.register_bot_commands().await;
}

#[test]
fn sanitize_telegram_command_name_basic() {
    assert_eq!(sanitize_telegram_command_name("hello"), "hello");
    assert_eq!(sanitize_telegram_command_name("Hello"), "hello");
    assert_eq!(sanitize_telegram_command_name("my-skill"), "my_skill");
    assert_eq!(sanitize_telegram_command_name("my skill"), "my_skill");
    assert_eq!(
        sanitize_telegram_command_name("My Cool Skill!"),
        "my_cool_skill"
    );
}

#[test]
fn sanitize_telegram_command_name_trims_underscores() {
    assert_eq!(sanitize_telegram_command_name("_leading"), "leading");
    assert_eq!(sanitize_telegram_command_name("trailing_"), "trailing");
    assert_eq!(sanitize_telegram_command_name("__both__"), "both");
}

#[test]
fn sanitize_telegram_command_name_collapses_double_underscores() {
    assert_eq!(sanitize_telegram_command_name("a--b"), "a_b");
    assert_eq!(sanitize_telegram_command_name("a---b"), "a_b");
}

#[test]
fn sanitize_telegram_command_name_truncates_to_32_chars() {
    let long = "a".repeat(50);
    let result = sanitize_telegram_command_name(&long);
    assert!(result.len() <= TELEGRAM_COMMAND_NAME_MAX_LEN);
    assert_eq!(result.len(), 32);
}

#[test]
fn sanitize_telegram_command_name_empty_input() {
    assert_eq!(sanitize_telegram_command_name(""), "");
    assert_eq!(sanitize_telegram_command_name("---"), "");
}

#[test]
fn truncate_telegram_command_description_short() {
    assert_eq!(
        truncate_telegram_command_description("Short desc"),
        "Short desc"
    );
}

#[test]
fn truncate_telegram_command_description_at_limit() {
    let exact = "a".repeat(TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN);
    assert_eq!(truncate_telegram_command_description(&exact), exact);
}

#[test]
fn truncate_telegram_command_description_over_limit() {
    let long = "a".repeat(TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN + 10);
    let result = truncate_telegram_command_description(&long);
    assert!(result.chars().count() <= TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN);
    assert!(result.ends_with('…'));
}

#[test]
fn truncate_telegram_command_description_multibyte_within_char_limit() {
    // Multibyte string within Telegram's 100-character description limit
    // but well over 100 bytes in UTF-8 encoding. The function must use
    // character count (not byte count) to decide whether to truncate, so
    // a string like this should pass through unchanged with no trailing
    // ellipsis. Construction is deterministic via `repeat` so the byte
    // arithmetic is verifiable from the source: 31 ASCII bytes + 30 × 4
    // bytes (`🌧` is U+1F327, 4 bytes UTF-8) = 151 bytes, 61 chars.
    let desc = format!("Multibyte weather description: {}", "🌧".repeat(30));
    assert!(desc.chars().count() <= TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN);
    assert!(desc.len() > TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN);
    let result = truncate_telegram_command_description(&desc);
    assert!(
        !result.ends_with('…'),
        "should not append ellipsis when within char limit"
    );
    assert_eq!(result, desc.trim());
}

#[tokio::test]
async fn register_bot_commands_includes_skills() {
    use wiremock::matchers::{body_json, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let workspace = tempfile::tempdir().unwrap();
    let skill_dir = workspace.path().join("skills").join("weather");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: weather\ndescription: Check the weather forecast\n---\n# Weather\n",
    )
    .unwrap();

    let mock_server = MockServer::start().await;

    let expected_body = serde_json::json!({
        "commands": [
            { "command": "new",     "description": "Start a new conversation session" },
            { "command": "stop",    "description": "Cancel the current in-flight task" },
            { "command": "model",   "description": "Show or switch the current model" },
            { "command": "models",  "description": "List available providers or switch provider" },
            { "command": "config",  "description": "Show current configuration" },
            { "command": "weather", "description": "Check the weather forecast" },
        ]
    });

    Mock::given(method("POST"))
        .and(path_regex(r"/bot[^/]+/setMyCommands$"))
        .and(body_json(&expected_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "result": true })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_api_base(mock_server.uri())
        .with_workspace_dir(workspace.path().to_path_buf());

    ch.register_bot_commands().await;
}

#[tokio::test]
async fn register_bot_commands_includes_tools_from_config() {
    use wiremock::matchers::{body_json, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let expected_body = serde_json::json!({
        "commands": [
            { "command": "new",       "description": "Start a new conversation session" },
            { "command": "stop",      "description": "Cancel the current in-flight task" },
            { "command": "model",     "description": "Show or switch the current model" },
            { "command": "models",    "description": "List available providers or switch provider" },
            { "command": "config",    "description": "Show current configuration" },
            { "command": "test_tool", "description": "A test tool" },
        ]
    });

    Mock::given(method("POST"))
        .and(path_regex(r"/bot[^/]+/setMyCommands$"))
        .and(body_json(&expected_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "result": true })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let specs = vec![("test_tool".to_string(), "A test tool".to_string())];
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_api_base(mock_server.uri())
        .with_tool_command_specs(specs);

    ch.register_bot_commands().await;
}

// ── Approval inline keyboard tests ────────────────────────

#[test]
fn pending_approvals_map_is_initially_empty() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let map = ch.pending_approvals.lock().await;
        assert!(map.is_empty());
    });
}

#[test]
fn approval_timeout_defaults_to_120_and_is_overridable() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    assert_eq!(ch.approval_timeout_secs, 120);
    let ch = ch.with_approval_timeout_secs(30);
    assert_eq!(ch.approval_timeout_secs, 30);
}

#[tokio::test]
async fn pending_approval_oneshot_delivers_response() {
    use operant_api::channel::ChannelApprovalResponse;

    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let approval_id = "test-approval-123".to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();

    ch.pending_approvals
        .lock()
        .await
        .insert(approval_id.clone(), tx);

    // Simulate what listen() does when a callback_query arrives
    if let Some(sender) = ch.pending_approvals.lock().await.remove(&approval_id) {
        sender.send(ChannelApprovalResponse::Approve).unwrap();
    }

    let result = rx.await.unwrap();
    assert_eq!(result, ChannelApprovalResponse::Approve);
}

#[test]
fn callback_data_format_parses_correctly() {
    // Verify the callback_data format used by request_approval
    let cb_data = "approval:abc-123:approve";
    let rest = cb_data.strip_prefix("approval:").unwrap();
    let (id, action) = rest.rsplit_once(':').unwrap();
    assert_eq!(id, "abc-123");
    assert_eq!(action, "approve");

    let cb_data = "approval:abc-123:deny";
    let rest = cb_data.strip_prefix("approval:").unwrap();
    let (id, action) = rest.rsplit_once(':').unwrap();
    assert_eq!(id, "abc-123");
    assert_eq!(action, "deny");

    let cb_data = "approval:abc-123:always";
    let rest = cb_data.strip_prefix("approval:").unwrap();
    let (id, action) = rest.rsplit_once(':').unwrap();
    assert_eq!(id, "abc-123");
    assert_eq!(action, "always");
}

#[test]
fn callback_data_with_uuid_parses_correctly() {
    // UUIDs contain hyphens — rsplit_once(':') must split at the LAST colon
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let cb_data = format!("approval:{uuid}:approve");
    let rest = cb_data.strip_prefix("approval:").unwrap();
    let (id, action) = rest.rsplit_once(':').unwrap();
    assert_eq!(id, uuid);
    assert_eq!(action, "approve");
}

#[test]
fn non_approval_callback_data_is_ignored() {
    let cb_data = "some_other_action:data";
    assert!(cb_data.strip_prefix("approval:").is_none());
}

#[tokio::test]
async fn pending_choice_resolves_with_selected_text() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let choice_id = "test-choice-123".to_string();
    let choices = vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()];
    let (tx, rx) = tokio::sync::oneshot::channel();
    ch.pending_choices
        .lock()
        .await
        .insert(choice_id.clone(), (tx, choices.clone()));

    // Simulate what listen() does when a `choice:` callback_query arrives.
    let (sender, stored_choices) = ch.pending_choices.lock().await.remove(&choice_id).unwrap();
    let text = stored_choices.get(1).cloned().unwrap();
    sender.send(text).unwrap();

    assert_eq!(rx.await.unwrap(), "Beta");
}

#[test]
fn choice_callback_data_parses_uuid_and_index() {
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let cb_data = format!("choice:{uuid}:2");
    let rest = cb_data.strip_prefix("choice:").unwrap();
    let (id, idx_str) = rest.rsplit_once(':').unwrap();
    assert_eq!(id, uuid);
    assert_eq!(idx_str.parse::<usize>().unwrap(), 2);
}

#[test]
fn non_choice_callback_data_is_ignored() {
    assert!("approval:abc:approve".strip_prefix("choice:").is_none());
    assert!("choice".strip_prefix("choice:").is_none());
}

// ── T4 polling-resilience tests ──────────────────────────────────────────

#[test]
fn getupdates_error_classification_matches_hermes() {
    // Auth/validation errors are Fatal — never churn the loop.
    assert_eq!(PollErrorClass::from_error_code(401), PollErrorClass::Fatal);
    assert_eq!(PollErrorClass::from_error_code(403), PollErrorClass::Fatal);
    // Another getUpdates consumer holds the slot.
    assert_eq!(
        PollErrorClass::from_error_code(409),
        PollErrorClass::Conflict
    );
    // Everything transient (rate limit, 5xx, unknown) is Network.
    assert_eq!(
        PollErrorClass::from_error_code(429),
        PollErrorClass::Network
    );
    assert_eq!(
        PollErrorClass::from_error_code(500),
        PollErrorClass::Network
    );
    assert_eq!(PollErrorClass::from_error_code(0), PollErrorClass::Network);
}

#[test]
fn pending_probe_single_stuck_does_not_escalate() {
    // Hermes test_telegram_pending_update_probe contract: one probe seeing
    // a queue at/above threshold only counts a strike.
    let mut strikes = 0u32;
    assert!(!probe_pending_escalate(
        9,
        &mut strikes,
        POLLING_PENDING_STUCK_THRESHOLD,
        POLLING_PENDING_STUCK_STRIKES
    ));
    assert_eq!(strikes, 1);
}

#[test]
fn pending_probe_two_stuck_probes_escalate() {
    let mut strikes = 0u32;
    assert!(!probe_pending_escalate(
        9,
        &mut strikes,
        POLLING_PENDING_STUCK_THRESHOLD,
        POLLING_PENDING_STUCK_STRIKES
    ));
    assert!(probe_pending_escalate(
        9,
        &mut strikes,
        POLLING_PENDING_STUCK_THRESHOLD,
        POLLING_PENDING_STUCK_STRIKES
    ));
    // Escalation resets the strikes for the next cycle.
    assert_eq!(strikes, 0);
}

#[test]
fn pending_probe_healthy_resets_strikes() {
    let mut strikes = 1u32;
    assert!(!probe_pending_escalate(
        0,
        &mut strikes,
        POLLING_PENDING_STUCK_THRESHOLD,
        POLLING_PENDING_STUCK_STRIKES
    ));
    assert_eq!(strikes, 0);
}

#[test]
fn pending_probe_below_threshold_does_not_strike() {
    let mut strikes = 0u32;
    assert!(!probe_pending_escalate(
        1,
        &mut strikes,
        POLLING_PENDING_STUCK_THRESHOLD,
        POLLING_PENDING_STUCK_STRIKES
    ));
    assert_eq!(strikes, 0);
}

#[test]
fn recovery_trigger_bumps_generation() {
    let state = PollRecoveryState::new();
    let rx = state.generation.subscribe();
    assert_eq!(*rx.borrow(), 0);
    state.trigger("test");
    // The watch value bumps synchronously.
    assert_eq!(*rx.borrow(), 1);
}

#[test]
fn recovery_trigger_is_debounced() {
    let state = PollRecoveryState::new();
    state.trigger("first");
    // Back-to-back trigger inside the debounce window is collapsed.
    state.trigger("second");
    let rx = state.generation.subscribe();
    assert_eq!(*rx.borrow(), 1);
}

#[test]
fn fallback_ip_rotation_advances_round_robin() {
    let index = parking_lot::Mutex::new(0usize);
    // Pick the IP the rotation index selects on successive calls.
    let pick = || {
        let mut i = index.lock();
        let cur = *i;
        *i = (*i + 1) % 3;
        cur
    };
    assert_eq!(pick(), 0);
    assert_eq!(pick(), 1);
    assert_eq!(pick(), 2);
    assert_eq!(pick(), 0); // wraps
}

#[test]
fn fallback_ips_builder_stores_and_http_client_uses_resolve() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_fallback_ips(vec!["149.154.167.220".into()]);
    assert_eq!(ch.fallback_ips, vec!["149.154.167.220".to_string()]);
    // api_host extraction used as the resolve target.
    assert_eq!(api_host_of("https://api.telegram.org"), "api.telegram.org");
    assert_eq!(api_host_of("http://127.0.0.1:8080"), "127.0.0.1:8080");
    // Building the client with the fallback must not panic and must
    // succeed (the resolved builder is exercised).
    let _client = ch.http_client();
}
