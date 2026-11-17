use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

const SPOTIFY_API: &str = "https://api.spotify.com/v1";

fn spotify_token() -> Option<String> {
    std::env::var("OPERANT_SPOTIFY_ACCESS_TOKEN")
        .or_else(|_| std::env::var("HERMES_SPOTIFY_ACCESS_TOKEN"))
        .ok()
}

fn spotify_available() -> bool {
    std::env::var("OPERANT_SPOTIFY_ACCESS_TOKEN")
        .or_else(|_| std::env::var("HERMES_SPOTIFY_ACCESS_TOKEN"))
        .is_ok()
        || (std::env::var("SPOTIFY_CLIENT_ID").is_ok()
            && std::env::var("SPOTIFY_CLIENT_SECRET").is_ok())
}

#[derive(Clone)]
struct SpotifyClient {
    client: reqwest::Client,
    base: String,
}

impl SpotifyClient {
    fn new() -> Option<Self> {
        let _token = spotify_token()?;
        Some(Self {
            client: reqwest::Client::new(),
            base: SPOTIFY_API.to_string(),
        })
    }

    /// Turns one HTTP response into the tool result the model is allowed to see.
    ///
    /// Every helper below used to answer `ToolResult::success(...,
    /// r.text().await.unwrap_or_default())` on any completed request. Two
    /// defects, both of them "reporting success without the thing the caller
    /// asked for":
    ///
    /// 1. **The status was never read.** Spotify answers a revoked token with
    ///    401 and a JSON body, a missing resource with 404, a rate limit with
    ///    429 (carrying a Retry-After the caller needs). All of those arrived as
    ///    `success: true`, so the agent read `{"error": {...}}` as if it were
    ///    player or playlist state.
    /// 2. **`unwrap_or_default()` on the body.** A connection that dropped
    ///    mid-response produced `success: true` with an EMPTY STRING — the
    ///    quietest possible failure, indistinguishable from a genuinely empty
    ///    payload.
    ///
    /// A non-2xx is an error; a body that could not be read is an error. This is
    /// the same shape `transcription_envelope` restored in iter-2554: a tool
    /// that hides a failure from the agent blinds the turn guardrail, the
    /// observer and the failure counter, not just the human.
    async fn response_result(name: &str, response: reqwest::Response) -> ToolResult {
        let status = response.status();
        let body = match response.text().await {
            Ok(body) => body,
            Err(e) => {
                return ToolResult::error(
                    name,
                    format!("Spotify returned HTTP {status} and the body could not be read: {e}"),
                );
            }
        };
        if !status.is_success() {
            let detail = if body.trim().is_empty() {
                "no response body".to_string()
            } else {
                body
            };
            return ToolResult::error(name, format!("Spotify returned HTTP {status}: {detail}"));
        }
        ToolResult::success(name, Value::String(body))
    }

    /// A token that is not valid header text is an error for the caller to
    /// report, not a panic.
    fn headers(&self) -> Result<reqwest::header::HeaderMap, String> {
        let token = spotify_token().unwrap_or_default();
        let mut h = reqwest::header::HeaderMap::new();
        let bearer = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|e| format!("Spotify token is not a valid header value: {e}"))?;
        h.insert(reqwest::header::AUTHORIZATION, bearer);
        h.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        Ok(h)
    }

    async fn get(&self, path: &str) -> ToolResult {
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return ToolResult::error("spotify", e),
        };
        match self
            .client
            .get(format!("{}{}", self.base, path))
            .headers(headers)
            .send()
            .await
        {
            Ok(r) => Self::response_result("spotify", r).await,
            Err(e) => ToolResult::error("spotify", format!("API error: {}", e)),
        }
    }

    async fn post(&self, path: &str, body: Option<Value>) -> ToolResult {
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return ToolResult::error("spotify", e),
        };
        let mut req = self
            .client
            .post(format!("{}{}", self.base, path))
            .headers(headers);
        if let Some(b) = body {
            req = req.json(&b);
        }
        match req.send().await {
            Ok(r) => Self::response_result("spotify", r).await,
            Err(e) => ToolResult::error("spotify", format!("API error: {}", e)),
        }
    }

    async fn put(&self, path: &str, body: Option<Value>) -> ToolResult {
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return ToolResult::error("spotify", e),
        };
        let mut req = self
            .client
            .put(format!("{}{}", self.base, path))
            .headers(headers);
        if let Some(b) = body {
            req = req.json(&b);
        }
        match req.send().await {
            Ok(r) => Self::response_result("spotify", r).await,
            Err(e) => ToolResult::error("spotify", format!("API error: {}", e)),
        }
    }

    async fn delete(&self, path: &str, body: Option<Value>) -> ToolResult {
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return ToolResult::error("spotify", e),
        };
        let mut req = self
            .client
            .delete(format!("{}{}", self.base, path))
            .headers(headers);
        if let Some(b) = body {
            req = req.json(&b);
        }
        match req.send().await {
            Ok(r) => Self::response_result("spotify", r).await,
            Err(e) => ToolResult::error("spotify", format!("API error: {}", e)),
        }
    }
}

