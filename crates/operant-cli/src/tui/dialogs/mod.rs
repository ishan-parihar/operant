// dialogs/mod.rs — Permission dialogs and confirmation dialogs.
//
// Decomposed from the dialogs.rs monolith:
//   - permission.rs   — Tool permission request dialogs (PermissionDialogKind
//                       + PermissionRequest + render_permission_dialog)
//   - tests.rs        — Unit tests

pub(crate) mod permission;

#[cfg(test)]
mod tests;

pub(crate) use permission::*;
