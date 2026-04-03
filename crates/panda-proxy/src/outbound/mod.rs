//! **Outbound pillar — AI gateway**
//!
//! OpenAI-compatible traffic to upstream LLMs: HTTP client join, SSE, provider adapters,
//! semantic cache, semantic upstream routing, and model failover chains.
//!
//! See `docs/architecture_two_pillars.md`.

pub mod adapter;
pub mod adapter_stream;
pub mod model_failover;
pub mod semantic_cache;
pub mod semantic_routing;
pub mod sse;
pub mod upstream;
