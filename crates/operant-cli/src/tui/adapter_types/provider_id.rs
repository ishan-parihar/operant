// adapter_types/provider_id.rs — Provider identifier enum.

pub enum ProviderId {
    OpencodeGo,
    OpencodeZen,
    Other(String),
}

impl From<ProviderId> for String {
    fn from(pid: ProviderId) -> String {
        match pid {
            ProviderId::OpencodeGo => "opencode-go".to_string(),
            ProviderId::OpencodeZen => "opencode-zen".to_string(),
            ProviderId::Other(s) => s,
        }
    }
}

impl<'a> From<&'a ProviderId> for String {
    fn from(pid: &'a ProviderId) -> String {
        match pid {
            ProviderId::OpencodeGo => "opencode-go".to_string(),
            ProviderId::OpencodeZen => "opencode-zen".to_string(),
            ProviderId::Other(s) => s.clone(),
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderId::OpencodeGo => write!(f, "opencode-go"),
            ProviderId::OpencodeZen => write!(f, "opencode-zen"),
            ProviderId::Other(s) => write!(f, "{}", s),
        }
    }
}

impl std::str::FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencode-go" => Ok(ProviderId::OpencodeGo),
            "opencode-zen" => Ok(ProviderId::OpencodeZen),
            other => Ok(ProviderId::Other(other.to_string())),
        }
    }
}

// (iter-208: pub mod mcp { ... } deleted — stub McpManager that returned
// empty data for /mcp overlay. load_mcp_servers now reads from
// App.core_mcp_manager (the real operant_core::mcp::McpManager).
// McpServerStatus/McpCatalogEntry/McpToolDef were never used outside
// the stub itself.)
//
// (iter-155: pub mod streaming {} deleted — was empty, only a deletion marker.)


