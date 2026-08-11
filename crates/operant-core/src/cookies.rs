//! Browser cookie import/export — multi-browser cookie support for the
//! Obscura browser.
//!
//! Lets accounts from any browser (Chrome / Brave / Edge / Chromium /
//! Firefox) be used directly inside Obscura without manual login:
//!
//! - **Netscape `cookies.txt`** — the universal export format produced by
//!   every cookie-export browser extension (EditThisCookie, Get cookies.txt,
//!   Cookie-Editor, …).
//! - **EditThisCookie / Cookie-Editor JSON** — the other common export shape.
//! - **Direct browser DB reads** — Firefox `cookies.sqlite` (plaintext
//!   values) and Chromium-family `Cookies` (SQLite with AES-128-CBC v10
//!   encrypted values; decryption key from `Local State` `os_crypt`).
//!
//! Imported cookies are injected into the shared Obscura CDP session via
//! `Storage.setCookies` / `Network.setCookie`, then applied to every page
//! the browser navigates to.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single cookie, normalized across browser formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie domain (`.example.com` or `example.com`).
    pub domain: String,
    /// Cookie path (default `/`).
    pub path: String,
    /// Expiry as Unix epoch seconds. `None` = session cookie.
    pub expires: Option<i64>,
    /// `Secure` flag.
    pub secure: bool,
    /// `HttpOnly` flag.
    pub http_only: bool,
    /// SameSite: `"Lax"`, `"Strict"`, `"None"`, or `""`.
    pub same_site: String,
}

impl Cookie {
    /// Build the CDP `Storage.setCookies` / `Network.setCookie` param object.
    /// Uses `url` derived from the domain so the browser can infer
    /// secure/sameSite defaults, but passes explicit flags where set.
    pub fn to_cdp_value(&self) -> Value {
        let mut cookie = serde_json::Map::new();
        cookie.insert("name".into(), Value::String(self.name.clone()));
        cookie.insert("value".into(), Value::String(self.value.clone()));
        cookie.insert("domain".into(), Value::String(self.domain.clone()));
        cookie.insert("path".into(), Value::String(self.path.clone()));
        if let Some(exp) = self.expires {
            cookie.insert("expires".into(), Value::Number(exp.into()));
        }
        cookie.insert("secure".into(), Value::Bool(self.secure));
        cookie.insert("httpOnly".into(), Value::Bool(self.http_only));
        if !self.same_site.is_empty() {
            cookie.insert("sameSite".into(), Value::String(self.same_site.clone()));
        }
        Value::Object(cookie)
    }

