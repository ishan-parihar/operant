//! `email` — extracted verbatim from gateway/mod.rs.

use crate::error::{Error, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::info;

use super::*;

// ---------------------------------------------------------------------------
// Email adapter (SMTP outbound, webhook inbound)
// ---------------------------------------------------------------------------

/// Email adapter — sends replies via SMTP, receives via webhook.
/// Requires `email_smtp_host`, `email_smtp_user`, `email_smtp_pass` in config.
pub struct EmailAdapter {
    enabled: bool,
    smtp_host: Option<String>,
    smtp_user: Option<String>,
    smtp_pass: Option<String>,
}

impl EmailAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            smtp_host: None,
            smtp_user: None,
            smtp_pass: None,
        }
    }

    pub fn with_smtp(
        mut self,
        host: Option<String>,
        user: Option<String>,
        pass: Option<String>,
    ) -> Self {
        self.smtp_host = host;
        self.smtp_user = user;
        self.smtp_pass = pass;
        self
    }
}

#[async_trait]
impl PlatformAdapter for EmailAdapter {
    fn name(&self) -> &str {
        "email"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!("Email adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Email adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        // Email sending uses SMTP which is blocking — spawn_blocking.
        let host = self
            .smtp_host
            .clone()
            .ok_or_else(|| Error::Config("SMTP host not configured".to_string()))?;
        let user = self.smtp_user.clone().unwrap_or_default();
        let pass = self.smtp_pass.clone().unwrap_or_default();
        let to = message.channel_id.clone();
        let body = message.content.clone();

        tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            // SMTP send with mandatory STARTTLS upgrade. Previously this
            // ran AUTH LOGIN over plaintext TCP, leaking the SMTP password
            // to anyone on the network path. (iter-125 — closes the
            // ponytail-audit security bug "EmailAdapter SMTP AUTH over
            // plaintext TCP".)
            //
            // NOTE: This is a minimal STARTTLS implementation. Production
            // use should switch to the `lettre` crate for proper MIME,
            // attachments, certificate validation, and connection pooling.
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let port = if host.contains(":") { "" } else { ":587" };
            let addr = format!("{}{}", host, port);

            let mut stream = TcpStream::connect(&addr)
                .map_err(|e| format!("SMTP connect failed: {e}"))?;
            // Set a 30s timeout so a hung SMTP server doesn't block the
            // gateway forever.
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));

            // Helper: read a multi-line SMTP response.
            fn read_response(stream: &mut TcpStream, buf: &mut [u8]) -> std::result::Result<String, String> {
                let n = stream.read(buf).map_err(|e| format!("SMTP read: {e}"))?;
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                // SMTP multi-line responses end with "<code> <text>" (space
                // after the code, not dash). We need to keep reading until
                // we see that.
                let mut full = text.clone();
                while let Some(line) = full.lines().last() {
                    if line.len() >= 4 && line.as_bytes()[3] == b' ' {
                        break;
                    }
                    let n2 = stream.read(buf).map_err(|e| format!("SMTP read: {e}"))?;
                    full.push_str(&String::from_utf8_lossy(&buf[..n2]));
                }
                Ok(full)
            }

            // Read greeting — the server must accept the connection.
            let mut buf = [0u8; 4096];
            let greeting = read_response(&mut stream, &mut buf)?;
            check_smtp_code(&greeting, &[220], "greeting")?;

