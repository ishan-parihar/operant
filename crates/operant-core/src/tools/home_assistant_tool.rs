//! Home Assistant Tool - Control smart home devices via REST API
//!
//! Uses the Home Assistant REST API to list entities, get detailed state,
//! discover available services, and call services (turn_on/off, set temperature, etc.).
//!
//! Configured via environment variables at call time:
//! - `HASS_URL`  (default: http://homeassistant.local:8123)
//! - `HASS_TOKEN` (required for authentication)
//!
//! Security:
//! - Entity IDs are validated against `^[a-z_][a-z0-9_]*\.[a-z0-9_]+$`
//! - Domain/service names are validated against `^[a-z][a-z0-9_]*$`
//! - Dangerous domains (shell_command, python_script, etc.) are blocked outright
//! - JSON strings in the `data` parameter are parsed and validated

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::Duration;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Validation regular expressions (compiled once at module load)
// ---------------------------------------------------------------------------

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Matches valid Home Assistant entity_id format: `domain.entity`
/// Examples: `light.living_room`, `sensor.temperature_1`, `climate.thermostat`
static ENTITY_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z_][a-z0-9_]*\.[a-z0-9_]+$").expect("Failed to compile entity_id regex")
});

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Matches valid domain or service name: lowercase ASCII, digits, underscores.
/// Prevents path-traversal payloads like `../../api/config` in URL segments.
static SERVICE_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9_]*$").expect("Failed to compile service/domain regex")
});

// ---------------------------------------------------------------------------
// Blocked domains
// ---------------------------------------------------------------------------
// These Home Assistant integration domains allow arbitrary code execution,
// command execution, or SSRF on the HA host. HA provides zero service-level
// access control, so safety must be enforced at our layer.

const BLOCKED_DOMAINS: &[&str] = &[
    "shell_command", // arbitrary shell commands as root in HA container
    "command_line",  // sensors/switches that execute shell commands
    "python_script", // sandboxed but can escalate via hass.services.call()
    "pyscript",      // scripting integration with broader access
    "hassio",        // addon control, host shutdown/reboot, stdin to containers
    "rest_command",  // HTTP requests from HA server (SSRF vector)
];

// ---------------------------------------------------------------------------
// Configuration helpers
// ---------------------------------------------------------------------------