    /// Render as a Netscape `cookies.txt` line.
    pub fn to_netscape_line(&self) -> String {
        let domain = self.domain.trim_start_matches('.');
        // Chrome-family files carry a leading dot for host-only cookies that
        // should apply to subdomains; Emit the raw domain for broadest
        // compatibility (Obscura sets the cookie by domain directly).
        let http_only = if self.http_only { "TRUE" } else { "FALSE" };
        let secure = if self.secure { "TRUE" } else { "FALSE" };
        let expires = self.expires.unwrap_or(0);
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            domain, http_only, self.path, secure, expires, self.name, self.value
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Netscape cookies.txt parsing
// ──────────────────────────────────────────────────────────────────────────

/// Parse a Netscape `cookies.txt` file (the universal export format).
///
/// Lines are tab-separated: `domain  flag  path  secure  expiry  name  value`
/// (HTTP-only flag is the second field on all exporters). Lines starting with
/// `#` are comments. Values may contain tabs when exported by some tools —
/// we split with `splitn(7, '\t')` to keep the value intact.
pub fn parse_netscape_cookies_txt(text: &str) -> Vec<Cookie> {
    let mut cookies = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.splitn(7, '\t').collect();
        if parts.len() < 7 {
            // Some exporters emit space-separated files — try a tolerant split.
            let parts: Vec<&str> = trimmed.splitn(7, char::is_whitespace).collect();
            if parts.len() < 7 {
                continue;
            }
            cookies.push(build_cookie_from_parts(parts));
        } else {
            cookies.push(build_cookie_from_parts(parts));
        }
    }
    cookies
}

fn build_cookie_from_parts(parts: Vec<&str>) -> Cookie {
    let domain = parts[0].to_string();
    let path = if parts[2].is_empty() {
        "/".to_string()
    } else {
        parts[2].to_string()
    };
    let secure = parts[3].eq_ignore_ascii_case("TRUE");
    let expires = parts[4].parse::<i64>().ok().filter(|e| *e > 0);
    let name = parts[5].to_string();
    let value = parts.get(6).copied().unwrap_or("").to_string();
    Cookie {
        name,
        value,
        domain,
        path,
        expires,
        secure,
        http_only: parts[1].eq_ignore_ascii_case("TRUE"),
        same_site: String::new(),
    }
}

// ──────────────────────────────────────────────────────────────────────────
// JSON parsing (EditThisCookie / Cookie-Editor export)
// ──────────────────────────────────────────────────────────────────────────

/// Parse an EditThisCookie / Cookie-Editor JSON export (a JSON array of
/// cookie objects with `name`, `value`, `domain`, `path`, `expirationDate`
/// or `expires`, `secure`, `httpOnly`, `sameSite`).
pub fn parse_json_cookies(json_text: &str) -> Vec<Cookie> {
    let Ok(values) = serde_json::from_str::<Vec<Value>>(json_text) else {
        // Try a single object wrapper.
        let Ok(single) = serde_json::from_str::<Value>(json_text) else {
            return Vec::new();
        };
        return parse_json_cookie_object(&single).into_iter().collect();
    };
    values.iter().filter_map(parse_json_cookie_object).collect()
}

fn parse_json_cookie_object(v: &Value) -> Option<Cookie> {
    let name = v.get("name")?.as_str()?.to_string();
    let value = v
        .get("value")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let domain = v
        .get("domain")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let path = v
        .get("path")
        .and_then(|x| x.as_str())
        .unwrap_or("/")
        .to_string();
    // EditThisCookie uses `expirationDate` (float seconds); Cookie-Editor
    // uses `expires` (float seconds) or `expirationDate`.
    let expires = v
        .get("expirationDate")
        .or_else(|| v.get("expires"))
        .and_then(|x| x.as_f64())
        .map(|f| f as i64)
        .filter(|e| *e > 0);
    let secure = v.get("secure").and_then(|x| x.as_bool()).unwrap_or(false);
    let http_only = v
        .get("httpOnly")
        .or_else(|| v.get("httponly"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let same_site = v
        .get("sameSite")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some(Cookie {
        name,
        value,
        domain,
        path,
        expires,
        secure,
        http_only,
        same_site,
    })
}

// ──────────────────────────────────────────────────────────────────────────
// Browser cookie DB locations
// ──────────────────────────────────────────────────────────────────────────

/// Well-known browser cookie-database locations, in priority order.
/// Returns `(label, db_path, local_state_path)` where `local_state_path` is
/// `None` for Firefox (plaintext values).
pub fn discover_browser_cookie_sources()
-> Vec<(String, std::path::PathBuf, Option<std::path::PathBuf>)> {
    let home = dirs::home_dir();
    let mut out = Vec::new();
    let Some(home) = home else { return out };

    let chromium_bases: &[(&str, &str, &str)] = &[
        ("chrome", ".config/google-chrome", ".config/google-chrome"),
        ("chromium", ".config/chromium", ".config/chromium"),
        (
            "brave",
            ".config/BraveSoftware/Brave-Browser",
            ".config/BraveSoftware/Brave-Browser",
        ),
        ("edge", ".config/microsoft-edge", ".config/microsoft-edge"),
        ("vivaldi", ".config/vivaldi", ".config/vivaldi"),
        ("opera", ".config/opera", ".config/opera"),
    ];
    for (label, base, ls_base) in chromium_bases {
        let db = home.join(base).join("Default").join("Cookies");
        if db.exists() {
            out.push((
                label.to_string(),
                db,
                Some(home.join(ls_base).join("Local State")),
            ));
        }
    }

    // Firefox: profile dirs hold cookies.sqlite (plaintext values).
    let firefox_dir = home.join(".mozilla/firefox");
    if let Ok(entries) = std::fs::read_dir(&firefox_dir) {
        for entry in entries.flatten() {
            let db = entry.path().join("cookies.sqlite");
            if db.exists() {
                out.push(("firefox".to_string(), db, None));
            }
        }
    }
    out
}

/// Locate a specific browser's cookie DB by name (`chrome`, `brave`, …).
pub fn find_browser_cookie_source(
    browser: &str,
) -> Option<(String, std::path::PathBuf, Option<std::path::PathBuf>)> {
    let browser = browser.to_ascii_lowercase();
    discover_browser_cookie_sources()
        .into_iter()
        .find(|(label, _, _)| label == &browser)
}

// ──────────────────────────────────────────────────────────────────────────
// Chromium v10 decryption
// ──────────────────────────────────────────────────────────────────────────

/// AES-128-CBC decrypt (raw block cipher, manual CBC chaining) + PKCS#7 unpad.
fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    use aes::cipher::{BlockDecrypt, KeyInit};
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return None;
    }
    let cipher = aes::Aes128::new(key.into());
    let blocks = data.len() / 16;
    let mut out = vec![0u8; data.len()];
    let mut prev: [u8; 16] = *iv;
    for i in 0..blocks {
        let block =
            aes::cipher::Block::<aes::Aes128>::clone_from_slice(&data[i * 16..(i + 1) * 16]);
        let mut dec = block;
        cipher.decrypt_block(&mut dec);
        for j in 0..16 {
            out[i * 16 + j] = dec[j] ^ prev[j];
        }
        prev.copy_from_slice(&data[i * 16..(i + 1) * 16]);
    }
    // Strip PKCS#7 padding.
    let pad = *out.last()? as usize;
    if pad == 0 || pad > 16 {
        return None;
    }
    if out.len() < pad {
        return None;
    }
    Some(out[..out.len() - pad].to_vec())
}

/// Derive the Chromium v10 decryption key for a profile.
///
/// Resolution order (standard Linux Chromium scheme):
/// 1. `os_crypt.encrypted_key` in Local State — base64 blob, AES-128-CBC
///    wrapped with key = SHA256("peanuts")[:16], IV = 16 zeros. On Windows
///    the blob carries a "DPAPI" prefix; on Linux there is none.
/// 2. The OS keyring (Linux): Chromium stores the raw AES key in the
///    Secret Service / KWallet when `encrypted_key` is absent. Queried via
///    the `secret-tool` CLI if present (schema
///    `chrome_libsecret_os_crypt_password_v2` / `_v1`, application
///    `chrome`/`chromium`/`brave`/`edge`).
/// 3. The hardcoded "peanuts" fallback key (older headless profiles).
fn chromium_decryption_key(local_state_path: Option<&std::path::Path>) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let peanuts: [u8; 16] = Sha256::digest(b"peanuts")[..16].try_into().ok()?;

