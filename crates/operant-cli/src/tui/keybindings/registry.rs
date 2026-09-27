// keybindings/registry.rs — Registry lookup/manipulation methods.
//
// Extracted from the keybindings.rs monolith. Construction, registration,
// lookup (with context fallback), matching, and removal.

use super::*;

impl KeyBindingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry with default keybindings
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.add_defaults();
        registry
    }

    /// Add a binding
    pub fn add(&mut self, binding: KeyBinding) {
        if let Some(ctx) = binding.context {
            self.bindings.entry(ctx).or_default().push(binding);
        } else {
            self.global_bindings.push(binding);
        }
    }

    /// Add multiple bindings
    #[allow(dead_code)] // Batched-registration API; no production caller yet
    pub fn add_many(&mut self, bindings: Vec<KeyBinding>) {
        for b in bindings {
            self.add(b);
        }
    }

    /// Find binding for a key event in a given context
    pub fn find(&self, event: &KeyEvent, context: BindingContext) -> Option<&KeyBinding> {
        // Check context-specific bindings first
        if let Some(ctx_bindings) = self.bindings.get(&context) {
            for binding in ctx_bindings {
                if self.matches(binding, event) {
                    return Some(binding);
                }
            }
        }

        // Check global bindings
        self.global_bindings
            .iter()
            .find(|&binding| self.matches(binding, event))
            .map(|v| v as _)
    }

    /// Find binding for a key event, trying multiple contexts in priority order
    #[allow(dead_code)] // Multi-context dispatch lands with the dispatch migration
    pub fn find_with_fallback(
        &self,
        event: &KeyEvent,
        contexts: &[BindingContext],
    ) -> Option<&KeyBinding> {
        for ctx in contexts {
            if let Some(binding) = self.find(event, *ctx) {
                return Some(binding);
            }
        }
        None
    }

    /// Check if a key event matches a binding
    fn matches(&self, binding: &KeyBinding, event: &KeyEvent) -> bool {
        binding.key == event.code && binding.modifiers == event.modifiers
    }

    /// Get all bindings for a context (for help display)
    #[allow(dead_code)] // Superseded by `catalogue`, which lists each binding once
    pub fn get_bindings(&self, context: BindingContext) -> Vec<&KeyBinding> {
        let mut result = Vec::new();
        if let Some(ctx_bindings) = self.bindings.get(&context) {
            result.extend(ctx_bindings);
        }
        result.extend(&self.global_bindings);
        result
    }

    /// Remove a binding
    #[allow(dead_code)] // Custom-binding API; no production caller yet
    pub fn remove(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        context: Option<BindingContext>,
    ) -> bool {
        if let Some(ctx) = context {
            if let Some(bindings) = self.bindings.get_mut(&ctx) {
                let len_before = bindings.len();
                bindings.retain(|b| b.key != key || b.modifiers != modifiers);
                return bindings.len() < len_before;
            }
        } else {
            let len_before = self.global_bindings.len();
            self.global_bindings
                .retain(|b| b.key != key || b.modifiers != modifiers);
            return self.global_bindings.len() < len_before;
        }
        false
    }
}
