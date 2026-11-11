//! Editor/pane integration installers — `operant doctor --fix` runs these.
//!
//! Two integrations live here, and both are the same shape: the assets travel
//! inside the binary, and an install writes them into the places the OTHER
//! program actually reads.
//!
//! * **Herdr** (`~/.config/herdr/plugins/`): a plugin manifest plus its
//!   scripts, registered with `herdr plugin link`. Herdr's plugin model is the
//!   only supported way for a third party to add panes and actions to its UI.
//! * **Omarchy** (`~/.local/bin/`, `~/.config/omarchy/`): user-level
//!   wrappers in front of the package-owned `/usr/bin/omarchy-*` scripts, plus
//!   the menu row that makes the agent selectable.
//!
//! WHY THE OPERANT BINARY OWNS THESE
//!
//! The alternative is a loose shell script that drifts from the binary the
//! moment either changes. Here the assets are compiled in, so the installed
//! copy is always the one that shipped with the running Operant, and
//! `operant doctor` can compare what is installed against what it would
//! install and say exactly which file is stale.
//!
//! NEITHER INTEGRATION IS REQUIRED
//!
//! Both are no-ops when the other program is absent. A machine without Herdr
//! or without Omarchy is a supported machine; doctor reports the integration
//! as unavailable (an advisory) rather than as a fault.

pub mod herdr_plugin;
pub mod omarchy_agent;