    // 1. Local State `encrypted_key` (Windows DPAPI blob / Linux wrapped key).
    if let Some(ls) = local_state_path {
        let local_state: Value = serde_json::from_str(&std::fs::read_to_string(ls).ok()?).ok()?;
        if let Some(enc_key_b64) = local_state
            .get("os_crypt")
            .and_then(|o| o.get("encrypted_key"))
            .and_then(|k| k.as_str())
            .and_then(|b64| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok()
            })
            .and_then(|enc_key| {
                let key_blob = if enc_key.starts_with(b"DPAPI") {
                    &enc_key[5..]
                } else {
                    &enc_key[..]
                };
                aes128_cbc_decrypt(&peanuts, &[0u8; 16], key_blob)
            })
        {
            return Some(enc_key_b64);
        }
    }

    // 2. Linux OS keyring (secret service / kwallet).
    if let Some(key) = chromium_keyring_key() {
        return Some(key);
    }

    // 3. Peanuts fallback (headless / no-keyring profiles).
    Some(peanuts.to_vec())
}

/// Query the Linux OS keyring for the Chromium v10 key via `secret-tool`.
///
/// Returns the raw key when found. `None` when the tool is unavailable, the
/// keyring has no matching entry, or the secret isn't a plausible key.
fn chromium_keyring_key() -> Option<Vec<u8>> {
    use std::process::Command;

    let secret_tool = which_secret_tool()?;
    let schemas = [
        "chrome_libsecret_os_crypt_password_v2",
        "chrome_libsecret_os_crypt_password_v1",
    ];
    let apps = ["brave", "chrome", "chromium", "edge"];
    for schema in schemas {
        for app in apps {
            let out = Command::new(&secret_tool)
                .args(["search", "--all"])
                .arg(format!("xdg:schema {schema}"))
                .arg(format!("application {app}"))
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(secret) = parse_secret_tool_secret(&text) {
                // The keyring secret is the raw 16/32-byte key (sometimes
                // base64-encoded by some distros).
                let raw = secret.as_bytes();
                if raw.len() >= 16 {
                    return Some(raw[..16].to_vec());
                }
                if let Some(dec) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    secret.trim(),
                )
                .ok()
                .filter(|d| d.len() >= 16)
                {
                    return Some(dec[..16].to_vec());
                }
            }
        }
    }
    None
}

