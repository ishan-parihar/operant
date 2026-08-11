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

/// Decrypt a Chromium-family `encrypted_value` blob (AES-128-CBC, "v10"
/// prefix) with the key from `Local State` (`os_crypt.encrypted_key`).
///
/// On Linux, when the OS keyring is unavailable Chromium falls back to a
/// hardcoded "peanuts" password; when a keyring is used the real key is
/// stored in `Local State` base64-encoded with a "DPAPI" prefix (Windows) or
/// a raw prefix. This implements the standard Linux scheme:
/// - `encrypted_key` in Local State: base64, prefixed `DPAPI` on Windows.
/// - AES-128-CBC decrypt of the key blob with key = SHA256("peanuts")[..16]
///   and IV = 16 zero bytes (Chromium's `OSCrypt::DecryptString`).
fn chromium_decryption_key(local_state_path: &std::path::Path) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let local_state: Value =
        serde_json::from_str(&std::fs::read_to_string(local_state_path).ok()?).ok()?;
    let enc_key_b64 = local_state
        .get("os_crypt")?
        .get("encrypted_key")?
        .as_str()?;
    let enc_key =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, enc_key_b64).ok()?;
    // Strip the "DPAPI" prefix (Windows). On Linux there is no prefix.
    let key_blob = if enc_key.starts_with(b"DPAPI") {
        &enc_key[5..]
    } else {
        &enc_key[..]
    };

    // Chromium's Linux fallback: the key blob is itself AES-128-CBC
    // encrypted with key = SHA256("peanuts")[:16], IV = 16 zeros.
    let key_material = Sha256::digest(b"peanuts");
    let aes_key: &[u8; 16] = &key_material[..16].try_into().ok()?;
    let iv = [0u8; 16];
    aes128_cbc_decrypt(aes_key, &iv, key_blob)
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

/// Read a Chromium-family `Cookies` SQLite DB (Chrome/Brave/Edge/…).
/// Requires the matching `Local State` for v10 decryption. Returns cookies
/// with decrypted values; undecryptable rows are skipped.
pub fn read_chromium_cookies(
    db_path: &std::path::Path,
    local_state_path: Option<&std::path::Path>,
) -> Vec<Cookie> {
    let key = local_state_path.and_then(chromium_decryption_key);
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT host_key, name, value, encrypted_value, path, expires_utc, \
         is_secure, is_httponly, samesite FROM cookies",
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
            let (host, name, plain, encrypted, path, expires_utc, secure, http_only, same_site) =
                row;
            let value = if !encrypted.is_empty() {
                match key
                    .as_deref()
                    .and_then(|k| chromium_decrypt_value(&encrypted, k))
                {
                    Some(v) => v,
                    None => continue, // Undecryptable (keyring-bound profile).
                }
            } else {
                plain
            };
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
    cookies
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
}