/// Read HASS_URL and HASS_TOKEN from environment variables at call time.
///
/// Matches the Python `_get_config()` behavior:
/// - HASS_URL defaults to `http://homeassistant.local:8123`
/// - HASS_TOKEN is required (empty string if unset, will cause 401)
/// - Trailing slash is stripped from URL
fn get_config() -> (String, String) {
    let url =
        std::env::var("HASS_URL").unwrap_or_else(|_| "http://homeassistant.local:8123".to_string());
    let token = std::env::var("HASS_TOKEN").unwrap_or_default();
    (url.trim_end_matches('/').to_string(), token)
}

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Build authorization headers for the Home Assistant REST API.
///
/// Returns a `HeaderMap` with:
/// - `Authorization: Bearer {token}`
/// - `Content-Type: application/json`
fn get_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token)).expect("Invalid Bearer token format"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

// ---------------------------------------------------------------------------
// Filtering and response helpers
// ---------------------------------------------------------------------------

/// Filter raw HA states by domain/area and return a compact summary.
///
/// Each entity in the output contains:
/// - `entity_id` (e.g. `"light.living_room"`)
/// - `state` (e.g. `"on"`, `"off"`, `"23.5"`)
/// - `friendly_name` from attributes (or empty string)
///
/// When `domain` is provided, only entities whose `entity_id` starts with
/// `"{domain}."` are included. When `area` is provided, entities are matched
/// against their `friendly_name` or `attributes.area` (case-insensitive substring).
fn filter_and_summarize(states: &[Value], domain: Option<&str>, area: Option<&str>) -> Value {
    let domain_lower = domain.map(str::to_lowercase);
    let area_lower = area.map(str::to_lowercase);

    let entities: Vec<Value> = states
        .iter()
        .filter(|s| {
            let entity_id = s.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");

            // Filter by domain prefix
            if let Some(ref d) = domain_lower
                && !entity_id.starts_with(&format!("{}.", d))
            {
                return false;
            }

            // Filter by area (matches against friendly_name or attributes.area)
            if let Some(ref a) = area_lower {
                let friendly_name = s
                    .get("attributes")
                    .and_then(|attrs| attrs.get("friendly_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let entity_area = s
                    .get("attributes")
                    .and_then(|attrs| attrs.get("area"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();

                if !friendly_name.contains(a.as_str()) && !entity_area.contains(a.as_str()) {
                    return false;
                }
            }

            true
        })
        .map(|s| {
            json!({
                "entity_id": s.get("entity_id").and_then(|v| v.as_str()).unwrap_or(""),
                "state": s.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                "friendly_name": s.get("attributes")
                    .and_then(|attrs| attrs.get("friendly_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            })
        })
        .collect();

    json!({
        "count": entities.len(),
        "entities": entities,
    })
}

/// Build the JSON payload for a Home Assistant service call.
///
/// Merges `data` fields first, then overlays `entity_id` (so entity_id takes
/// precedence if it appears in both places, matching Python behavior).
fn build_service_payload(entity_id: Option<&str>, data: Option<&Value>) -> Value {
    let mut payload = serde_json::Map::new();

    // Merge data fields into payload
    if let Some(data_val) = data
        && let Some(obj) = data_val.as_object()
    {
        for (k, v) in obj {
            payload.insert(k.clone(), v.clone());
        }
    }

    // entity_id parameter takes precedence over data["entity_id"]
    if let Some(eid) = entity_id {
        payload.insert("entity_id".to_string(), Value::String(eid.to_string()));
    }

    Value::Object(payload)
}

/// Parse a Home Assistant service call response into a structured result.
///
/// Extracts affected entities from the response array (or returns an empty
/// list if the response is not an array).
fn parse_service_response(domain: &str, service: &str, result: &Value) -> Value {
    let affected: Vec<Value> = match result.as_array() {
        Some(arr) => arr
            .iter()
            .map(|s| {
                json!({
                    "entity_id": s.get("entity_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "state": s.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                })
            })
            .collect(),
        None => vec![],
    };

    json!({
        "success": true,
        "service": format!("{}.{}", domain, service),
        "affected_entities": affected,
    })
}

// ===========================================================================
// Tool arguments schema
// ===========================================================================

/// Arguments for the Home Assistant tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct HomeAssistantArgs {
    /// Action to perform: list_entities, get_state, list_services, or call_service.
    action: String,

    /// Entity ID to query or target (e.g. "light.living_room", "climate.thermostat").
    /// Required for get_state, optional for call_service.
    entity_id: Option<String>,

    /// Domain filter for list_entities and list_services (e.g. "light", "switch", "climate", "sensor").
    /// Omit to include all domains.
    domain: Option<String>,

    /// Area/room filter for list_entities. Matches against entity friendly_name or area attribute.
    /// Example: "living room", "kitchen", "bedroom".
    area: Option<String>,

    /// Service name for call_service (e.g. "turn_on", "turn_off", "toggle", "set_temperature").
    /// Required for call_service.
    service: Option<String>,

    /// Additional service data as a JSON string for call_service.
    /// Examples: {"brightness": 255, "color_name": "blue"} for lights,
    /// {"temperature": 22, "hvac_mode": "heat"} for climate.
    data: Option<String>,
}

// ===========================================================================
// Tool implementation
// ===========================================================================

/// Home Assistant tool for controlling smart home devices via REST API.
///
/// Combines four actions into a single tool discriminated by the `action` parameter:
/// - `list_entities` — list/filter entities
/// - `get_state` — get detailed state for a single entity
/// - `list_services` — discover available services per domain
/// - `call_service` — call a service to control a device
pub struct HomeAssistantTool;

impl HomeAssistantTool {
    /// Create a new HomeAssistantTool.
    pub fn new() -> Self {
        Self
    }

    // -----------------------------------------------------------------------
    // Action: list_entities
    // -----------------------------------------------------------------------

    /// Fetch entity states from HA and optionally filter by domain/area.
    ///
    /// Returns a compact summary with count and entity list (entity_id, state, friendly_name).
    async fn handle_list_entities(&self, args: &Value) -> ToolResult {
        let domain = args.get("domain").and_then(|v| v.as_str());
        let area = args.get("area").and_then(|v| v.as_str());

        let (hass_url, hass_token) = get_config();
        let url = format!("{}/api/states", hass_url);
        let headers = get_headers(&hass_token);

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_entities",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let response = match client.get(&url).headers(headers).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_entities",
                    format!(
                        "Network error when connecting to Home Assistant at {}: {}",
                        url, e
                    ),
                );
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return ToolResult::error(
                "ha_list_entities",
                format!("Home Assistant API error ({}): {}", status, body),
            );
        }

        let states: Vec<Value> = match response.json().await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_entities",
                    format!("Failed to parse Home Assistant response: {}", e),
                );
            }
        };

        let result = filter_and_summarize(&states, domain, area);
        ToolResult::success("ha_list_entities", json!({ "result": result }))
    }

    // -----------------------------------------------------------------------
    // Action: get_state
    // -----------------------------------------------------------------------

    /// Fetch detailed state of a single entity from HA.
    ///
    /// Returns full state including all attributes, last_changed, and last_updated.
    async fn handle_get_state(&self, args: &Value) -> ToolResult {
        let entity_id = match args.get("entity_id").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => {
                return ToolResult::error("ha_get_state", "Missing required parameter: entity_id");
            }
        };

        // Validate entity_id format
        if !ENTITY_ID_RE.is_match(entity_id) {
            return ToolResult::error(
                "ha_get_state",
                format!(
                    "Invalid entity_id format: '{}'. Expected format: 'domain.entity' \
                     (e.g. 'light.living_room', 'climate.thermostat', 'sensor.temperature')",
                    entity_id
                ),
            );
        }

        let (hass_url, hass_token) = get_config();
        let url = format!("{}/api/states/{}", hass_url, entity_id);
        let headers = get_headers(&hass_token);

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "ha_get_state",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let response = match client.get(&url).headers(headers).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "ha_get_state",
                    format!(
                        "Network error when fetching state for '{}' from {}: {}",
                        entity_id, url, e
                    ),
                );
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return ToolResult::error(
                "ha_get_state",
                format!(
                    "Home Assistant API error ({}) for '{}': {}",
                    status, entity_id, body
                ),
            );
        }

        let data: Value = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                return ToolResult::error(
                    "ha_get_state",
                    format!("Failed to parse state response for '{}': {}", entity_id, e),
                );
            }
        };

        let result = json!({
            "entity_id": data.get("entity_id").and_then(|v| v.as_str()).unwrap_or(""),
            "state": data.get("state").and_then(|v| v.as_str()).unwrap_or(""),
            "attributes": data.get("attributes").unwrap_or(&json!({})),
            "last_changed": data.get("last_changed"),
            "last_updated": data.get("last_updated"),
        });

        ToolResult::success("ha_get_state", json!({ "result": result }))
    }

    // -----------------------------------------------------------------------
    // Action: list_services
    // -----------------------------------------------------------------------

    /// Fetch available services from HA and optionally filter by domain.
    ///
    /// Returns a compact structure with domain, service descriptions, and field descriptions.
    async fn handle_list_services(&self, args: &Value) -> ToolResult {
        let domain = args.get("domain").and_then(|v| v.as_str());

        let (hass_url, hass_token) = get_config();
        let url = format!("{}/api/services", hass_url);
        let headers = get_headers(&hass_token);

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_services",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let response = match client.get(&url).headers(headers).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_services",
                    format!("Network error when fetching services from {}: {}", url, e),
                );
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return ToolResult::error(
                "ha_list_services",
                format!("Home Assistant API error ({}): {}", status, body),
            );
        }

        let services: Vec<Value> = match response.json().await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::error(
                    "ha_list_services",
                    format!("Failed to parse services response: {}", e),
                );
            }
        };

        // Filter by domain if specified
        let filtered: Vec<&Value> = if let Some(d) = domain {
            services
                .iter()
                .filter(|s| s.get("domain").and_then(|v| v.as_str()) == Some(d))
                .collect()
        } else {
            services.iter().collect()
        };

        // Compact the output for context efficiency
        let compact_domains: Vec<Value> = filtered
            .iter()
            .map(|svc_domain| {
                let d = svc_domain
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let services_map = svc_domain
                    .get("services")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        let mut compact = serde_json::Map::new();
                        for (svc_name, svc_info) in m {
                            let mut entry = serde_json::Map::new();
                            entry.insert(
                                "description".to_string(),
                                Value::String(
                                    svc_info
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                ),
                            );

                            // Include field descriptions if present
                            if let Some(fields) = svc_info.get("fields").and_then(|v| v.as_object())
                            {
                                let field_descriptions: serde_json::Map<String, Value> = fields
                                    .iter()
                                    .filter_map(|(fk, fv)| {
                                        fv.as_object().map(|fv_obj| {
                                            (
                                                fk.clone(),
                                                Value::String(
                                                    fv_obj
                                                        .get("description")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("")
                                                        .to_string(),
                                                ),
                                            )
                                        })
                                    })
                                    .collect();

                                if !field_descriptions.is_empty() {
                                    entry.insert(
                                        "fields".to_string(),
                                        Value::Object(field_descriptions),
                                    );
                                }
                            }

                            compact.insert(svc_name.clone(), Value::Object(entry));
                        }
                        Value::Object(compact)
                    })
                    .unwrap_or(Value::Object(serde_json::Map::new()));

                json!({
                    "domain": d,
                    "services": services_map,
                })
            })
            .collect();

        let result = json!({
            "count": compact_domains.len(),
            "domains": compact_domains,
        });

        ToolResult::success("ha_list_services", json!({ "result": result }))
    }

    // -----------------------------------------------------------------------
    // Action: call_service
    // -----------------------------------------------------------------------

    /// Call a Home Assistant service.
    ///
    /// Validates domain, service, and entity_id formats; checks blocked domains;
    /// parses the `data` parameter from JSON string; then POSTs to HA.
    async fn handle_call_service(&self, args: &Value) -> ToolResult {
        let domain = match args.get("domain").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolResult::error("ha_call_service", "Missing required parameter: domain");
            }
        };

        let service = match args.get("service").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::error("ha_call_service", "Missing required parameter: service");
            }
        };

        // Validate domain format BEFORE blocklist check — prevents path traversal
        // in /api/services/{domain}/{service} and blocklist bypass via payloads
        // like "shell_command/../light".
        if !SERVICE_NAME_RE.is_match(domain) {
            return ToolResult::error(
                "ha_call_service",
                format!(
                    "Invalid domain format: '{}'. Domain must contain only \
                     lowercase ASCII letters, digits, and underscores (e.g. 'light', 'climate').",
                    domain
                ),
            );
        }

        if !SERVICE_NAME_RE.is_match(service) {
            return ToolResult::error(
                "ha_call_service",
                format!(
                    "Invalid service format: '{}'. Service must contain only \
                     lowercase ASCII letters, digits, and underscores (e.g. 'turn_on', 'set_temperature').",
                    service
                ),
            );
        }

        // Check blocked domains
        if BLOCKED_DOMAINS.contains(&domain) {
            return ToolResult::error(
                "ha_call_service",
                format!(
                    "Service domain '{}' is blocked for security reasons. \
                     Blocked domains: {}",
                    domain,
                    BLOCKED_DOMAINS.join(", ")
                ),
            );
        }

        // Validate entity_id if provided
        let entity_id = args.get("entity_id").and_then(|v| v.as_str());
        if let Some(eid) = entity_id
            && !ENTITY_ID_RE.is_match(eid)
        {
            return ToolResult::error(
                "ha_call_service",
                format!(
                    "Invalid entity_id format: '{}'. Expected format: 'domain.entity' \
                         (e.g. 'light.living_room').",
                    eid
                ),
            );
        }

        // Parse data from JSON string
        let data_str = args.get("data").and_then(|v| v.as_str());
        let data: Option<Value> = match data_str {
            Some(s) if !s.trim().is_empty() => match serde_json::from_str(s) {
                Ok(v) => Some(v),
                Err(e) => {
                    return ToolResult::error(
                        "ha_call_service",
                        format!("Invalid JSON string in 'data' parameter: {}", e),
                    );
                }
            },
            _ => None,
        };

        let (hass_url, hass_token) = get_config();
        let url = format!("{}/api/services/{}/{}", hass_url, domain, service);
        let headers = get_headers(&hass_token);
        let payload = build_service_payload(entity_id, data.as_ref());

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "ha_call_service",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let response = match client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "ha_call_service",
                    format!("Network error when calling {}.{}: {}", domain, service, e),
                );
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return ToolResult::error(
                "ha_call_service",
                format!(
                    "Home Assistant API error ({}) when calling {}.{}: {}",
                    status, domain, service, body
                ),
            );
        }

        let response_body: Value = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "ha_call_service",
                    format!(
                        "Failed to parse response from {}.{}: {}",
                        domain, service, e
                    ),
                );
            }
        };

        let result = parse_service_response(domain, service, &response_body);
        ToolResult::success("ha_call_service", json!({ "result": result }))
    }
}