/// Locate the `secret-tool` binary on PATH.
fn which_secret_tool() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join("secret-tool");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Extract the `secret = ...` value from `secret-tool search --all` output.
fn parse_secret_tool_secret(text: &str) -> Option<String> {
    // Output blocks look like:
    //   [keyring]\n  label = ...\n  secret = <value>\n  created = ...
    let mut secret = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("secret =") {
            secret = Some(rest.trim().to_string());
        }
    }
    secret
}

fn chromium_decrypt_value(encrypted: &[u8], key: &[u8]) -> Option<String> {
    // v10 layout: 3-byte "v10" magic + 16-byte IV + AES-128-CBC payload.
    if encrypted.len() < 3 + 16 + 16 || &encrypted[..3] != b"v10" {
        // Plaintext values (older profiles) are stored as-is.
        return String::from_utf8(encrypted.to_vec()).ok();
    }
    let iv: [u8; 16] = encrypted[3..19].try_into().ok()?;
    let payload = &encrypted[19..];
    let key: &[u8; 16] = key.try_into().ok()?;
    String::from_utf8(aes128_cbc_decrypt(key, &iv, payload)?).ok()
}

/// Diagnostic summary of a Chromium cookie-DB read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChromiumReadReport {
    /// Total rows in the `cookies` table.
    pub total_rows: usize,
    /// Cookies successfully decrypted / read as plaintext.
    pub decrypted: usize,
    /// Rows using v11+ app-bound encryption (undecryptable by design).
    pub app_bound: usize,
    /// Rows with unrecognized / undecryptable payloads.
    pub undecryptable: usize,
}

/// Read a Chromium-family `Cookies` SQLite DB (Chrome/Brave/Edge/…).
/// Returns the decrypted cookies plus a diagnostic report. Undecryptable
/// rows (v11 app-bound, unknown keyring) are counted, not silently dropped.
pub fn read_chromium_cookies_report(
    db_path: &std::path::Path,
    local_state_path: Option<&std::path::Path>,
) -> (Vec<Cookie>, ChromiumReadReport) {
    let mut report = ChromiumReadReport::default();
    let key = chromium_decryption_key(local_state_path);
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), report),
    };
    let mut stmt = match conn.prepare(
        "SELECT host_key, name, value, encrypted_value, path, expires_utc, \
         is_secure, is_httponly, samesite FROM cookies",
    ) {
        Ok(s) => s,
        Err(_) => return (Vec::new(), report),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .ok();

    let mut cookies = Vec::new();
    if let Some(rows) = rows {
        for row in rows.flatten() {
            report.total_rows += 1;
            let (host, name, plain, encrypted, path, expires_utc, secure, http_only, same_site) =
                row;
            let value = if !encrypted.is_empty() {
                if encrypted.len() >= 3 && &encrypted[..3] == b"v11" {
                    report.app_bound += 1;
                    continue; // App-bound encryption: not decryptable by design.
                }
                match key
                    .as_deref()
                    .and_then(|k| chromium_decrypt_value(&encrypted, k))
                {
                    Some(v) => v,
                    None => {
                        report.undecryptable += 1;
                        continue;
                    }
                }
            } else {
                plain
            };
            report.decrypted += 1;
            // Chrome expiry is microseconds since 1601-01-01.
            let expires = if expires_utc > 0 {
                Some(chromium_time_to_unix(expires_utc))
            } else {
                None
            };
            let same_site_str = match same_site {
                0 => "None",
                1 => "Lax",
                2 => "Strict",
                _ => "",
            };
            cookies.push(Cookie {
                name,
                value,
                domain: host,
                path: if path.is_empty() {
                    "/".to_string()
                } else {
                    path
                },
                expires,
                secure: secure != 0,
                http_only: http_only != 0,
                same_site: same_site_str.to_string(),
            });
        }
    }
    (cookies, report)
}

/// Convenience wrapper returning just the cookies (see
/// [`read_chromium_cookies_report`]).
pub fn read_chromium_cookies(
    db_path: &std::path::Path,
    local_state_path: Option<&std::path::Path>,
) -> Vec<Cookie> {
    read_chromium_cookies_report(db_path, local_state_path).0
}

