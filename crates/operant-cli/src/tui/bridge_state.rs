// bridge_state.rs — Bridge connection state and status badge rendering.
//
// Mirrors upstream bridge_state.rs with Operant-specific styling.

/// The current state of the remote gateway/bridge connection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BridgeConnectionState {
    /// No bridge configured / not in use.
    #[default]
    Disconnected,
    /// Currently attempting to connect.
    Connecting,
    /// Successfully connected with session info.
    Connected {
        session_url: String,
        peer_count: u32,
    },
    /// Lost connection and retrying.
    Reconnecting { attempt: u32 },
    /// Connection failed unrecoverably.
    Failed { reason: String },
    /// Outbound-only mode (no incoming peers).
    OutboundOnly,
}

impl BridgeConnectionState {}

#[cfg(test)]
mod tests {}
