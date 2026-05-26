//! OpalZero Kernel — proprietary reasoning layer.
//!
//! This crate contains the high-performance implementations that differentiate
//! OpalZero from generic agent frameworks:
//!
//! - [`engine::OpenAIProvider`]  — hardened GPT-4o / GPT-4o-mini provider.
//! - [`governor::OpalZeroGovernor`] — full Auditor with rich quality prompts and
//!   per-role system prompt engineering.
//!
//! Downstream binaries (`opalzero-core/main.rs`, `opalzero-server`) depend on this
//! crate.  The open-source `opalzero-core` library does NOT depend on it, keeping
//! the public API clean of any proprietary implementation detail.

pub mod claude;
pub mod engine;
pub mod governor;

/// Convenience re-exports for binary entry-points.
pub mod prelude {
    pub use crate::claude::ClaudeProvider;
    pub use crate::engine::OpenAIProvider;
    pub use crate::governor::OpalZeroGovernor;
}