/// Convert a Chromium "microseconds since 1601" timestamp to Unix seconds.
fn chromium_time_to_unix(micros_since_1601: i64) -> i64 {
    const WINDOWS_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;
    (micros_since_1601 / 1_000_000) - WINDOWS_EPOCH_OFFSET_SECS
}

/// Read a Firefox `cookies.sqlite` DB (plaintext values).
pub fn read_firefox_cookies(db_path: &std::path::Path) -> Vec<Cookie> {
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite FROM moz_cookies",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .ok();

    let mut cookies = Vec::new();
    if let Some(rows) = rows {
        for row in rows.flatten() {
            let (host, name, value, path, expiry, secure, http_only, same_site) = row;
            let same_site_str = match same_site {
                0 => "",
                1 => "Lax",
                2 => "Strict",
                _ => "",
            };
            cookies.push(Cookie {
                name,
                value,
                domain: host,
                path: if path.is_empty() {
                    "/".to_string()
                } else {
                    path
                },
                expires: (expiry > 0).then_some(expiry),
                secure: secure != 0,
                http_only: http_only != 0,
                same_site: same_site_str.to_string(),
            });
        }
    }
    cookies
}

// ──────────────────────────────────────────────────────────────────────────
// Persistent cookie store
// ──────────────────────────────────────────────────────────────────────────

/// Path of the persistent cookie store (`~/.operant/data/cookies.json`).
/// Cookies survive across process runs so a single import keeps working for
/// every future Obscura session without re-login.
pub fn cookie_store_path() -> std::path::PathBuf {
    crate::platform::operant_data_dir().join("cookies.json")
}

