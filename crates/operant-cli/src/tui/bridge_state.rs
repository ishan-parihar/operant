// bridge_state.rs — Bridge connection state.
//
// NOTE: The bridge feature is not yet wired end-to-end. The App always
// initializes `bridge_state` to `Disconnected` and no code path ever sets
// any other variant. The render.rs branches that match on this enum are
// therefore dead. When the bridge feature is wired up, restore the
// Connecting / Connected / Reconnecting / Failed / OutboundOnly variants
// and the corresponding render branches from git history.

/// The current state of the remote bridge connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BridgeConnectionState {
    /// No bridge configured / not in use.
    #[default]
    Disconnected,
}
