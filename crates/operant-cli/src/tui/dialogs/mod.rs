// dialogs/mod.rs — Permission dialogs and confirmation dialogs.
//
// Decomposed from the dialogs.rs monolith:
//   - permission.rs   — Tool permission request dialogs (PermissionDialogKind
//                       + PermissionRequest + render_permission_dialog)
//   - mcp_approval.rs — MCP server approval dialog (McpApprovalDialogState +
//                       render_mcp_approval_dialog + key handling)
//   - tests.rs        — Unit tests

pub(crate) mod mcp_approval;
pub(crate) mod permission;

#[cfg(test)]
mod tests;

pub(crate) use mcp_approval::*;
pub(crate) use permission::*;
