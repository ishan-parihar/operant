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

            // Read greeting
            let mut buf = [0u8; 4096];
            let _ = read_response(&mut stream, &mut buf)?;

            // EHLO
            write!(stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            let ehlo_resp = read_response(&mut stream, &mut buf)?;

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
            use std::sync::OnceLock;
            static TLS_CONFIG: OnceLock<std::sync::Arc<rustls::ClientConfig>> = OnceLock::new();
            let config = TLS_CONFIG.get_or_init(|| {
                let mut roots = rustls::RootCertStore::empty();
                // rustls-native-certs v0.7 returns its own cert iterator;
                // we add each to the rustls RootCertStore manually.
                for cert in rustls_native_certs::load_native_certs()
                    .unwrap_or_default()
                    .into_iter()
                {
                    let _ = roots.add(cert);
                }
                std::sync::Arc::new(
                    rustls::ClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_no_client_auth(),
                )
            });
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
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // AUTH LOGIN (base64-encoded user/pass — now encrypted in transit)
            use base64::Engine;
            write!(tls_stream, "AUTH LOGIN\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&user))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&pass))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // MAIL FROM
            write!(tls_stream, "MAIL FROM:<{}>\r\n", user).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // RCPT TO
            write!(tls_stream, "RCPT TO:<{}>\r\n", to).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // DATA
            write!(tls_stream, "DATA\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // Email body
            write!(
                tls_stream,
                "From: <{}>\r\nTo: <{}>\r\nSubject: Operant Reply\r\n\r\n{}\r\n.\r\n",
                user, to, body
            )
            .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

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