            // EHLO
            write!(stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            let ehlo_resp = read_response(&mut stream, &mut buf)?;
            check_smtp_code(&ehlo_resp, &[250], "EHLO")?;

            // STARTTLS — refuse to proceed without TLS upgrade.
            write!(stream, "STARTTLS\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            let starttls_resp = read_response(&mut stream, &mut buf)?;
            if !starttls_resp.starts_with("220") {
                return Err(format!(
                    "SMTP server does not support STARTTLS (got: {}). Refusing to send credentials over plaintext. Configure an SMTP host that supports STARTTLS, or set up a TLS tunnel.",
                    starttls_resp.lines().next().unwrap_or("").trim()
                ));
            }

            // Upgrade to TLS. Use a minimal TLS connector with the system
            // root store + hostname verification (rustls).
            //
            // The cached value is the RESULT, not the config: building the
            // config can fail (the protocol versions are resolved against
            // the named provider), and a `LazyLock` initializer cannot
            // propagate an error out of itself.
            static TLS_CONFIG: std::sync::LazyLock<
                std::result::Result<std::sync::Arc<rustls::ClientConfig>, String>,
            > = std::sync::LazyLock::new(|| {
                let mut roots = rustls::RootCertStore::empty();
                // rustls-native-certs 0.8 returns the certs it loaded next
                // to the errors it hit; a bad store entry is skipped, not
                // fatal.
                let loaded = rustls_native_certs::load_native_certs();
                if !loaded.errors.is_empty() {
                    tracing::warn!(
                        count = loaded.errors.len(),
                        "some native root certificates could not be loaded"
                    );
                }
                for cert in loaded.certs {
                    let _ = roots.add(cert);
                }
                // The provider is named EXPLICITLY: `builder()` resolves it
                // from the crate features and PANICS when they are
                // ambiguous, and they are here (hyper-rustls turns on
                // `aws-lc-rs` by default, tokio-rustls turns on `ring`).
                operant_config::schema::client_config(roots)
                    .map(std::sync::Arc::new)
                    .map_err(|e| format!("rustls client config: {e}"))
            });
            let config = TLS_CONFIG
                .clone()
                .map_err(|e| format!("SMTP TLS setup: {e}"))?;
            let server_name = host
                .split(':')
                .next()
                .unwrap_or(&host)
                .to_string();
            let server_name = rustls::pki_types::ServerName::try_from(server_name)
                .map_err(|e| format!("invalid SMTP hostname for TLS: {e}"))?
                .to_owned();
            let mut connector = rustls::client::ClientConnection::new(
                config.clone(),
                server_name,
            ).map_err(|e| format!("TLS init: {e}"))?;
            let mut tls_stream = rustls::Stream::new(&mut connector, &mut stream);

            // EHLO again over the encrypted channel.
            write!(tls_stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[250], "EHLO (TLS)")?;

            // AUTH LOGIN (base64-encoded user/pass — now encrypted in transit)
            use base64::Engine;
            write!(tls_stream, "AUTH LOGIN\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[334], "AUTH LOGIN")?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&user))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[334], "AUTH username")?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&pass))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            // 235 is "authentication succeeded"; 535 is "authentication
            // failed" and must NOT be reported as a successful send.
            check_smtp_code(&resp, &[235], "AUTH password")?;

            // MAIL FROM
            write!(tls_stream, "MAIL FROM:<{}>\r\n", user).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[250], "MAIL FROM")?;

            // RCPT TO
            write!(tls_stream, "RCPT TO:<{}>\r\n", to).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[250, 251], "RCPT TO")?;

            // DATA
            write!(tls_stream, "DATA\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            check_smtp_code(&resp, &[354], "DATA")?;

            // Email body
            write!(
                tls_stream,
                "From: <{}>\r\nTo: <{}>\r\nSubject: Operant Reply\r\n\r\n{}\r\n.\r\n",
                user, to, body
            )
            .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            let resp = read_tls_response(&mut tls_stream, &mut buf)?;
            // 250 is "message accepted"; 552 is "message too large".
            check_smtp_code(&resp, &[250], "message body")?;

            // QUIT
            write!(tls_stream, "QUIT\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;

            let _ = ehlo_resp; // suppress unused warning
            Ok(())
        })
        .await
        .map_err(|e| Error::Agent(format!("SMTP task failed: {e}")))?
        .map_err(|e| Error::Agent(format!("SMTP send failed: {e}")))?;

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse email webhook payload (format depends on the email forwarding service)
        let from = update["from"]
            .as_str()
            .or_else(|| update["sender"].as_str())
            .unwrap_or("");
        let subject = update["subject"].as_str().unwrap_or("(no subject)");
        let body = update["body"]
            .as_str()
            .or_else(|| update["text"].as_str())
            .unwrap_or("");

        if from.is_empty() && body.is_empty() {
            return Ok(None);
        }

        Ok(Some(IncomingMessage {
            platform: "email".to_string(),
            channel_id: from.to_string(),
            user_id: from.to_string(),
            username: from.to_string(),
            content: format!("Subject: {}\n\n{}", subject, body),
            is_group_chat: false,
            is_channel_post: false,
            timestamp: chrono::Utc::now().timestamp(),
            thread_id: None,
            media_urls: Vec::new(),

            raw: update,
        }))
    }