impl Default for HomeAssistantTool {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// OperantTool trait implementation
// ===========================================================================

#[async_trait]
impl OperantTool for HomeAssistantTool {
    fn name(&self) -> &str {
        "homeassistant"
    }

    fn description(&self) -> &str {
        "Control Home Assistant smart home devices via REST API. \
         Supports: listing/filtering entities, getting detailed entity state, \
         listing available services per domain, and calling services to control \
         devices (turn on/off, set temperature, etc.). Requires HASS_TOKEN env var."
    }

    fn toolset(&self) -> &str {
        "smart_home"
    }

    fn is_available(&self) -> bool {
        std::env::var("HASS_TOKEN").is_ok() || std::env::var("HASS_URL").is_ok()
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<HomeAssistantArgs>(
            "homeassistant",
            "Control Home Assistant smart home devices via REST API. \
             Actions: list_entities (list/filter entities), get_state (get entity details), \
             list_services (list available services), call_service (call a service on an entity). \
             Requires HASS_TOKEN environment variable to be set.",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::error(
                    "homeassistant",
                    "Missing required parameter: action. Must be one of: \
                     list_entities, get_state, list_services, call_service",
                );
            }
        };

        match action {
            "list_entities" => self.handle_list_entities(&args).await,
            "get_state" => self.handle_get_state(&args).await,
            "list_services" => self.handle_list_services(&args).await,
            "call_service" => self.handle_call_service(&args).await,
            _ => ToolResult::error(
                "homeassistant",
                format!(
                    "Unknown action: '{}'. Must be one of: list_entities, get_state, \
                     list_services, call_service",
                    action
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =======================================================================
    // Schema & identity tests
    // =======================================================================

    #[test]
    fn test_name_and_description() {
        let tool = HomeAssistantTool::new();
        assert_eq!(tool.name(), "homeassistant");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema_has_all_properties() {
        let schema = HomeAssistantTool::new().schema();
        assert_eq!(schema.name, "homeassistant");

        let schema_json = serde_json::to_value(&schema).unwrap();
        let props = schema_json["parameters"]["properties"]
            .as_object()
            .expect("Schema should have 'properties' object");

        assert!(
            props.contains_key("action"),
            "Schema should have 'action' property"
        );
        assert!(
            props.contains_key("entityId"),
            "Schema should have 'entityId' property"
        );
        assert!(
            props.contains_key("domain"),
            "Schema should have 'domain' property"
        );
        assert!(
            props.contains_key("area"),
            "Schema should have 'area' property"
        );
        assert!(
            props.contains_key("service"),
            "Schema should have 'service' property"
        );
        assert!(
            props.contains_key("data"),
            "Schema should have 'data' property"
        );
    }

    #[test]
    fn test_schema_action_is_required() {
        let schema = HomeAssistantTool::new().schema();
        let schema_json = serde_json::to_value(&schema).unwrap();
        let required = schema_json["parameters"]["required"]
            .as_array()
            .expect("Schema should have 'required' array");
        assert!(
            required.iter().any(|v| v == "action"),
            "'action' should be in the required array"
        );
    }

    // =======================================================================
    // execute() dispatch tests
    // =======================================================================

    #[tokio::test]
    async fn test_missing_action_returns_error() {
        let tool = HomeAssistantTool::new();
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("action"));
    }

    #[tokio::test]
    async fn test_unknown_action_returns_error() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(json!({ "action": "nonexistent" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Unknown action"));
    }

    // =======================================================================
    // get_state validation tests
    // =======================================================================

    #[tokio::test]
    async fn test_get_state_missing_entity_id() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(json!({ "action": "get_state" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("entity_id"));
    }

    #[tokio::test]
    async fn test_get_state_invalid_entity_id() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "get_state", "entity_id": "INVALID!@#" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("entity_id") || err.contains("format"));
    }

    #[tokio::test]
    async fn test_get_state_empty_entity_id() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "get_state", "entity_id": "" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    // =======================================================================
    // call_service validation tests
    // =======================================================================

    #[tokio::test]
    async fn test_call_service_missing_domain() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "call_service", "service": "turn_on" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("domain"));
    }

    #[tokio::test]
    async fn test_call_service_missing_service() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "call_service", "domain": "light" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("service"));
    }