pub struct SpotifyPlaybackTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PlaybackArgs {
    action: PlaybackAction,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    position_ms: Option<u32>,
    #[serde(default)]
    uris: Option<Vec<String>>,
    #[serde(default)]
    context_uri: Option<String>,
    #[serde(default)]
    state: Option<bool>,
    #[serde(default)]
    volume_percent: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum PlaybackAction {
    Get,
    StartResume,
    Pause,
    Next,
    Previous,
    Seek,
    SetRepeat,
    SetShuffle,
    SetVolume,
    Transfer,
}

#[async_trait]
impl OperantTool for SpotifyPlaybackTool {
    fn name(&self) -> &str {
        "spotify_playback"
    }
    fn description(&self) -> &str {
        "Control Spotify playback: get state, play/pause, next/previous, seek, repeat, shuffle, volume, transfer."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<PlaybackArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: PlaybackArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        match p.action {
            PlaybackAction::Get => sc.get("/me/player").await,
            PlaybackAction::StartResume => {
                let mut qs = String::new();
                if let Some(ref d) = p.device_id {
                    qs = format!("?device_id={}", d);
                }
                let mut body = None;
                if let Some(ref u) = p.uris {
                    body = Some(json!({"uris": u}));
                } else if let Some(ref c) = p.context_uri {
                    body = Some(json!({"context_uri": c}));
                }
                sc.put(&format!("/me/player/play{}", qs), body).await
            }
            PlaybackAction::Pause => {
                let qs = p
                    .device_id
                    .as_ref()
                    .map(|d| format!("?device_id={}", d))
                    .unwrap_or_default();
                sc.put(&format!("/me/player/pause{}", qs), None).await
            }
            PlaybackAction::Next => {
                let qs = p
                    .device_id
                    .as_ref()
                    .map(|d| format!("?device_id={}", d))
                    .unwrap_or_default();
                sc.post(&format!("/me/player/next{}", qs), None).await
            }
            PlaybackAction::Previous => {
                let qs = p
                    .device_id
                    .as_ref()
                    .map(|d| format!("?device_id={}", d))
                    .unwrap_or_default();
                sc.post(&format!("/me/player/previous{}", qs), None).await
            }
            PlaybackAction::Seek => {
                let ms = p.position_ms.unwrap_or(0);
                sc.put(&format!("/me/player/seek?position_ms={}", ms), None)
                    .await
            }
            PlaybackAction::SetRepeat => {
                let state = if p.state.unwrap_or(false) {
                    "context"
                } else {
                    "off"
                };
                sc.put(&format!("/me/player/repeat?state={}", state), None)
                    .await
            }
            PlaybackAction::SetShuffle => {
                let state = if p.state.unwrap_or(true) {
                    "true"
                } else {
                    "false"
                };
                sc.put(&format!("/me/player/shuffle?state={}", state), None)
                    .await
            }
            PlaybackAction::SetVolume => {
                let vol = p.volume_percent.unwrap_or(50).min(100);
                sc.put(&format!("/me/player/volume?volume_percent={}", vol), None)
                    .await
            }
            PlaybackAction::Transfer => {
                let device_id = p.device_id.unwrap_or_default();
                sc.put("/me/player", Some(json!({"device_ids": [device_id]})))
                    .await
            }
        }
    }
}

pub struct SpotifyDevicesTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct DevicesArgs {
    action: DevicesAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum DevicesAction {
    List,
}

#[async_trait]
impl OperantTool for SpotifyDevicesTool {
    fn name(&self) -> &str {
        "spotify_devices"
    }
    fn description(&self) -> &str {
        "List available Spotify devices for playback."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<DevicesArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let _p: DevicesArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        sc.get("/me/player/devices").await
    }
}

pub struct SpotifyQueueTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct QueueArgs {
    action: QueueAction,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum QueueAction {
    Get,
    Add,
}

#[async_trait]
impl OperantTool for SpotifyQueueTool {
    fn name(&self) -> &str {
        "spotify_queue"
    }
    fn description(&self) -> &str {
        "View the playback queue or add items to it."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<QueueArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: QueueArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        match p.action {
            QueueAction::Get => sc.get("/me/player/queue").await,
            QueueAction::Add => {
                let uri = p.uri.unwrap_or_default();
                sc.post(&format!("/me/player/queue?uri={}", uri), None)
                    .await
            }
        }
    }
}

pub struct SpotifySearchTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct SearchArgs {
    action: SearchAction,
    query: String,
    #[serde(default)]
    types: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum SearchAction {
    Search,
}

#[async_trait]
impl OperantTool for SpotifySearchTool {
    fn name(&self) -> &str {
        "spotify_search"
    }
    fn description(&self) -> &str {
        "Search Spotify for tracks, albums, artists, or playlists."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SearchArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: SearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        let types = p.types.unwrap_or_else(|| "track".to_string());
        let limit = p.limit.unwrap_or(10).min(50);
        let encoded: String = urlencoding(&p.query);
        sc.get(&format!(
            "/search?q={}&type={}&limit={}",
            encoded, types, limit
        ))
        .await
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

pub struct SpotifyPlaylistsTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PlaylistsArgs {
    action: PlaylistsAction,
    #[serde(default)]
    playlist_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    public: Option<bool>,
    #[serde(default)]
    uris: Option<Vec<String>>,
    #[serde(default)]
    position: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum PlaylistsAction {
    List,
    Get,
    Create,
    AddItems,
    RemoveItems,
}

#[async_trait]
impl OperantTool for SpotifyPlaylistsTool {
    fn name(&self) -> &str {
        "spotify_playlists"
    }
    fn description(&self) -> &str {
        "Manage Spotify playlists: list, view, create, add/remove tracks."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<PlaylistsArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: PlaylistsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        match p.action {
            PlaylistsAction::List => {
                let limit = p.limit.unwrap_or(20).min(50);
                sc.get(&format!("/me/playlists?limit={}", limit)).await
            }
            PlaylistsAction::Get => {
                let id = p.playlist_id.unwrap_or_default();
                sc.get(&format!("/playlists/{}", id)).await
            }
            PlaylistsAction::Create => {
                let name = p.name.unwrap_or_else(|| "New Playlist".to_string());
                let desc = p.description.unwrap_or_default();
                sc.post("/me/playlists", Some(json!({"name": name, "description": desc, "public": p.public.unwrap_or(true)}))).await
            }
            PlaylistsAction::AddItems => {
                let id = p.playlist_id.unwrap_or_default();
                let uris = p.uris.unwrap_or_default();
                let mut body = json!({"uris": uris});
                if let Some(pp) = p.position {
                    body["position"] = json!(pp);
                }
                sc.post(&format!("/playlists/{}/tracks", id), Some(body))
                    .await
            }
            PlaylistsAction::RemoveItems => {
                let id = p.playlist_id.unwrap_or_default();
                let uris = p.uris.unwrap_or_default();
                let tracks: Vec<Value> = uris.iter().map(|u| json!({"uri": u})).collect();
                sc.delete(
                    &format!("/playlists/{}/tracks", id),
                    Some(json!({"tracks": tracks})),
                )
                .await
            }
        }
    }
}

pub struct SpotifyAlbumsTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AlbumsArgs {
    action: AlbumsAction,
    #[serde(default)]
    album_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum AlbumsAction {
    ListSaved,
    Get,
    GetTracks,
}

#[async_trait]
impl OperantTool for SpotifyAlbumsTool {
    fn name(&self) -> &str {
        "spotify_albums"
    }
    fn description(&self) -> &str {
        "Browse your saved albums or view album details and tracks."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AlbumsArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: AlbumsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        match p.action {
            AlbumsAction::ListSaved => {
                let limit = p.limit.unwrap_or(20).min(50);
                sc.get(&format!("/me/albums?limit={}", limit)).await
            }
            AlbumsAction::Get => {
                let id = p.album_id.unwrap_or_default();
                sc.get(&format!("/albums/{}", id)).await
            }
            AlbumsAction::GetTracks => {
                let id = p.album_id.unwrap_or_default();
                let limit = p.limit.unwrap_or(20).min(50);
                sc.get(&format!("/albums/{}/tracks?limit={}", id, limit))
                    .await
            }
        }
    }
}

pub struct SpotifyLibraryTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct LibraryArgs {
    action: LibraryAction,
    #[serde(default)]
    ids: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
enum LibraryAction {
    SavedTracks,
    SavedAlbums,
    CheckSaved,
}

#[async_trait]
impl OperantTool for SpotifyLibraryTool {
    fn name(&self) -> &str {
        "spotify_library"
    }
    fn description(&self) -> &str {
        "Access your Spotify library: saved tracks, albums, and check saved status."
    }
    fn toolset(&self) -> &str {
        "media"
    }
    fn is_available(&self) -> bool {
        spotify_available()
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LibraryArgs>(self.name(), self.description())
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let p: LibraryArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };
        let sc = match SpotifyClient::new() {
            Some(c) => c,
            None => return ToolResult::error(self.name(), "HERMES_SPOTIFY_ACCESS_TOKEN not set"),
        };
        match p.action {
            LibraryAction::SavedTracks => {
                let limit = p.limit.unwrap_or(20).min(50);
                sc.get(&format!("/me/tracks?limit={}", limit)).await
            }
            LibraryAction::SavedAlbums => {
                let limit = p.limit.unwrap_or(20).min(50);
                sc.get(&format!("/me/albums?limit={}", limit)).await
            }
            LibraryAction::CheckSaved => {
                let ids = p.ids.clone().unwrap_or_default();
                sc.get(&format!("/me/tracks/contains?ids={}", ids.join(",")))
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_schema {
        ($name:ident, $tool:expr, $tool_name:expr) => {
            #[tokio::test]
            async fn $name() {
                assert_eq!($tool.name(), $tool_name);
                assert!(!$tool.description().is_empty());
                let schema = $tool.schema();
                assert_eq!(schema.name, $tool_name);
                assert!(serde_json::to_string(&schema.parameters).is_ok());
            }
        };
    }

    #[tokio::test]
    async fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a/b"), "a%2Fb");
        assert_eq!(urlencoding("test"), "test");
    }

    test_schema!(
        test_playback_schema,
        SpotifyPlaybackTool,
        "spotify_playback"
    );
    test_schema!(test_devices_schema, SpotifyDevicesTool, "spotify_devices");
    test_schema!(test_queue_schema, SpotifyQueueTool, "spotify_queue");
    test_schema!(test_search_schema, SpotifySearchTool, "spotify_search");
    test_schema!(
        test_playlists_schema,
        SpotifyPlaylistsTool,
        "spotify_playlists"
    );
    test_schema!(test_albums_schema, SpotifyAlbumsTool, "spotify_albums");
    test_schema!(test_library_schema, SpotifyLibraryTool, "spotify_library");

    /// A one-line HTTP response server, so these tests drive a REAL reqwest
    /// response through `response_result` rather than a synthetic one. `status`
    /// and `body` are the only things a test needs to vary, and a real socket
    /// means a body that drops mid-read is testable too (reqwest errors on it
    /// instead of a mock quietly returning an empty string).
    async fn serve_once(status_line: &'static str, body: &'static str, drop_body: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            // Read the request head so the client's write completes.
            let mut buf = vec![0u8; 64 * 1024];
            let _ = socket.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            if drop_body {
                // Announce a body, send part of it, then hang up: reqwest's
                // `text()` must return Err, which the OLD code turned into
                // `success: true` with an empty string.
                let partial = &body[..body.len() / 2];
                let _ = socket
                    .write_all(format!("{head}{partial}").as_bytes())
                    .await;
                let _ = socket.shutdown().await;
            } else {
                let _ = socket.write_all(format!("{head}{body}").as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    /// The two defects this file had: a non-2xx Spotify answer and an
    /// unreadable body both arrived as `success: true`. A tool that reports
    /// success on failure blinds the turn guardrail, the observer and the
    /// failure counter, not just the reader.
    #[tokio::test]
    async fn a_spotify_error_status_is_not_reported_as_success() {
        let url = serve_once(
            "401 Unauthorized",
            r#"{"error":{"status":401,"message":"Invalid access token"}}"#,
            false,
        )
        .await;
        let response = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect("the loopback server answers");

        let result = SpotifyClient::response_result("spotify", response).await;
        assert!(!result.success, "a 401 must not be a success: {result:?}");
        let error = result.error.unwrap_or_default();
        assert!(
            error.contains("401"),
            "the status must reach the agent: {error}"
        );
        assert!(
            error.contains("Invalid access token"),
            "the provider's message must reach the agent: {error}"
        );
    }

    /// A 429 carries a Retry-After the caller needs; dropping it to
    /// `success: true` loses the one actionable field.
    #[tokio::test]
    async fn a_rate_limited_spotify_answer_is_an_error() {
        let url = serve_once(
            "429 Too Many Requests",
            r#"{"error":"rate limited"}"#,
            false,
        )
        .await;
        let response = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect("the loopback server answers");

        let result = SpotifyClient::response_result("spotify", response).await;
        assert!(!result.success, "429 must not be success: {result:?}");
    }

    /// The `unwrap_or_default()` half: a body that could not be read used to
    /// be success with an empty string.
    #[tokio::test]
    async fn a_spotify_body_that_cannot_be_read_is_an_error() {
        let url = serve_once("200 OK", "{\"is_playing\":true}", true).await;
        let response = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect("the loopback server answers");

        let result = SpotifyClient::response_result("spotify", response).await;
        assert!(
            !result.success,
            "an unreadable body must not be success: {result:?}"
        );
    }
}