    fn config_json(&self) -> serde_json::Value {
        json!({
            "platform": "email",
            "enabled": self.enabled,
            "smtp_host": self.smtp_host,
            "smtp_user_configured": self.smtp_user.is_some(),
            // The adapter can SEND but not RECEIVE: nothing in the
            // runtime invokes this adapter's `handle_update`, so an
            // inbound message never reaches the agent. The status report
            // used to say "enabled" and nothing more, so an operator
            // believed they could receive email messages. (RT-86.)
            "inbound": false,
            "outbound": true,
        })
    }

    async fn start_with_channel(
        &self,
        _message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        self.start().await
    }

    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let mut msg = OutgoingMessage::new(channel_id.to_string(), message.content.clone());
        msg.parse_markdown = message.parse_markdown;
        self.send_message(msg).await?;
        Ok(String::new())
    }
}

/// Check that an SMTP reply carries one of the expected status codes.
///
/// The adapter used to read every server reply into a buffer and DISCARD
/// it, so a `535 Authentication failed`, a `550 mailbox unavailable` and a
/// `552 message too large` all produced `Ok(())` — the send reported
/// SUCCESS for a message the server had rejected. The operator saw the
/// reply come back and had no idea the mail was never sent.
///
/// The code is the first three characters of the reply. A multi-line
/// reply's LAST line carries the final code, and that is the one that
/// matters — the earlier lines are continuations.
fn check_smtp_code(reply: &str, expected: &[u16], step: &str) -> std::result::Result<(), String> {
    let last = reply.lines().last().unwrap_or("").trim();
    let code: u16 = last
        .get(..3)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("SMTP {step}: unparseable server reply: {last:?}"))?;
    if expected.contains(&code) {
        return Ok(());
    }
    Err(format!(
        "SMTP {step}: server replied {code} ({last:?}); expected one of {expected:?}. \
         The message was NOT sent."
    ))
}

/// Read a multi-line SMTP reply over a TLS stream.
///
/// The plain-stream path has `read_response`; the TLS one needs the same
/// "keep reading until the final line" loop, over `rustls::Stream`.
fn read_tls_response<S>(stream: &mut S, buf: &mut [u8]) -> std::result::Result<String, String>
where
    S: std::io::Read,
{
    let n = stream
        .read(buf)
        .map_err(|e| format!("SMTP read (TLS): {e}"))?;
    let mut full = String::from_utf8_lossy(&buf[..n]).to_string();
    // A multi-line reply ends with "<code> <text>" — a SPACE after the
    // code, not a dash.
    while let Some(line) = full.lines().last() {
        if line.len() >= 4 && line.as_bytes()[3] == b' ' {
            break;
        }
        let n2 = stream
            .read(buf)
            .map_err(|e| format!("SMTP read (TLS): {e}"))?;
        if n2 == 0 {
            break;
        }
        full.push_str(&String::from_utf8_lossy(&buf[..n2]));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::check_smtp_code;

    #[test]
    fn an_accepted_reply_passes() {
        assert!(check_smtp_code("250 2.1.0 Ok\r\n", &[250], "EHLO").is_ok());
        assert!(check_smtp_code("235 2.7.0 Authentication successful\r\n", &[235], "AUTH").is_ok());
    }

    #[test]
    fn a_rejected_reply_fails() {
        // The defect: these all used to be reported as a successful send.
        let err = check_smtp_code(
            "535 5.7.8 Authentication credentials invalid\r\n",
            &[235],
            "AUTH",
        )
        .expect_err("a failed authentication must not pass");
        assert!(err.contains("535"), "the error must name the code: {err}");
        assert!(
            err.contains("NOT sent"),
            "the error must say it was not sent"
        );

        assert!(check_smtp_code("550 5.1.1 mailbox unavailable\r\n", &[250], "RCPT TO").is_err());
        assert!(
            check_smtp_code("552 5.3.4 message too large\r\n", &[250], "message body").is_err()
        );
    }

    #[test]
    fn the_last_line_carries_the_decisive_code() {
        // A multi-line reply's continuations are not the verdict.
        let reply = "250-first line\r\n250 2.0.0 Ok\r\n";
        assert!(check_smtp_code(reply, &[250], "EHLO").is_ok());
        let reply = "250-first line\r\n535 5.7.8 bad credentials\r\n";
        assert!(check_smtp_code(reply, &[250], "EHLO").is_err());
    }

    #[test]
    fn an_unparseable_reply_fails() {
        assert!(check_smtp_code("garbage", &[250], "EHLO").is_err());
        assert!(check_smtp_code("", &[250], "EHLO").is_err());
    }
}
