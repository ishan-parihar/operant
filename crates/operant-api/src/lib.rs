//! Operant API layer — trait definitions and shared types.
//!
//! This crate defines the fundamental abstractions that all Operant subsystems
//! depend on. No implementations, no heavy dependencies. Every other crate in
//! the workspace depends on this. The compiler enforces that no implementation
//! crate can import another without going through these interfaces.
//!
//! ## Traits
//! - [`provider::Provider`] — LLM inference backends
//! - [`channel::Channel`] — messaging platform integrations
//! - [`tool::Tool`] — agent-callable capabilities
//! - [`memory_traits::Memory`] — conversation memory backends
//! - [`observability_traits::Observer`] — metrics and tracing
//! - [`runtime_traits::RuntimeAdapter`] — execution environment adapters
//! - [`peripherals_traits::Peripheral`] — hardware board integrations

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
pub mod agent;
pub mod channel;
pub mod glob;
pub mod media;
pub mod memory_traits;
pub mod observability_traits;
pub mod peripherals_traits;
pub mod provider;
pub mod runtime_traits;
pub mod schema;
pub mod session_keys;
pub mod tool;
pub mod vad;