    #[tokio::test]
    async fn test_call_service_blocked_domain() {
        let tool = HomeAssistantTool::new();
        // Test each blocked domain
        for blocked in BLOCKED_DOMAINS {
            let result = tool
                .execute(
                    json!({ "action": "call_service", "domain": blocked, "service": "run" }),
                    ToolContext::default(),
                )
                .await;
            assert!(!result.success, "Domain '{}' should be blocked", blocked);
            let err = result.error.unwrap_or_default();
            assert!(
                err.contains("blocked"),
                "Blocked domain '{}' error should mention 'blocked': {}",
                blocked,
                err
            );
        }
    }

    #[tokio::test]
    async fn test_call_service_invalid_domain_path_traversal() {
        let tool = HomeAssistantTool::new();
        // These look like path-traversal attempts that might bypass the blocklist
        let attacks = &[
            "../../api/config",
            "shell_command/../light",
            "../config",
            "light/../../shell_command",
        ];
        for attack in attacks {
            let result = tool
                .execute(
                    json!({ "action": "call_service", "domain": attack, "service": "run" }),
                    ToolContext::default(),
                )
                .await;
            assert!(
                !result.success,
                "Path traversal '{}' should be rejected",
                attack
            );
        }
    }

    #[tokio::test]
    async fn test_call_service_invalid_domain_uppercase() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "call_service", "domain": "Light", "service": "turn_on" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success, "Uppercase domain should be rejected");
    }

    #[tokio::test]
    async fn test_call_service_invalid_domain_with_dot() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({ "action": "call_service", "domain": "light.test", "service": "turn_on" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success, "Domain with dot should be rejected");
    }

    #[tokio::test]
    async fn test_call_service_invalid_entity_id() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({
                    "action": "call_service",
                    "domain": "light",
                    "service": "turn_on",
                    "entity_id": "bad-entity!"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("entity_id") || err.contains("format"));
    }

    #[tokio::test]
    async fn test_call_service_invalid_data_json() {
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({
                    "action": "call_service",
                    "domain": "light",
                    "service": "turn_on",
                    "entity_id": "light.test",
                    "data": "not valid json {{{"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("JSON") || err.contains("data"));
    }

    #[tokio::test]
    async fn test_call_service_empty_data_is_ok() {
        // Empty data string should be treated as None (no error)
        let tool = HomeAssistantTool::new();
        let result = tool
            .execute(
                json!({
                    "action": "call_service",
                    "domain": "light",
                    "service": "turn_on",
                    "entity_id": "light.test",
                    "data": ""
                }),
                ToolContext::default(),
            )
            .await;
        // This will fail at network (no HA running) but not at validation
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        // The error should be network-related, not JSON-related
        assert!(
            !err.contains("JSON"),
            "Empty data should not cause JSON parse error: {}",
            err
        );
        assert!(
            !err.contains("Invalid"),
            "Empty data should not cause validation error: {}",
            err
        );
    }

    // =======================================================================
    // filter_and_summarize unit tests
    // =======================================================================

    #[test]
    fn test_filter_no_domain_no_area() {
        let states = vec![
            json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": { "friendly_name": "Living Room Light" }
            }),
            json!({
                "entity_id": "switch.kitchen",
                "state": "off",
                "attributes": { "friendly_name": "Kitchen Switch" }
            }),
            json!({
                "entity_id": "sensor.temperature",
                "state": "23.5",
                "attributes": { "friendly_name": "Temperature Sensor" }
            }),
        ];

        let result = filter_and_summarize(&states, None, None);
        assert_eq!(result["count"], 3);
        assert_eq!(result["entities"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_filter_by_domain() {
        let states = vec![
            json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": { "friendly_name": "Living Room Light" }
            }),
            json!({
                "entity_id": "light.kitchen",
                "state": "off",
                "attributes": { "friendly_name": "Kitchen Light" }
            }),
            json!({
                "entity_id": "switch.garage",
                "state": "on",
                "attributes": { "friendly_name": "Garage Switch" }
            }),
        ];

        let result = filter_and_summarize(&states, Some("light"), None);
        assert_eq!(result["count"], 2);
        assert_eq!(result["entities"][0]["entity_id"], "light.living_room");
        assert_eq!(result["entities"][1]["entity_id"], "light.kitchen");
    }

    #[test]
    fn test_filter_by_area_friendly_name() {
        let states = vec![
            json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": { "friendly_name": "Living Room Light" }
            }),
            json!({
                "entity_id": "light.kitchen",
                "state": "off",
                "attributes": { "friendly_name": "Kitchen Light" }
            }),
        ];

        let result = filter_and_summarize(&states, None, Some("kitchen"));
        assert_eq!(result["count"], 1);
        assert_eq!(result["entities"][0]["entity_id"], "light.kitchen");
    }

    #[test]
    fn test_filter_by_area_attribute() {
        let states = vec![
            json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": {
                    "friendly_name": "Living Room Light",
                    "area": "Living Room"
                }
            }),
            json!({
                "entity_id": "light.kitchen",
                "state": "off",
                "attributes": {
                    "friendly_name": "Kitchen Light",
                    "area": "Kitchen"
                }
            }),
        ];

        let result = filter_and_summarize(&states, None, Some("living"));
        assert_eq!(result["count"], 1);
        assert_eq!(result["entities"][0]["entity_id"], "light.living_room");
    }

    #[test]
    fn test_filter_by_domain_and_area() {
        let states = vec![
            json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": { "friendly_name": "Living Room Light", "area": "Living Room" }
            }),
            json!({
                "entity_id": "light.kitchen",
                "state": "off",
                "attributes": { "friendly_name": "Kitchen Light" }
            }),
            json!({
                "entity_id": "switch.kitchen",
                "state": "off",
                "attributes": { "friendly_name": "Kitchen Switch" }
            }),
        ];

        let result = filter_and_summarize(&states, Some("light"), Some("kitchen"));
        assert_eq!(result["count"], 1);
        assert_eq!(result["entities"][0]["entity_id"], "light.kitchen");
    }

    #[test]
    fn test_filter_no_matches() {
        let states = vec![json!({
            "entity_id": "light.living_room",
            "state": "on",
            "attributes": { "friendly_name": "Living Room Light" }
        })];

        let result = filter_and_summarize(&states, Some("climate"), None);
        assert_eq!(result["count"], 0);
        assert!(result["entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_filter_empty_states() {
        let states: Vec<Value> = vec![];
        let result = filter_and_summarize(&states, None, None);
        assert_eq!(result["count"], 0);
        assert!(result["entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_filter_missing_attributes() {
        let states = vec![json!({
            "entity_id": "light.living_room",
            "state": "on"
            // No attributes key at all
        })];

        // Should not panic, should return empty friendly_name
        let result = filter_and_summarize(&states, None, None);
        assert_eq!(result["count"], 1);
        assert_eq!(result["entities"][0]["friendly_name"], "");
    }

    #[test]
    fn test_filter_by_area_no_match_friendly_name() {
        let states = vec![json!({
            "entity_id": "light.living_room",
            "state": "on",
            "attributes": { "friendly_name": "LR Light" }
        })];

        // "living" should match "Living Room Light" but not "LR Light"
        let result = filter_and_summarize(&states, None, Some("living"));
        assert_eq!(result["count"], 0);
    }

    // =======================================================================
    // build_service_payload unit tests
    // =======================================================================

    #[test]
    fn test_build_payload_entity_id_only() {
        let payload = build_service_payload(Some("light.living_room"), None);
        assert_eq!(payload["entity_id"], "light.living_room");
        assert_eq!(payload.as_object().unwrap().len(), 1);
    }

    #[test]
    fn test_build_payload_data_only() {
        let data = json!({"brightness": 255, "color_name": "blue"});
        let payload = build_service_payload(None, Some(&data));
        assert_eq!(payload["brightness"], 255);
        assert_eq!(payload["color_name"], "blue");
        assert_eq!(payload.as_object().unwrap().len(), 2);
    }

    #[test]
    fn test_build_payload_entity_id_and_data() {
        let data = json!({"brightness": 100, "color_name": "warm_white"});
        let payload = build_service_payload(Some("light.test"), Some(&data));
        assert_eq!(payload["brightness"], 100);
        assert_eq!(payload["color_name"], "warm_white");
        assert_eq!(payload["entity_id"], "light.test");
    }

    #[test]
    fn test_build_payload_entity_id_overrides_data() {
        // entity_id parameter should take precedence over data["entity_id"]
        let data = json!({"entity_id": "light.from_data", "brightness": 200});
        let payload = build_service_payload(Some("light.from_param"), Some(&data));
        assert_eq!(payload["entity_id"], "light.from_param");
        assert_eq!(payload["brightness"], 200);
    }

    #[test]
    fn test_build_payload_empty() {
        let payload = build_service_payload(None, None);
        assert!(payload.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_build_payload_data_not_object() {
        // If data is not an object (e.g. string), it should be ignored
        let data = json!("just a string");
        let payload = build_service_payload(Some("light.test"), Some(&data));
        assert_eq!(payload["entity_id"], "light.test");
        assert_eq!(payload.as_object().unwrap().len(), 1);
    }

    // =======================================================================
    // parse_service_response unit tests
    // =======================================================================

    #[test]
    fn test_parse_service_response_with_results() {
        let ha_result = json!([
            { "entity_id": "light.living_room", "state": "on" },
            { "entity_id": "light.kitchen", "state": "on" },
        ]);
        let parsed = parse_service_response("light", "turn_on", &ha_result);
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["service"], "light.turn_on");
        assert_eq!(parsed["affected_entities"].as_array().unwrap().len(), 2);
        assert_eq!(
            parsed["affected_entities"][0]["entity_id"],
            "light.living_room"
        );
        assert_eq!(parsed["affected_entities"][0]["state"], "on");
    }

    #[test]
    fn test_parse_service_response_empty_array() {
        let ha_result = json!([]);
        let parsed = parse_service_response("climate", "set_temperature", &ha_result);
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["service"], "climate.set_temperature");
        assert!(parsed["affected_entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_parse_service_response_not_array() {
        let ha_result = json!({"success": true});
        let parsed = parse_service_response("scene", "turn_on", &ha_result);
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["service"], "scene.turn_on");
        assert!(parsed["affected_entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_parse_service_response_missing_fields() {
        let ha_result = json!([
            { "entity_id": "light.living_room" },
            { "state": "on" },
        ]);
        let parsed = parse_service_response("light", "toggle", &ha_result);
        assert_eq!(
            parsed["affected_entities"][0]["entity_id"],
            "light.living_room"
        );
        assert_eq!(parsed["affected_entities"][0]["state"], "");
        assert_eq!(parsed["affected_entities"][1]["entity_id"], "");
        assert_eq!(parsed["affected_entities"][1]["state"], "on");
    }

    // =======================================================================
    // Regex validation tests
    // =======================================================================

    #[test]
    fn test_entity_id_valid_examples() {
        let valid = vec![
            "light.living_room",
            "sensor.temperature_1",
            "climate.thermostat",
            "binary_sensor.door_sensor",
            "media_player.living_room_tv",
            "cover.garage_door",
            "fan.ceiling_fan",
            "switch.outside_1",
            "lock.front_door",
            "_test.valid",
        ];
        for eid in valid {
            assert!(
                ENTITY_ID_RE.is_match(eid),
                "Entity ID '{}' should be valid",
                eid
            );
        }
    }

    #[test]
    fn test_entity_id_invalid_examples() {
        let invalid = vec![
            "",
            "light",
            ".light",
            "light.",
            "Light.LivingRoom",
            "light.living room",
            "light.living-room",
            "123.light",
            "../api/config",
            "light/../config",
        ];
        for eid in invalid {
            assert!(
                !ENTITY_ID_RE.is_match(eid),
                "Entity ID '{}' should be invalid",
                eid
            );
        }
    }

    #[test]
    fn test_entity_id_valid_digits_after_dot() {
        assert!(ENTITY_ID_RE.is_match("sensor.123"));
        assert!(ENTITY_ID_RE.is_match("sensor.123abc"));
    }

    #[test]
    fn test_service_name_valid_examples() {
        let valid = vec![
            "light",
            "turn_on",
            "set_temperature",
            "shell_command",
            "python_script",
            "a",
            "z_1",
        ];
        for name in valid {
            assert!(
                SERVICE_NAME_RE.is_match(name),
                "Service name '{}' should be valid",
                name
            );
        }
    }

    #[test]
    fn test_service_name_invalid_examples() {
        let invalid = vec![
            "",
            "Light",
            "light.service",
            "light/service",
            "light service",
            "1light",
            "../config",
            "../../api",
        ];
        for name in invalid {
            assert!(
                !SERVICE_NAME_RE.is_match(name),
                "Service name '{}' should be invalid",
                name
            );
        }
    }

    // =======================================================================
    // get_config unit tests
    // =======================================================================

    #[test]
    fn test_get_config_defaults_when_env_unset() {
        let prev_url = std::env::var("HASS_URL").ok();
        let prev_token = std::env::var("HASS_TOKEN").ok();
        // SAFETY: test-only env mutation in #[cfg(test)]
        unsafe { std::env::remove_var("HASS_URL") };
        // SAFETY: test-only env mutation in #[cfg(test)]
        unsafe { std::env::remove_var("HASS_TOKEN") };

        let (url, token) = get_config();
        assert_eq!(url, "http://homeassistant.local:8123");
        assert_eq!(token, "");

        if let Some(u) = prev_url {
            unsafe { std::env::set_var("HASS_URL", u) };
        }
        if let Some(t) = prev_token {
            unsafe { std::env::set_var("HASS_TOKEN", t) };
        }
    }

    #[test]
    fn test_get_config_strips_trailing_slash() {
        let prev_url = std::env::var("HASS_URL").ok();
        unsafe { std::env::set_var("HASS_URL", "http://homeassistant.local:8123/") };

        let (url, _) = get_config();
        assert!(
            !url.ends_with('/'),
            "URL should not have trailing slash: {}",
            url
        );

        if let Some(u) = prev_url {
            unsafe { std::env::set_var("HASS_URL", u) };
        } else {
            unsafe { std::env::remove_var("HASS_URL") };
        }
    }

    // =======================================================================
    // get_headers unit tests
    // =======================================================================

    #[test]
    fn test_get_headers_has_auth() {
        let headers = get_headers("test_token_123");
        let auth = headers
            .get("Authorization")
            .expect("Should have Authorization header");
        assert_eq!(auth.to_str().unwrap(), "Bearer test_token_123");
    }

    #[test]
    fn test_get_headers_has_content_type() {
        let headers = get_headers("token");
        let ct = headers
            .get("Content-Type")
            .expect("Should have Content-Type header");
        assert_eq!(ct.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_get_headers_empty_token() {
        let headers = get_headers("");
        let auth = headers
            .get("Authorization")
            .expect("Should have Authorization header even with empty token");
        assert_eq!(auth.to_str().unwrap(), "Bearer ");
    }
}