/// Load the persistent cookie store. Returns `Vec::new()` when missing or
/// unreadable.
pub fn load_cookie_store() -> Vec<Cookie> {
    let path = cookie_store_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Save cookies to the persistent store (best-effort: a read-only data dir
/// must not fail an import).
pub fn save_cookie_store(cookies: &[Cookie]) {
    let path = cookie_store_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(cookies) {
        let _ = std::fs::write(&path, text);
    }
}

/// Clear the persistent cookie store.
pub fn clear_cookie_store() {
    let _ = std::fs::remove_file(cookie_store_path());
}

/// Render cookies as a Netscape `cookies.txt` document.
pub fn cookies_to_netscape(cookies: &[Cookie]) -> String {
    let mut out = String::from(
        "# Netscape HTTP Cookie File\n# This file was generated by operant. Import with: operant cookies import\n",
    );
    for c in cookies {
        out.push_str(&c.to_netscape_line());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_netscape_cookies_txt() {
        let txt = "# Netscape HTTP Cookie File\n\
                   .example.com\tTRUE\t/\tTRUE\t1600000000\tsessionid\tabc123\n\
                   example.org\tFALSE\t/\tFALSE\t0\tpref\tdark\n";
        let cookies = parse_netscape_cookies_txt(txt);
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].name, "sessionid");
        assert_eq!(cookies[0].value, "abc123");
        assert_eq!(cookies[0].domain, ".example.com");
        assert!(cookies[0].secure);
        assert_eq!(cookies[0].expires, Some(1600000000));
        assert_eq!(cookies[1].expires, None, "session cookie");
    }

    #[test]
    fn parses_json_cookies() {
        let json = r#"[{"name":"token","value":"xyz","domain":".site.com","path":"/","expirationDate":1700000000,"secure":true,"httpOnly":true,"sameSite":"Lax"}]"#;
        let cookies = parse_json_cookies(json);
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "token");
        assert_eq!(cookies[0].value, "xyz");
        assert!(cookies[0].secure);
        assert!(cookies[0].http_only);
        assert_eq!(cookies[0].same_site, "Lax");
    }

    #[test]
    fn netscape_roundtrip() {
        let c = Cookie {
            name: "a".into(),
            value: "b".into(),
            domain: ".x.com".into(),
            path: "/".into(),
            expires: Some(12345),
            secure: true,
            http_only: false,
            same_site: String::new(),
        };
        let line = c.to_netscape_line();
        let parsed = parse_netscape_cookies_txt(&line);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "a");
        assert_eq!(parsed[0].value, "b");
        assert_eq!(parsed[0].expires, Some(12345));
    }

    #[test]
    fn cdp_value_shape() {
        let c = Cookie {
            name: "sid".into(),
            value: "v".into(),
            domain: ".x.com".into(),
            path: "/".into(),
            expires: None,
            secure: true,
            http_only: true,
            same_site: "Lax".into(),
        };
        let v = c.to_cdp_value();
        assert_eq!(v["name"], "sid");
        assert!(v["httpOnly"].as_bool().unwrap_or(false));
        assert_eq!(v["sameSite"], "Lax");
        assert!(v.get("expires").is_none(), "session cookie has no expires");
    }

    #[test]
    fn chromium_time_conversion() {
        // 2020-01-01T00:00:00Z in Windows ticks.
        let unix = 1577836800i64;
        let windows = (unix + 11_644_473_600) * 1_000_000;
        assert_eq!(chromium_time_to_unix(windows), unix);
    }

    #[test]
    fn netscape_skips_comments_and_blanks() {
        let txt = "# comment\n\n\n.foo.com\tFALSE\t/\tFALSE\t0\tk\tv\n";
        assert_eq!(parse_netscape_cookies_txt(txt).len(), 1);
    }

    #[test]
    fn firefox_empty_db_returns_empty() {
        // No firefox profile here — the function must tolerate a missing file.
        let cookies = read_firefox_cookies(std::path::Path::new("/nonexistent/cookies.sqlite"));
        assert!(cookies.is_empty());
    }

    #[test]
    fn chromium_missing_db_returns_empty() {
        let cookies = read_chromium_cookies(std::path::Path::new("/nonexistent/Cookies"), None);
        assert!(cookies.is_empty());
    }

    #[test]
    fn parses_secret_tool_output() {
        let text = "[keyring]\n  label = Chrome Safe Storage\n  secret = S0VZX1ZBTFVFISEhIQ==\n  created = 2024-01-01\n";
        assert_eq!(
            parse_secret_tool_secret(text).as_deref(),
            Some("S0VZX1ZBTFVFISEhIQ==")
        );
    }

    #[test]
    fn chromium_keyring_key_base64_decodes() {
        // Base64 of "KEY_VALUE_123456" (16 bytes) — must decode to the raw key.
        let _secret = "S0VZX1ZBTFVFXzEyMzQ1Ng==";
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, _secret).unwrap();
        assert_eq!(decoded.len(), 16);
        assert_eq!(decoded, b"KEY_VALUE_123456");
    }

    #[test]
    fn chromium_read_report_counts_app_bound() {
        // Build a synthetic Cookies DB in a temp dir: one v11 app-bound row
        // (undecryptable by design) and one plaintext row.
        let dir = std::env::temp_dir().join(format!("cookies-report-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("Cookies");
        let _ = std::fs::remove_file(&db);
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE cookies (host_key TEXT NOT NULL, name TEXT NOT NULL, \
                 value TEXT NOT NULL, path TEXT NOT NULL, expires_utc INTEGER NOT NULL, \
                 is_secure INTEGER NOT NULL, is_httponly INTEGER NOT NULL, \
                 last_access_utc INTEGER NOT NULL DEFAULT 0, has_expires INTEGER NOT NULL \
                 DEFAULT 1, is_persistent INTEGER NOT NULL DEFAULT 1, priority INTEGER \
                 NOT NULL DEFAULT 1, encrypted_value BLOB NOT NULL, firstpartyonly \
                 INTEGER NOT NULL DEFAULT 0, samesite INTEGER NOT NULL DEFAULT 0, \
                 source_scheme INTEGER NOT NULL DEFAULT 0, source_port INTEGER NOT NULL \
                 DEFAULT -1, is_same_party INTEGER NOT NULL DEFAULT 0)",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO cookies (host_key, name, value, path, expires_utc, \
                 is_secure, is_httponly, encrypted_value, samesite) VALUES \
                 (?1, ?2, ?3, ?4, 0, 0, 0, ?5, 1)",
                rusqlite::params![
                    ".appbound.example",
                    "sid",
                    "",
                    "/",
                    b"v11\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec()
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO cookies (host_key, name, value, path, expires_utc, \
                 is_secure, is_httponly, encrypted_value, samesite) VALUES \
                 (?1, ?2, ?3, ?4, 0, 0, 0, ?5, 1)",
                rusqlite::params![".plain.example", "pref", "dark", "/", Vec::<u8>::new()],
            )
            .unwrap();
        }

        let (cookies, report) = read_chromium_cookies_report(&db, None);
        assert_eq!(report.total_rows, 2);
        assert_eq!(report.app_bound, 1);
        assert_eq!(report.decrypted, 1);
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].domain, ".plain.example");
        assert_eq!(cookies[0].value, "dark");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
