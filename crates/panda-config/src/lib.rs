//! GitOps-style configuration: parse and validate YAML without starting the server.
//!
//! Kept separate from the proxy so unit tests can cover invalid files and defaults
//! without binding to a network stack.

use std::path::Path;

use http::header::HeaderName;
use serde::Deserialize;

fn format_listen_address(addr: &str, port: u16) -> String {
    if addr.contains(':') && !addr.starts_with('[') {
        format!("[{addr}]:{port}")
    } else {
        format!("{addr}:{port}")
    }
}

/// Optional “Kong handshake”: attested edge injects identity headers; Panda verifies before trusting them.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TrustedGatewayConfig {
    /// Header the edge sets to a shared secret (must match env `PANDA_TRUSTED_GATEWAY_SECRET`).
    #[serde(default)]
    pub attestation_header: Option<String>,
    /// End-user subject after OIDC at the edge (e.g. `X-User-Id`).
    #[serde(default)]
    pub subject_header: Option<String>,
    #[serde(default)]
    pub tenant_header: Option<String>,
    /// Space- or comma-separated scopes (e.g. `X-User-Scopes`).
    #[serde(default)]
    pub scopes_header: Option<String>,
}

impl TrustedGatewayConfig {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        for name in [
            self.attestation_header.as_deref(),
            self.subject_header.as_deref(),
            self.tenant_header.as_deref(),
            self.scopes_header.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if name.trim().is_empty() {
                anyhow::bail!("trusted_gateway header names must be non-empty when set");
            }
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("invalid trusted_gateway header name: {name:?}"))?;
        }
        Ok(())
    }
}

/// EU AI Act–style audit export (stub: local signed JSONL; S3/GCS in `docs/compliance_export.md`).
#[derive(Debug, Clone, Deserialize)]
pub struct ComplianceExportConfig {
    #[serde(default)]
    pub enabled: bool,
    /// `off` | `local_jsonl`. Object-store modes are design-only until wired.
    #[serde(default = "default_compliance_export_mode")]
    pub mode: String,
    /// Directory for `local_jsonl` daily files (`panda-compliance-YYYYMMDD.jsonl`).
    #[serde(default)]
    pub local_path: String,
    /// Optional env var holding shared secret for HMAC-SHA256 over each record (hex in output).
    #[serde(default)]
    pub signing_secret_env: Option<String>,
}

fn default_compliance_export_mode() -> String {
    "off".to_string()
}

impl Default for ComplianceExportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_compliance_export_mode(),
            local_path: String::new(),
            signing_secret_env: None,
        }
    }
}

/// W3C `traceparent` and generated IDs (see `ensure_correlation_id` in proxy).
#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConfig {
    #[serde(default = "default_correlation_header")]
    pub correlation_header: String,
    /// Header name used for optional ops endpoint authentication.
    #[serde(default = "default_admin_auth_header")]
    pub admin_auth_header: String,
    /// Optional env var name for shared secret protecting ops endpoints.
    #[serde(default)]
    pub admin_secret_env: Option<String>,
    #[serde(default)]
    pub compliance_export: ComplianceExportConfig,
}

fn default_correlation_header() -> String {
    "x-request-id".to_string()
}

fn default_admin_auth_header() -> String {
    "x-panda-admin-secret".to_string()
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            correlation_header: default_correlation_header(),
            admin_auth_header: default_admin_auth_header(),
            admin_secret_env: None,
            compliance_export: ComplianceExportConfig::default(),
        }
    }
}

/// TPM backends: optional Redis for multi-instance totals (`PANDA_REDIS_URL` overrides `redis_url`).
#[derive(Debug, Clone, Deserialize)]
pub struct TpmConfig {
    #[serde(default)]
    pub redis_url: Option<String>,
    /// If true, enforce token budgets per identity bucket.
    #[serde(default)]
    pub enforce_budget: bool,
    /// Max prompt-token estimate per rolling minute per bucket.
    #[serde(default = "default_tpm_budget_tokens_per_minute")]
    pub budget_tokens_per_minute: u64,
    /// Optional fixed `Retry-After` (seconds) for 429 responses; when unset, derive from window.
    #[serde(default)]
    pub retry_after_seconds: Option<u64>,
    /// When Redis is configured but unreachable at startup, treat TPM as **degraded** (stricter cap).
    #[serde(default = "default_true")]
    pub redis_unavailable_degraded_limits: bool,
    /// When a Redis TPM command fails at runtime, enter degraded mode until a command succeeds again.
    #[serde(default = "default_true")]
    pub redis_command_error_degraded_limits: bool,
    /// Effective budget multiplier while degraded (e.g. `0.5` ≈ 50% “safe mode”).
    #[serde(default = "default_tpm_degraded_limit_ratio")]
    pub redis_degraded_limit_ratio: f64,
}

fn default_tpm_budget_tokens_per_minute() -> u64 {
    60_000
}

fn default_tpm_degraded_limit_ratio() -> f64 {
    0.5
}

fn default_true() -> bool {
    true
}

impl Default for TpmConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            enforce_budget: false,
            budget_tokens_per_minute: default_tpm_budget_tokens_per_minute(),
            retry_after_seconds: None,
            redis_unavailable_degraded_limits: true,
            redis_command_error_degraded_limits: true,
            redis_degraded_limit_ratio: default_tpm_degraded_limit_ratio(),
        }
    }
}

/// Wasm plugins: load `*.wasm` from `directory` (Phase 2).
#[derive(Debug, Clone, Deserialize)]
pub struct PluginsConfig {
    /// Directory containing `.wasm` guest modules (must exist when set).
    #[serde(default)]
    pub directory: Option<String>,
    /// Max request body bytes buffered for Wasm body hooks.
    #[serde(default = "default_wasm_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
    /// Best-effort per-hook timeout in milliseconds.
    #[serde(default = "default_wasm_execution_timeout_ms")]
    pub execution_timeout_ms: u64,
    /// If true, plugin failures reject request path; if false, fail open and continue.
    #[serde(default)]
    pub fail_closed: bool,
    /// If true, watch plugin directory and hot-swap runtime on changes.
    #[serde(default)]
    pub hot_reload: bool,
    /// Poll interval for hot-reload scans (milliseconds).
    #[serde(default = "default_wasm_reload_interval_ms")]
    pub reload_interval_ms: u64,
    /// Debounce window for filesystem change storms (milliseconds).
    #[serde(default = "default_wasm_reload_debounce_ms")]
    pub reload_debounce_ms: u64,
    /// Max successful hot reloads allowed per rolling minute.
    #[serde(default = "default_wasm_max_reloads_per_minute")]
    pub max_reloads_per_minute: u64,
}

fn default_wasm_max_request_body_bytes() -> usize {
    256 * 1024
}

fn default_wasm_execution_timeout_ms() -> u64 {
    25
}

fn default_wasm_reload_interval_ms() -> u64 {
    2000
}

fn default_wasm_reload_debounce_ms() -> u64 {
    500
}

fn default_wasm_max_reloads_per_minute() -> u64 {
    30
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            directory: None,
            max_request_body_bytes: default_wasm_max_request_body_bytes(),
            execution_timeout_ms: default_wasm_execution_timeout_ms(),
            fail_closed: false,
            hot_reload: false,
            reload_interval_ms: default_wasm_reload_interval_ms(),
            reload_debounce_ms: default_wasm_reload_debounce_ms(),
            max_reloads_per_minute: default_wasm_max_reloads_per_minute(),
        }
    }
}

/// Unified edge auth block (merged into [`IdentityConfig`] at load time).
///
/// Use this for JWKS-backed verification; legacy HS256 remains under `identity`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthConfig {
    /// Optional marker (e.g. `"jwt"`); reserved for future auth kinds.
    #[serde(default, rename = "type")]
    pub auth_type: Option<String>,
    /// When set, RSA tokens (`RS256`/`RS384`/`RS512`) are verified using keys from this JWKS URL.
    #[serde(default)]
    pub jwks_url: Option<String>,
    /// When true, sets [`IdentityConfig::require_jwt`] so every proxied route requires a bearer JWT.
    #[serde(default)]
    pub enforce_on_all_routes: bool,
}

/// Optional identity controls for Phase 3 entry.
#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    /// If true, require a bearer JWT on proxied requests.
    #[serde(default)]
    pub require_jwt: bool,
    /// Env var name containing HS256 secret (default: PANDA_JWT_HS256_SECRET).
    #[serde(default = "default_jwt_hs256_secret_env")]
    pub jwt_hs256_secret_env: String,
    /// When set, asymmetric JWTs are verified with keys loaded from this JWKS document (HTTPS GET).
    #[serde(default)]
    pub jwks_url: Option<String>,
    /// How long to cache a fetched JWKS before refresh (seconds).
    #[serde(default = "default_jwks_cache_ttl_seconds")]
    pub jwks_cache_ttl_seconds: u64,
    /// Expected issuer (`iss`) values. Empty means issuer is not checked.
    #[serde(default)]
    pub accepted_issuers: Vec<String>,
    /// Expected audience (`aud`) values. Empty means audience is not checked.
    #[serde(default)]
    pub accepted_audiences: Vec<String>,
    /// Required scopes (all must be present). Empty means no scope check.
    #[serde(default)]
    pub required_scopes: Vec<String>,
    /// Route-specific scope requirements (matched by path prefix).
    #[serde(default)]
    pub route_scope_rules: Vec<RouteScopeRule>,
    /// If true, mint a scoped agent token and forward in `x-panda-agent-token`.
    #[serde(default)]
    pub enable_token_exchange: bool,
    /// Env var name containing HS256 secret for minted agent tokens.
    #[serde(default = "default_agent_token_secret_env")]
    pub agent_token_secret_env: String,
    /// Lifetime for minted agent tokens.
    #[serde(default = "default_agent_token_ttl_seconds")]
    pub agent_token_ttl_seconds: u64,
    /// Scopes to embed in minted agent tokens (space-delimited in `scope` claim).
    #[serde(default)]
    pub agent_token_scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RouteScopeRule {
    pub path_prefix: String,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            require_jwt: false,
            jwt_hs256_secret_env: default_jwt_hs256_secret_env(),
            jwks_url: None,
            jwks_cache_ttl_seconds: default_jwks_cache_ttl_seconds(),
            accepted_issuers: vec![],
            accepted_audiences: vec![],
            required_scopes: vec![],
            route_scope_rules: vec![],
            enable_token_exchange: false,
            agent_token_secret_env: default_agent_token_secret_env(),
            agent_token_ttl_seconds: default_agent_token_ttl_seconds(),
            agent_token_scopes: vec![],
        }
    }
}

fn default_jwks_cache_ttl_seconds() -> u64 {
    3600
}

fn default_jwt_hs256_secret_env() -> String {
    "PANDA_JWT_HS256_SECRET".to_string()
}

fn default_agent_token_secret_env() -> String {
    "PANDA_AGENT_TOKEN_HS256_SECRET".to_string()
}

fn default_agent_token_ttl_seconds() -> u64 {
    300
}

/// Starter prompt-safety controls (Phase 3).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PromptSafetyConfig {
    /// If true, run deny-pattern scan on request path/query/body.
    #[serde(default)]
    pub enabled: bool,
    /// If true, log would-block events but do not reject traffic.
    #[serde(default)]
    pub shadow_mode: bool,
    /// Case-insensitive literal patterns that trigger 403 when found.
    #[serde(default)]
    pub deny_patterns: Vec<String>,
}

/// Starter PII scrubbing controls (Phase 3).
#[derive(Debug, Clone, Deserialize)]
pub struct PiiConfig {
    /// If true, redact request body using regex patterns.
    #[serde(default)]
    pub enabled: bool,
    /// If true, detect/log redaction candidates but do not mutate request body.
    #[serde(default)]
    pub shadow_mode: bool,
    /// Regex patterns to redact from buffered request body.
    #[serde(default)]
    pub redact_patterns: Vec<String>,
    /// Replacement text used for each matched PII region.
    #[serde(default = "default_pii_replacement")]
    pub replacement: String,
}

fn default_pii_replacement() -> String {
    "[REDACTED]".to_string()
}

impl Default for PiiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            shadow_mode: false,
            redact_patterns: vec![],
            replacement: default_pii_replacement(),
        }
    }
}

/// Phase 4: semantic cache controls (MVP in-memory backend).
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticCacheConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Backend kind: `memory` (default) or `redis` (Redis-compatible; works with Dragonfly).
    #[serde(default = "default_semantic_cache_backend")]
    pub backend: String,
    /// Optional Redis URL for `backend=redis`; env `PANDA_SEMANTIC_CACHE_REDIS_URL` also supported.
    #[serde(default)]
    pub redis_url: Option<String>,
    /// Similarity threshold placeholder for future embedding/vector backend.
    #[serde(default = "default_semantic_cache_similarity_threshold")]
    pub similarity_threshold: f32,
    /// Max number of cached responses kept in-memory.
    #[serde(default = "default_semantic_cache_max_entries")]
    pub max_entries: usize,
    /// Time-to-live for cache entries in seconds.
    #[serde(default = "default_semantic_cache_ttl_seconds")]
    pub ttl_seconds: u64,
}

fn default_semantic_cache_similarity_threshold() -> f32 {
    0.92
}

fn default_semantic_cache_backend() -> String {
    "memory".to_string()
}

fn default_semantic_cache_max_entries() -> usize {
    10_000
}

fn default_semantic_cache_ttl_seconds() -> u64 {
    300
}

impl Default for SemanticCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_semantic_cache_backend(),
            redis_url: None,
            similarity_threshold: default_semantic_cache_similarity_threshold(),
            max_entries: default_semantic_cache_max_entries(),
            ttl_seconds: default_semantic_cache_ttl_seconds(),
        }
    }
}

/// Phase 4: universal adapter target provider.
#[derive(Debug, Clone, Deserialize)]
pub struct AdapterConfig {
    /// Backend provider protocol to target from OpenAI-compatible ingress.
    /// Supported: `openai` (passthrough), `anthropic` (request/response mapping for non-streaming chat).
    #[serde(default = "default_adapter_provider")]
    pub provider: String,
    /// Anthropic API version header when provider is `anthropic`.
    #[serde(default = "default_adapter_anthropic_version")]
    pub anthropic_version: String,
}

fn default_adapter_provider() -> String {
    "openai".to_string()
}

fn default_adapter_anthropic_version() -> String {
    "2023-06-01".to_string()
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            provider: default_adapter_provider(),
            anthropic_version: default_adapter_anthropic_version(),
        }
    }
}

/// Phase 4: MCP tool servers (stub wiring; real transports added incrementally).
#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    /// When true, load MCP server entries; the proxy runtime is built in the `panda-proxy` crate.
    #[serde(default)]
    pub enabled: bool,
    /// If true, MCP client errors do not fail the overall request path (until chat/tool loop is wired).
    #[serde(default = "default_mcp_fail_open")]
    pub fail_open: bool,
    #[serde(default = "default_mcp_tool_timeout_ms")]
    pub tool_timeout_ms: u64,
    #[serde(default = "default_mcp_max_tool_payload_bytes")]
    pub max_tool_payload_bytes: usize,
    /// Maximum number of non-streaming tool-call follow-up rounds per request.
    #[serde(default = "default_mcp_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// Streaming MCP first-round probe budget in bytes before passthrough fallback.
    #[serde(default = "default_mcp_stream_probe_bytes")]
    pub stream_probe_bytes: usize,
    /// Rolling window size in seconds for MCP probe status snapshots.
    #[serde(default = "default_mcp_probe_window_seconds")]
    pub probe_window_seconds: u64,
    /// When true, discovered tools may be advertised to the model (OpenAI-style `tools` field).
    #[serde(default)]
    pub advertise_tools: bool,
    /// Proof-of-intent mode for tool calls: `off`, `audit`, or `enforce`.
    #[serde(default = "default_mcp_proof_of_intent_mode")]
    pub proof_of_intent_mode: String,
    /// Optional allowlist rules: which tool names are allowed for each intent label.
    #[serde(default)]
    pub intent_tool_policies: Vec<McpIntentToolPolicy>,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// Optional human-in-the-loop gate before selected MCP tools run (HTTP approval callback).
    #[serde(default)]
    pub hitl: McpHitlConfig,
}

/// Pause high-risk MCP tool calls until an external approval endpoint returns success.
#[derive(Debug, Clone, Deserialize)]
pub struct McpHitlConfig {
    #[serde(default)]
    pub enabled: bool,
    /// HTTPS URL that accepts POST JSON and returns `{"approved": true}` or `{"status":"approved"}`.
    #[serde(default)]
    pub approval_url: String,
    #[serde(default = "default_mcp_hitl_timeout_ms")]
    pub timeout_ms: u64,
    /// When set, send `Authorization: Bearer $TOKEN` using this env var (optional).
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    /// Tool keys to gate: OpenAI function name (`server_tool`) and/or `server.tool`.
    #[serde(default)]
    pub tools: Vec<String>,
    /// If true, proceed with the tool when the approval call fails or times out.
    #[serde(default)]
    pub fail_open: bool,
}

fn default_mcp_hitl_timeout_ms() -> u64 {
    120_000
}

impl Default for McpHitlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            approval_url: String::new(),
            timeout_ms: default_mcp_hitl_timeout_ms(),
            bearer_token_env: None,
            tools: vec![],
            fail_open: false,
        }
    }
}

fn default_mcp_fail_open() -> bool {
    true
}

fn default_mcp_tool_timeout_ms() -> u64 {
    30_000
}

fn default_mcp_max_tool_payload_bytes() -> usize {
    1_048_576
}

fn default_mcp_max_tool_rounds() -> usize {
    4
}

fn default_mcp_stream_probe_bytes() -> usize {
    16 * 1024
}

fn default_mcp_probe_window_seconds() -> u64 {
    60
}

fn default_mcp_proof_of_intent_mode() -> String {
    "off".to_string()
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            fail_open: default_mcp_fail_open(),
            tool_timeout_ms: default_mcp_tool_timeout_ms(),
            max_tool_payload_bytes: default_mcp_max_tool_payload_bytes(),
            max_tool_rounds: default_mcp_max_tool_rounds(),
            stream_probe_bytes: default_mcp_stream_probe_bytes(),
            probe_window_seconds: default_mcp_probe_window_seconds(),
            advertise_tools: false,
            proof_of_intent_mode: default_mcp_proof_of_intent_mode(),
            intent_tool_policies: vec![],
            servers: vec![],
            hitl: McpHitlConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// Stable id for routing (e.g. `filesystem`, `corp-sql`).
    pub name: String,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
    /// When set, spawn this process and speak MCP over stdin/stdout (JSON-RPC, one object per line).
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

fn default_mcp_server_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpIntentToolPolicy {
    /// Intent label (example: `general`, `data_read`, `filesystem`).
    pub intent: String,
    /// Tool names allowed for this intent.
    /// Entries may be full OpenAI function names (`mcp_server_tool`) or canonical `server.tool`.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

/// Terminate TLS on the **same** `listen` socket (HTTP disabled for that process).
#[derive(Debug, Clone, Deserialize)]
pub struct TlsListenConfig {
    pub cert_pem: String,
    pub key_pem: String,
    /// If set, require a client certificate issued by this CA (PEM path).
    #[serde(default)]
    pub client_ca_pem: Option<String>,
}

/// Optional `server:` block: nested listen/port/TLS (alternative to top-level `listen` / `tls`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerSection {
    /// Full `host:port` or `[ipv6]:port` (overrides top-level `listen` when set).
    #[serde(default)]
    pub listen: Option<String>,
    /// When `listen` is unset, combined with `port` (default address `127.0.0.1`).
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub tls: Option<TlsListenConfig>,
}

/// Per-route HTTP rate limit (requests per second, shared across clients for that route).
#[derive(Debug, Clone, Deserialize)]
pub struct RouteRateLimitConfig {
    pub rps: u32,
}

/// Optional path-based upstream override (Kong-style routing light).
///
/// The first matching route with the **longest** `path_prefix` wins; otherwise the top-level
/// [`PandaConfig::upstream`] is used. Optional [`RouteConfig::methods`] restricts HTTP verbs for
/// that prefix (405 + `Allow` when not listed).
#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    /// Must start with `/`. Request paths that start with this prefix use `upstream` for that hop.
    #[serde(alias = "path")]
    pub path_prefix: String,
    pub upstream: String,
    #[serde(default)]
    pub rate_limit: Option<RouteRateLimitConfig>,
    /// Override TPM budget (tokens/minute) for this path; inherits [`TpmConfig::budget_tokens_per_minute`] when unset.
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    /// Override semantic cache: `None` = use global `semantic_cache.enabled`; `false` = never cache this path.
    #[serde(default)]
    pub semantic_cache: Option<bool>,
    /// Restrict MCP tool exposure/calls to these server names (must exist in global `mcp.servers`).
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
    /// Override adapter for this path (`openai` | `anthropic`); YAML key `type`.
    #[serde(default, rename = "type")]
    pub adapter_type: Option<String>,
    /// If non-empty, only these HTTP methods may be used for this path prefix (e.g. `GET`, `POST`, `PUT`, `PATCH`, `DELETE`).
    /// Omitted or empty → all methods allowed. Values are normalized to canonical form at load time.
    #[serde(default)]
    pub methods: Vec<String>,
}

/// On upstream HTTP 429, retry the same logical chat request against a secondary provider.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitFallbackConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Base URL for `anthropic` (e.g. `https://api.anthropic.com`) or full request URL for `openai_compatible`.
    #[serde(default)]
    pub upstream: String,
    /// `anthropic`: map OpenAI chat JSON → Anthropic Messages API. `openai_compatible`: POST the same JSON to `upstream`.
    #[serde(default = "default_rate_limit_fallback_provider")]
    pub provider: String,
    /// Env var for the fallback API key (`ANTHROPIC_API_KEY`, Azure `api-key`, etc.).
    #[serde(default = "default_rate_limit_fallback_api_key_env")]
    pub api_key_env: String,
    /// For `openai_compatible` on Azure-style hosts: send `api-key` instead of `Authorization: Bearer`.
    #[serde(default)]
    pub use_api_key_header: bool,
}

fn default_rate_limit_fallback_provider() -> String {
    "anthropic".to_string()
}

fn default_rate_limit_fallback_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

impl Default for RateLimitFallbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            upstream: String::new(),
            provider: default_rate_limit_fallback_provider(),
            api_key_env: default_rate_limit_fallback_api_key_env(),
            use_api_key_header: false,
        }
    }
}

/// Compress long OpenAI-style chat histories via a summarizer model before forwarding upstream.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextManagementConfig {
    #[serde(default)]
    pub enabled: bool,
    /// When `messages` length exceeds this, run summarization (after keeping a tail).
    #[serde(default = "default_context_max_messages")]
    pub max_messages: usize,
    /// Recent turns to keep verbatim after summarizing the prefix.
    #[serde(default = "default_context_keep_recent_messages")]
    pub keep_recent_messages: usize,
    /// OpenAI-compatible base URL for the summarizer (e.g. `https://api.openai.com`).
    #[serde(default)]
    pub summarizer_upstream: String,
    #[serde(default)]
    pub summarizer_model: String,
    /// Env var holding the bearer token for `summarizer_upstream`.
    #[serde(default = "default_context_summarizer_api_key_env")]
    pub summarizer_api_key_env: String,
    #[serde(default = "default_context_summarizer_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_context_summary_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_context_summarization_max_tokens")]
    pub summarization_max_tokens: u32,
}

fn default_context_max_messages() -> usize {
    48
}

fn default_context_keep_recent_messages() -> usize {
    16
}

fn default_context_summarizer_api_key_env() -> String {
    "PANDA_SUMMARIZER_API_KEY".to_string()
}

fn default_context_summarizer_timeout_ms() -> u64 {
    60_000
}

fn default_context_summary_system_prompt() -> String {
    "Summarize the following chat messages into a concise factual summary for continuing the conversation. Preserve important entities, decisions, constraints, and open tasks. Do not invent details."
        .to_string()
}

fn default_context_summarization_max_tokens() -> u32 {
    2048
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_messages: default_context_max_messages(),
            keep_recent_messages: default_context_keep_recent_messages(),
            summarizer_upstream: String::new(),
            summarizer_model: String::new(),
            summarizer_api_key_env: default_context_summarizer_api_key_env(),
            request_timeout_ms: default_context_summarizer_timeout_ms(),
            system_prompt: default_context_summary_system_prompt(),
            summarization_max_tokens: default_context_summarization_max_tokens(),
        }
    }
}

/// OIDC login for the Developer Console (Okta, Entra, etc.). Off by default (Core track).
#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleOidcConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Issuer base URL (e.g. `https://login.microsoftonline.com/{tenant}/v2.0`).
    #[serde(default)]
    pub issuer_url: String,
    #[serde(default)]
    pub client_id: String,
    /// Env var holding the OIDC client secret (confidential client).
    #[serde(default)]
    pub client_secret_env: String,
    /// Browser redirect URI path on this host (must match IdP app registration).
    #[serde(default = "default_console_oidc_redirect_path")]
    pub redirect_path: String,
    /// Public base URL for redirects (e.g. `https://panda.example.com`). If empty, derived from request `Host` (dev only).
    #[serde(default)]
    pub redirect_base_url: String,
    #[serde(default = "default_console_oidc_scopes")]
    pub scopes: Vec<String>,
    #[serde(default = "default_console_oidc_cookie_name")]
    pub cookie_name: String,
    #[serde(default = "default_console_oidc_session_ttl_seconds")]
    pub session_ttl_seconds: u64,
    /// Env var with secret bytes for signing console session cookies (HS256).
    #[serde(default = "default_console_oidc_signing_secret_env")]
    pub signing_secret_env: String,
    /// Optional JWT claim name on the IdP `id_token` for RBAC (string or array of strings).
    #[serde(default)]
    pub roles_claim: String,
    /// If non-empty, require at least one of these role strings in `roles_claim`.
    #[serde(default)]
    pub required_roles: Vec<String>,
}

fn default_console_oidc_redirect_path() -> String {
    "/console/oauth/callback".to_string()
}

fn default_console_oidc_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
    ]
}

fn default_console_oidc_cookie_name() -> String {
    "panda_console_session".to_string()
}

fn default_console_oidc_session_ttl_seconds() -> u64 {
    86400
}

fn default_console_oidc_signing_secret_env() -> String {
    "PANDA_CONSOLE_SESSION_SECRET".to_string()
}

impl Default for ConsoleOidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret_env: String::new(),
            redirect_path: default_console_oidc_redirect_path(),
            redirect_base_url: String::new(),
            scopes: default_console_oidc_scopes(),
            cookie_name: default_console_oidc_cookie_name(),
            session_ttl_seconds: default_console_oidc_session_ttl_seconds(),
            signing_secret_env: default_console_oidc_signing_secret_env(),
            roles_claim: String::new(),
            required_roles: vec![],
        }
    }
}

/// Per-department prompt-token budget (rolling minute), plus optional org-wide cap (Enterprise).
#[derive(Debug, Clone, Deserialize)]
pub struct BudgetHierarchyDepartmentLimit {
    pub department: String,
    pub prompt_tokens_per_minute: u64,
}

/// Hierarchical token budgets keyed by a JWT claim (e.g. `department`). Requires Redis.
#[derive(Debug, Clone, Deserialize)]
pub struct BudgetHierarchyConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Top-level claim name on the JWT payload (e.g. `department` or `cost_center`).
    #[serde(default = "default_budget_hierarchy_jwt_claim")]
    pub jwt_claim: String,
    /// Optional org-wide prompt budget per rolling minute (all departments share this cap).
    #[serde(default)]
    pub org_prompt_tokens_per_minute: Option<u64>,
    #[serde(default)]
    pub departments: Vec<BudgetHierarchyDepartmentLimit>,
    /// Override Redis URL; defaults to `tpm.redis_url` / `PANDA_REDIS_URL` when unset.
    #[serde(default)]
    pub redis_url: Option<String>,
}

fn default_budget_hierarchy_jwt_claim() -> String {
    "department".to_string()
}

impl Default for BudgetHierarchyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            jwt_claim: default_budget_hierarchy_jwt_claim(),
            org_prompt_tokens_per_minute: None,
            departments: vec![],
            redis_url: None,
        }
    }
}

/// Wire protocol for a failover hop (ingress remains OpenAI-shaped unless transformed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFailoverProtocol {
    /// OpenAI Chat Completions, Embeddings, Responses, etc. (same JSON/path as client).
    #[serde(alias = "openai")]
    OpenaiCompatible,
    /// Anthropic Messages API — Panda maps OpenAI chat JSON → `/v1/messages` for this hop only.
    Anthropic,
}

impl Default for ModelFailoverProtocol {
    fn default() -> Self {
        Self::OpenaiCompatible
    }
}

/// Which logical API this backend accepts for failover (empty `supports` = defaults for [`ModelFailoverProtocol`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFailoverOperation {
    ChatCompletions,
    Embeddings,
    Responses,
    Images,
    Audio,
}

/// One physical backend in a model parity / failover chain.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelFailoverBackend {
    pub upstream: String,
    /// When set, replace `Authorization` (or `api-key`) for this hop only.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub use_api_key_header: bool,
    #[serde(default)]
    pub protocol: ModelFailoverProtocol,
    /// Empty: infer from `protocol` (OpenAI: chat + embeddings + responses; Anthropic: chat only).
    #[serde(default)]
    pub supports: Vec<ModelFailoverOperation>,
    /// When `false`, this backend is skipped for streaming chat requests. Default: accept streaming.
    #[serde(default)]
    pub supports_streaming: Option<bool>,
}

/// Map request `model` names to an ordered list of backends (model parity / failover).
#[derive(Debug, Clone, Deserialize)]
pub struct ModelFailoverGroup {
    /// If empty, matches any `model` on the chat path.
    #[serde(default)]
    pub match_models: Vec<String>,
    #[serde(default)]
    pub backends: Vec<ModelFailoverBackend>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelFailoverConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Prefix for OpenAI-style chat (e.g. `/v1/chat` matches `/v1/chat/completions`).
    #[serde(default = "default_model_failover_path_prefix")]
    pub path_prefix: String,
    /// Optional prefix for `/v1/embeddings` (or compatible) failover. When unset, embeddings are not in the failover map.
    #[serde(default)]
    pub embeddings_path_prefix: Option<String>,
    /// Optional prefix for OpenAI Responses API failover (`/v1/responses`). Pass-through only (no Anthropic mapping).
    #[serde(default)]
    pub responses_path_prefix: Option<String>,
    /// Optional prefix for OpenAI Images API failover (`/v1/images`).
    #[serde(default)]
    pub images_path_prefix: Option<String>,
    /// Optional prefix for OpenAI Audio API failover (`/v1/audio`).
    #[serde(default)]
    pub audio_path_prefix: Option<String>,
    /// When `false` (default), once a hop returns `200` with an SSE body, Panda does not fail over to the next backend on mid-stream errors (safe default).
    #[serde(default)]
    pub allow_failover_after_first_byte: bool,
    /// Enable local in-process failover circuit breaker.
    #[serde(default)]
    pub circuit_breaker_enabled: bool,
    /// Open circuit after this many consecutive retryable failures per backend.
    #[serde(default = "default_model_failover_cb_failure_threshold")]
    pub circuit_breaker_failure_threshold: u32,
    /// Keep circuit open for this many seconds.
    #[serde(default = "default_model_failover_cb_open_seconds")]
    pub circuit_breaker_open_seconds: u64,
    #[serde(default)]
    pub groups: Vec<ModelFailoverGroup>,
}

fn default_model_failover_path_prefix() -> String {
    "/v1/chat".to_string()
}

fn default_model_failover_cb_failure_threshold() -> u32 {
    3
}

fn default_model_failover_cb_open_seconds() -> u64 {
    30
}

impl Default for ModelFailoverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path_prefix: default_model_failover_path_prefix(),
            embeddings_path_prefix: None,
            responses_path_prefix: None,
            images_path_prefix: None,
            audio_path_prefix: None,
            allow_failover_after_first_byte: false,
            circuit_breaker_enabled: false,
            circuit_breaker_failure_threshold: default_model_failover_cb_failure_threshold(),
            circuit_breaker_open_seconds: default_model_failover_cb_open_seconds(),
            groups: vec![],
        }
    }
}

/// Top-level config as loaded from disk (e.g. `panda.yaml`).
#[derive(Debug, Clone, Deserialize)]
pub struct PandaConfig {
    /// Bind address. Leave empty when using [`PandaConfig::server`] `listen` or `port`.
    #[serde(default)]
    pub listen: String,
    #[serde(default)]
    pub server: Option<ServerSection>,
    pub upstream: String,
    /// Per-path upstream bases; see [`RouteConfig`]. Empty preserves single-upstream behavior.
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub trusted_gateway: TrustedGatewayConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub tpm: TpmConfig,
    #[serde(default)]
    pub tls: Option<TlsListenConfig>,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub identity: IdentityConfig,
    /// Merged into `identity` (JWKS URL, `enforce_on_all_routes` → `require_jwt`).
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub prompt_safety: PromptSafetyConfig,
    #[serde(default)]
    pub pii: PiiConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub semantic_cache: SemanticCacheConfig,
    #[serde(default)]
    pub adapter: AdapterConfig,
    #[serde(default)]
    pub rate_limit_fallback: RateLimitFallbackConfig,
    #[serde(default)]
    pub context_management: ContextManagementConfig,
    #[serde(default)]
    pub console_oidc: ConsoleOidcConfig,
    #[serde(default)]
    pub budget_hierarchy: BudgetHierarchyConfig,
    #[serde(default)]
    pub model_failover: ModelFailoverConfig,
}

impl PandaConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        Self::from_yaml_str(&raw)
    }

    pub fn from_yaml_str(raw: &str) -> anyhow::Result<Self> {
        let mut cfg: Self = serde_yaml::from_str(raw)?;
        cfg.resolve_server_section()?;
        cfg.merge_auth_section();
        cfg.normalize_route_methods()?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Parse and canonicalize [`RouteConfig::methods`] (dedupe, validate tokens as HTTP methods).
    fn normalize_route_methods(&mut self) -> anyhow::Result<()> {
        use http::Method;
        for r in &mut self.routes {
            let mut out = Vec::new();
            for s in std::mem::take(&mut r.methods) {
                let t = s.trim();
                if t.is_empty() {
                    anyhow::bail!("routes.methods entry must not be empty or whitespace");
                }
                let m = Method::from_bytes(t.as_bytes()).map_err(|_| {
                    anyhow::anyhow!("routes.methods invalid HTTP method: {:?}", s)
                })?;
                // Canonical form for matching (HTTP methods are uppercase in practice; `http` may keep
                // lowercase tokens as extension methods).
                let canon = m.as_str().to_ascii_uppercase();
                if !out.contains(&canon) {
                    out.push(canon);
                }
            }
            r.methods = out;
        }
        Ok(())
    }

    /// Merge [`AuthConfig`] into [`IdentityConfig`] (JWKS URL, `enforce_on_all_routes` → `require_jwt`).
    fn merge_auth_section(&mut self) {
        if let Some(ref u) = self.auth.jwks_url {
            let t = u.trim();
            if !t.is_empty() {
                self.identity.jwks_url = Some(t.to_string());
            }
        }
        if self.auth.enforce_on_all_routes {
            self.identity.require_jwt = true;
        }
    }

    /// Merge [`PandaConfig::server`] into `listen` / `tls` when present.
    ///
    /// - Non-empty `server.listen` overrides the top-level `listen` string.
    /// - `server.port` (+ optional `server.address`) is used **only** when `listen` is still empty
    ///   after that, so `listen: "127.0.0.1:9000"` with `server: { port: 8080, tls: ... }` keeps
    ///   `9000` and only applies nested TLS.
    fn resolve_server_section(&mut self) -> anyhow::Result<()> {
        let Some(ref s) = self.server else {
            return Ok(());
        };
        if let Some(ref l) = s.listen {
            if !l.trim().is_empty() {
                self.listen = l.clone();
            }
        }
        if self.listen.trim().is_empty() {
            if let Some(port) = s.port {
                let addr = s.address.as_deref().unwrap_or("127.0.0.1");
                self.listen = format_listen_address(addr, port);
            }
        }
        if let Some(ref tls) = s.tls {
            self.tls = Some(tls.clone());
        }
        Ok(())
    }

    pub fn listen_addr(&self) -> anyhow::Result<std::net::SocketAddr> {
        if let Ok(override_listen) = std::env::var("PANDA_LISTEN_OVERRIDE") {
            let t = override_listen.trim();
            if !t.is_empty() {
                return t.parse().map_err(|e| {
                    anyhow::anyhow!("invalid PANDA_LISTEN_OVERRIDE (expected host:port): {e}")
                });
            }
        }
        Ok(self.listen.parse()?)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.listen.trim().is_empty() {
            anyhow::bail!("`listen` must be set (top-level `listen`, or `server.listen` / `server.port`)");
        }
        if self.upstream.trim().is_empty() {
            anyhow::bail!("`upstream` must not be empty");
        }
        let _: http::Uri = self
            .upstream
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid `upstream` URL: {e}"))?;
        self.trusted_gateway.validate()?;
        if self.observability.correlation_header.trim().is_empty() {
            anyhow::bail!("`observability.correlation_header` must not be empty");
        }
        HeaderName::from_bytes(self.observability.correlation_header.as_bytes()).map_err(|_| {
            anyhow::anyhow!("invalid observability.correlation_header token")
        })?;
        if self.observability.admin_auth_header.trim().is_empty() {
            anyhow::bail!("`observability.admin_auth_header` must not be empty");
        }
        HeaderName::from_bytes(self.observability.admin_auth_header.as_bytes()).map_err(|_| {
            anyhow::anyhow!("invalid observability.admin_auth_header token")
        })?;
        if self
            .observability
            .admin_secret_env
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        {
            anyhow::bail!("observability.admin_secret_env must be non-empty when set");
        }
        if let Some(ref t) = self.tls {
            if !Path::new(&t.cert_pem).is_file() {
                anyhow::bail!("tls.cert_pem not a file: {}", t.cert_pem);
            }
            if !Path::new(&t.key_pem).is_file() {
                anyhow::bail!("tls.key_pem not a file: {}", t.key_pem);
            }
            if let Some(ref ca) = t.client_ca_pem {
                if !Path::new(ca).is_file() {
                    anyhow::bail!("tls.client_ca_pem not a file: {ca}");
                }
            }
        }
        if let Some(ref d) = self.plugins.directory {
            if d.trim().is_empty() {
                anyhow::bail!("`plugins.directory` must not be empty when set");
            }
            if !Path::new(d).is_dir() {
                anyhow::bail!("plugins.directory is not a directory: {d}");
            }
        }
        if self.plugins.max_request_body_bytes == 0 {
            anyhow::bail!("plugins.max_request_body_bytes must be > 0");
        }
        if self.plugins.execution_timeout_ms == 0 {
            anyhow::bail!("plugins.execution_timeout_ms must be > 0");
        }
        if self.plugins.reload_interval_ms == 0 {
            anyhow::bail!("plugins.reload_interval_ms must be > 0");
        }
        if self.plugins.reload_debounce_ms == 0 {
            anyhow::bail!("plugins.reload_debounce_ms must be > 0");
        }
        if self.plugins.max_reloads_per_minute == 0 {
            anyhow::bail!("plugins.max_reloads_per_minute must be > 0");
        }
        if self.tpm.enforce_budget && self.tpm.budget_tokens_per_minute == 0 {
            anyhow::bail!("tpm.budget_tokens_per_minute must be > 0 when enforce_budget=true");
        }
        if self.tpm.retry_after_seconds.is_some_and(|n| n == 0) {
            anyhow::bail!("tpm.retry_after_seconds must be > 0 when set");
        }
        if !(self.tpm.redis_degraded_limit_ratio > 0.0 && self.tpm.redis_degraded_limit_ratio <= 1.0) {
            anyhow::bail!("tpm.redis_degraded_limit_ratio must be in (0, 1]");
        }
        let ce = &self.observability.compliance_export;
        if ce.enabled {
            let m = ce.mode.to_ascii_lowercase();
            if m != "local_jsonl" {
                anyhow::bail!(
                    "observability.compliance_export.mode must be \"local_jsonl\" when enabled (object-store modes are documented only)"
                );
            }
            if ce.local_path.trim().is_empty() {
                anyhow::bail!("observability.compliance_export.local_path is required when enabled");
            }
        }
        if ce
            .signing_secret_env
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
        {
            anyhow::bail!("observability.compliance_export.signing_secret_env must be non-empty when set");
        }
        if self.identity.require_jwt {
            let has_hs256 = !self.identity.jwt_hs256_secret_env.trim().is_empty();
            let has_jwks = self
                .identity
                .jwks_url
                .as_ref()
                .is_some_and(|u| !u.trim().is_empty());
            if !has_hs256 && !has_jwks {
                anyhow::bail!(
                    "when identity.require_jwt=true, set identity.jwt_hs256_secret_env and/or identity.jwks_url (or auth.jwks_url)"
                );
            }
        }
        if let Some(ref u) = self.identity.jwks_url {
            if !u.trim().is_empty() {
                let _: http::Uri = u
                    .parse()
                    .map_err(|e| anyhow::anyhow!("identity.jwks_url invalid URI: {e}"))?;
                if self.identity.jwks_cache_ttl_seconds == 0 {
                    anyhow::bail!("identity.jwks_cache_ttl_seconds must be > 0 when jwks_url is set");
                }
            }
        }
        if self
            .identity
            .accepted_issuers
            .iter()
            .any(|v| v.trim().is_empty())
        {
            anyhow::bail!("identity.accepted_issuers entries must be non-empty");
        }
        if self
            .identity
            .accepted_audiences
            .iter()
            .any(|v| v.trim().is_empty())
        {
            anyhow::bail!("identity.accepted_audiences entries must be non-empty");
        }
        if self
            .identity
            .required_scopes
            .iter()
            .any(|v| v.trim().is_empty())
        {
            anyhow::bail!("identity.required_scopes entries must be non-empty");
        }
        for r in &self.identity.route_scope_rules {
            if r.path_prefix.trim().is_empty() {
                anyhow::bail!("identity.route_scope_rules.path_prefix must be non-empty");
            }
            if r.required_scopes.is_empty() {
                anyhow::bail!("identity.route_scope_rules.required_scopes must not be empty");
            }
            if r.required_scopes.iter().any(|v| v.trim().is_empty()) {
                anyhow::bail!("identity.route_scope_rules.required_scopes entries must be non-empty");
            }
        }
        if self.identity.enable_token_exchange {
            if !self.identity.require_jwt {
                anyhow::bail!("identity.enable_token_exchange requires identity.require_jwt=true");
            }
            if self.identity.agent_token_secret_env.trim().is_empty() {
                anyhow::bail!("identity.agent_token_secret_env must be non-empty");
            }
            if self.identity.agent_token_ttl_seconds == 0 {
                anyhow::bail!("identity.agent_token_ttl_seconds must be > 0");
            }
            if self.identity.agent_token_scopes.is_empty() {
                anyhow::bail!("identity.agent_token_scopes must not be empty when token exchange is enabled");
            }
            if self
                .identity
                .agent_token_scopes
                .iter()
                .any(|v| v.trim().is_empty())
            {
                anyhow::bail!("identity.agent_token_scopes entries must be non-empty");
            }
        }
        if self
            .prompt_safety
            .deny_patterns
            .iter()
            .any(|v| v.trim().is_empty())
        {
            anyhow::bail!("prompt_safety.deny_patterns entries must be non-empty");
        }
        if self.pii.enabled {
            if self.pii.replacement.trim().is_empty() {
                anyhow::bail!("pii.replacement must be non-empty when pii.enabled=true");
            }
            if self.pii.redact_patterns.is_empty() {
                anyhow::bail!("pii.redact_patterns must not be empty when pii.enabled=true");
            }
            for p in &self.pii.redact_patterns {
                if p.trim().is_empty() {
                    anyhow::bail!("pii.redact_patterns entries must be non-empty");
                }
                regex::Regex::new(p)
                    .map_err(|e| anyhow::anyhow!("invalid pii.redact_patterns regex {p:?}: {e}"))?;
            }
        }
        if self.mcp.enabled {
            if self.mcp.tool_timeout_ms == 0 {
                anyhow::bail!("mcp.tool_timeout_ms must be > 0 when mcp.enabled=true");
            }
            if self.mcp.max_tool_payload_bytes == 0 {
                anyhow::bail!("mcp.max_tool_payload_bytes must be > 0 when mcp.enabled=true");
            }
            if self.mcp.max_tool_rounds == 0 {
                anyhow::bail!("mcp.max_tool_rounds must be > 0 when mcp.enabled=true");
            }
            if self.mcp.stream_probe_bytes == 0 {
                anyhow::bail!("mcp.stream_probe_bytes must be > 0 when mcp.enabled=true");
            }
            if self.mcp.probe_window_seconds == 0 {
                anyhow::bail!("mcp.probe_window_seconds must be > 0 when mcp.enabled=true");
            }
            if self.mcp.servers.is_empty() {
                anyhow::bail!("mcp.servers must not be empty when mcp.enabled=true");
            }
            let mut seen = std::collections::HashSet::<String>::new();
            for s in &self.mcp.servers {
                if s.name.trim().is_empty() {
                    anyhow::bail!("mcp.servers entries must have non-empty name");
                }
                if let Some(ref c) = s.command {
                    if c.trim().is_empty() {
                        anyhow::bail!("mcp.servers command must be non-empty when set");
                    }
                }
                if !seen.insert(s.name.clone()) {
                    anyhow::bail!("mcp.servers names must be unique: duplicate {:?}", s.name);
                }
            }
            if !self.mcp.servers.iter().any(|s| s.enabled) {
                anyhow::bail!("mcp.enabled requires at least one mcp.servers entry with enabled=true");
            }
            match self.mcp.proof_of_intent_mode.as_str() {
                "off" | "audit" | "enforce" => {}
                _ => anyhow::bail!("mcp.proof_of_intent_mode must be one of: off, audit, enforce"),
            }
            for p in &self.mcp.intent_tool_policies {
                if p.intent.trim().is_empty() {
                    anyhow::bail!("mcp.intent_tool_policies.intent must be non-empty");
                }
                if p.allowed_tools.is_empty() {
                    anyhow::bail!("mcp.intent_tool_policies.allowed_tools must not be empty");
                }
                if p.allowed_tools.iter().any(|t| t.trim().is_empty()) {
                    anyhow::bail!("mcp.intent_tool_policies.allowed_tools entries must be non-empty");
                }
            }
            if self.mcp.hitl.enabled {
                if self.mcp.hitl.approval_url.trim().is_empty() {
                    anyhow::bail!("mcp.hitl.approval_url must be set when mcp.hitl.enabled=true");
                }
                let u: http::Uri = self
                    .mcp
                    .hitl
                    .approval_url
                    .trim()
                    .parse()
                    .map_err(|e| anyhow::anyhow!("mcp.hitl.approval_url invalid URI: {e}"))?;
                if u.scheme_str() != Some("http") && u.scheme_str() != Some("https") {
                    anyhow::bail!("mcp.hitl.approval_url must use http or https");
                }
                if self.mcp.hitl.timeout_ms == 0 {
                    anyhow::bail!("mcp.hitl.timeout_ms must be > 0 when mcp.hitl.enabled=true");
                }
                if self.mcp.hitl.tools.is_empty() {
                    anyhow::bail!("mcp.hitl.tools must not be empty when mcp.hitl.enabled=true");
                }
                if self.mcp.hitl.tools.iter().any(|t| t.trim().is_empty()) {
                    anyhow::bail!("mcp.hitl.tools entries must be non-empty");
                }
                if self
                    .mcp
                    .hitl
                    .bearer_token_env
                    .as_ref()
                    .is_some_and(|v| v.trim().is_empty())
                {
                    anyhow::bail!("mcp.hitl.bearer_token_env must be non-empty when set");
                }
            }
        } else if self.mcp.hitl.enabled {
            anyhow::bail!("mcp.hitl.enabled requires mcp.enabled=true");
        }
        if self.rate_limit_fallback.enabled {
            if self.rate_limit_fallback.upstream.trim().is_empty() {
                anyhow::bail!("rate_limit_fallback.upstream must be set when rate_limit_fallback.enabled=true");
            }
            if self.rate_limit_fallback.api_key_env.trim().is_empty() {
                anyhow::bail!("rate_limit_fallback.api_key_env must be non-empty when rate_limit_fallback.enabled=true");
            }
            match self.rate_limit_fallback.provider.as_str() {
                "anthropic" => {
                    let u: http::Uri = self
                        .rate_limit_fallback
                        .upstream
                        .trim()
                        .parse()
                        .map_err(|e| anyhow::anyhow!("rate_limit_fallback.upstream invalid URI: {e}"))?;
                    if u.scheme_str() != Some("http") && u.scheme_str() != Some("https") {
                        anyhow::bail!("rate_limit_fallback.upstream must use http or https");
                    }
                }
                "openai_compatible" => {
                    let u: http::Uri = self
                        .rate_limit_fallback
                        .upstream
                        .trim()
                        .parse()
                        .map_err(|e| anyhow::anyhow!("rate_limit_fallback.upstream invalid URI: {e}"))?;
                    if u.scheme_str() != Some("http") && u.scheme_str() != Some("https") {
                        anyhow::bail!("rate_limit_fallback.upstream must use http or https");
                    }
                }
                _ => anyhow::bail!(
                    "rate_limit_fallback.provider must be one of: anthropic, openai_compatible"
                ),
            }
        }
        if self.context_management.enabled {
            if self.context_management.max_messages == 0 {
                anyhow::bail!("context_management.max_messages must be > 0 when context_management.enabled=true");
            }
            if self.context_management.keep_recent_messages == 0 {
                anyhow::bail!(
                    "context_management.keep_recent_messages must be > 0 when context_management.enabled=true"
                );
            }
            if self.context_management.keep_recent_messages >= self.context_management.max_messages {
                anyhow::bail!(
                    "context_management.keep_recent_messages must be < context_management.max_messages"
                );
            }
            if self.context_management.summarizer_upstream.trim().is_empty() {
                anyhow::bail!(
                    "context_management.summarizer_upstream must be set when context_management.enabled=true"
                );
            }
            let _: http::Uri = self
                .context_management
                .summarizer_upstream
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("context_management.summarizer_upstream invalid URI: {e}"))?;
            if self.context_management.summarizer_model.trim().is_empty() {
                anyhow::bail!(
                    "context_management.summarizer_model must be set when context_management.enabled=true"
                );
            }
            if self.context_management.summarizer_api_key_env.trim().is_empty() {
                anyhow::bail!(
                    "context_management.summarizer_api_key_env must be non-empty when context_management.enabled=true"
                );
            }
            if self.context_management.request_timeout_ms == 0 {
                anyhow::bail!(
                    "context_management.request_timeout_ms must be > 0 when context_management.enabled=true"
                );
            }
            if self.context_management.summarization_max_tokens == 0 {
                anyhow::bail!(
                    "context_management.summarization_max_tokens must be > 0 when context_management.enabled=true"
                );
            }
            if self.context_management.system_prompt.trim().is_empty() {
                anyhow::bail!("context_management.system_prompt must be non-empty when context_management.enabled=true");
            }
        }
        if self.console_oidc.enabled {
            if self.console_oidc.issuer_url.trim().is_empty() {
                anyhow::bail!("console_oidc.issuer_url is required when console_oidc.enabled=true");
            }
            let iu: http::Uri = self
                .console_oidc
                .issuer_url
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("console_oidc.issuer_url invalid URI: {e}"))?;
            if iu.scheme_str() != Some("https") && iu.scheme_str() != Some("http") {
                anyhow::bail!("console_oidc.issuer_url must use http or https");
            }
            if self.console_oidc.client_id.trim().is_empty() {
                anyhow::bail!("console_oidc.client_id is required when console_oidc.enabled=true");
            }
            if self.console_oidc.client_secret_env.trim().is_empty() {
                anyhow::bail!("console_oidc.client_secret_env is required when console_oidc.enabled=true");
            }
            if self.console_oidc.signing_secret_env.trim().is_empty() {
                anyhow::bail!("console_oidc.signing_secret_env is required when console_oidc.enabled=true");
            }
            if self.console_oidc.redirect_path.trim().is_empty()
                || !self.console_oidc.redirect_path.starts_with('/')
            {
                anyhow::bail!("console_oidc.redirect_path must be a non-empty absolute path");
            }
            if self.console_oidc.session_ttl_seconds == 0 {
                anyhow::bail!("console_oidc.session_ttl_seconds must be > 0 when console_oidc.enabled=true");
            }
            if self.console_oidc.cookie_name.trim().is_empty() {
                anyhow::bail!("console_oidc.cookie_name must be non-empty when console_oidc.enabled=true");
            }
            if self.console_oidc.scopes.is_empty() {
                anyhow::bail!("console_oidc.scopes must not be empty when console_oidc.enabled=true");
            }
            for r in &self.console_oidc.required_roles {
                if r.trim().is_empty() {
                    anyhow::bail!("console_oidc.required_roles entries must be non-empty");
                }
            }
            if !self.console_oidc.redirect_base_url.trim().is_empty() {
                let b: http::Uri = self
                    .console_oidc
                    .redirect_base_url
                    .trim()
                    .parse()
                    .map_err(|e| anyhow::anyhow!("console_oidc.redirect_base_url invalid URI: {e}"))?;
                if b.scheme_str() != Some("https") && b.scheme_str() != Some("http") {
                    anyhow::bail!("console_oidc.redirect_base_url must use http or https");
                }
            }
        }
        if self.budget_hierarchy.enabled {
            let redis = self.effective_budget_hierarchy_redis_url();
            if redis.is_none() {
                anyhow::bail!(
                    "budget_hierarchy.enabled requires tpm.redis_url, budget_hierarchy.redis_url, or PANDA_REDIS_URL"
                );
            }
            if self.budget_hierarchy.jwt_claim.trim().is_empty() {
                anyhow::bail!("budget_hierarchy.jwt_claim must be non-empty when budget_hierarchy.enabled=true");
            }
            if self.budget_hierarchy.org_prompt_tokens_per_minute == Some(0) {
                anyhow::bail!("budget_hierarchy.org_prompt_tokens_per_minute must be > 0 when set");
            }
            if self.budget_hierarchy.departments.is_empty()
                && self.budget_hierarchy.org_prompt_tokens_per_minute.is_none()
            {
                anyhow::bail!(
                    "budget_hierarchy requires at least one departments[] entry or org_prompt_tokens_per_minute"
                );
            }
            for d in &self.budget_hierarchy.departments {
                if d.department.trim().is_empty() {
                    anyhow::bail!("budget_hierarchy.departments.department must be non-empty");
                }
                if d.prompt_tokens_per_minute == 0 {
                    anyhow::bail!("budget_hierarchy.departments.prompt_tokens_per_minute must be > 0");
                }
            }
            if !self.identity.require_jwt {
                anyhow::bail!("budget_hierarchy.enabled requires identity.require_jwt=true for JWT claim extraction");
            }
        }
        if self.model_failover.enabled {
            if self.model_failover.path_prefix.trim().is_empty()
                || !self.model_failover.path_prefix.starts_with('/')
            {
                anyhow::bail!("model_failover.path_prefix must be a non-empty path starting with /");
            }
            if self.model_failover.groups.is_empty() {
                anyhow::bail!("model_failover.groups must not be empty when model_failover.enabled=true");
            }
            for g in &self.model_failover.groups {
                if g.backends.is_empty() {
                    anyhow::bail!("each model_failover group must have at least one backend");
                }
                for m in &g.match_models {
                    if m.trim().is_empty() {
                        anyhow::bail!("model_failover.match_models entries must be non-empty");
                    }
                }
                for b in &g.backends {
                    if b.upstream.trim().is_empty() {
                        anyhow::bail!("model_failover backend upstream must not be empty");
                    }
                    let u: http::Uri = b
                        .upstream
                        .trim()
                        .parse()
                        .map_err(|e| anyhow::anyhow!("model_failover backend upstream invalid URI: {e}"))?;
                    if u.scheme_str() != Some("http") && u.scheme_str() != Some("https") {
                        anyhow::bail!("model_failover backend upstream must use http or https");
                    }
                    if b.api_key_env.as_ref().is_some_and(|e| e.trim().is_empty()) {
                        anyhow::bail!("model_failover backend api_key_env must be non-empty when set");
                    }
                    if b.protocol == ModelFailoverProtocol::Anthropic {
                        for op in &b.supports {
                            if *op != ModelFailoverOperation::ChatCompletions {
                                anyhow::bail!(
                                    "model_failover: anthropic protocol backends only support chat_completions (got {:?})",
                                    op
                                );
                            }
                        }
                    }
                }
            }
            if let Some(ref e) = self.model_failover.embeddings_path_prefix {
                let t = e.trim();
                if t.is_empty() || !t.starts_with('/') {
                    anyhow::bail!("model_failover.embeddings_path_prefix must be a non-empty path starting with /");
                }
            }
            if let Some(ref r) = self.model_failover.responses_path_prefix {
                let t = r.trim();
                if t.is_empty() || !t.starts_with('/') {
                    anyhow::bail!("model_failover.responses_path_prefix must be a non-empty path starting with /");
                }
            }
            if let Some(ref r) = self.model_failover.images_path_prefix {
                let t = r.trim();
                if t.is_empty() || !t.starts_with('/') {
                    anyhow::bail!("model_failover.images_path_prefix must be a non-empty path starting with /");
                }
            }
            if let Some(ref r) = self.model_failover.audio_path_prefix {
                let t = r.trim();
                if t.is_empty() || !t.starts_with('/') {
                    anyhow::bail!("model_failover.audio_path_prefix must be a non-empty path starting with /");
                }
            }
            if self.model_failover.circuit_breaker_enabled {
                if self.model_failover.circuit_breaker_failure_threshold == 0 {
                    anyhow::bail!("model_failover.circuit_breaker_failure_threshold must be > 0");
                }
                if self.model_failover.circuit_breaker_open_seconds == 0 {
                    anyhow::bail!("model_failover.circuit_breaker_open_seconds must be > 0");
                }
            }
        }
        if self.semantic_cache.enabled {
            match self.semantic_cache.backend.as_str() {
                "memory" | "redis" => {}
                _ => anyhow::bail!("semantic_cache.backend must be one of: memory, redis"),
            }
            if self
                .semantic_cache
                .redis_url
                .as_ref()
                .is_some_and(|v| v.trim().is_empty())
            {
                anyhow::bail!("semantic_cache.redis_url must be non-empty when set");
            }
            if !(0.0..=1.0).contains(&self.semantic_cache.similarity_threshold) {
                anyhow::bail!("semantic_cache.similarity_threshold must be within [0.0, 1.0]");
            }
            if self.semantic_cache.max_entries == 0 {
                anyhow::bail!("semantic_cache.max_entries must be > 0 when semantic_cache.enabled=true");
            }
            if self.semantic_cache.ttl_seconds == 0 {
                anyhow::bail!("semantic_cache.ttl_seconds must be > 0 when semantic_cache.enabled=true");
            }
        }
        match self.adapter.provider.as_str() {
            "openai" | "anthropic" => {}
            _ => anyhow::bail!("adapter.provider must be one of: openai, anthropic"),
        }
        if self.adapter.anthropic_version.trim().is_empty() {
            anyhow::bail!("adapter.anthropic_version must be non-empty");
        }
        let mut seen_prefixes = std::collections::HashSet::<String>::new();
        for r in &self.routes {
            if r.path_prefix.trim().is_empty() {
                anyhow::bail!("routes.path_prefix must be non-empty");
            }
            if !r.path_prefix.starts_with('/') {
                anyhow::bail!(
                    "routes.path_prefix must start with '/': {:?}",
                    r.path_prefix
                );
            }
            if r.upstream.trim().is_empty() {
                anyhow::bail!("routes.upstream must not be empty");
            }
            let _: http::Uri = r
                .upstream
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid routes.upstream URL: {e}"))?;
            if !seen_prefixes.insert(r.path_prefix.clone()) {
                anyhow::bail!("routes.path_prefix must be unique: {:?}", r.path_prefix);
            }
            if let Some(ref rl) = r.rate_limit {
                if rl.rps == 0 {
                    anyhow::bail!("routes.rate_limit.rps must be > 0 when set");
                }
            }
            if let Some(n) = r.tpm_limit {
                if n == 0 {
                    anyhow::bail!("routes.tpm_limit must be > 0 when set");
                }
            }
            if r.semantic_cache == Some(true) && !self.semantic_cache.enabled {
                anyhow::bail!("routes.semantic_cache=true requires semantic_cache.enabled=true");
            }
            if let Some(ref t) = r.adapter_type {
                match t.as_str() {
                    "openai" | "anthropic" => {}
                    _ => anyhow::bail!("routes.type must be one of: openai, anthropic"),
                }
            }
            if let Some(ref names) = r.mcp_servers {
                if !names.is_empty() {
                    if !self.mcp.enabled {
                        anyhow::bail!("routes.mcp_servers requires mcp.enabled=true");
                    }
                    let known: std::collections::HashSet<_> =
                        self.mcp.servers.iter().map(|s| s.name.as_str()).collect();
                    for n in names {
                        if n.trim().is_empty() {
                            anyhow::bail!("routes.mcp_servers entries must be non-empty");
                        }
                        if !known.contains(n.as_str()) {
                            anyhow::bail!(
                                "routes.mcp_servers references unknown MCP server: {:?}",
                                n
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Longest-prefix matching route for an **ingress** path (client URI path).
    pub fn effective_route_for_path(&self, path: &str) -> Option<&RouteConfig> {
        let mut best: Option<(&RouteConfig, usize)> = None;
        for r in &self.routes {
            if path.starts_with(r.path_prefix.as_str()) {
                let len = r.path_prefix.len();
                if best.map(|(_, l)| len > l).unwrap_or(true) {
                    best = Some((r, len));
                }
            }
        }
        best.map(|(r, _)| r)
    }

    /// When the matching route sets [`RouteConfig::methods`], verifies the request method is allowed.
    ///
    /// Returns `Err(allow_list)` where `allow_list` is suitable for an HTTP `Allow` response header
    /// (comma-separated methods) when the method is not permitted; otherwise `Ok(())`.
    /// No matching route or empty `methods` → `Ok(())`.
    pub fn check_ingress_method(&self, path: &str, method: &http::Method) -> Result<(), String> {
        let Some(route) = self.effective_route_for_path(path) else {
            return Ok(());
        };
        if route.methods.is_empty() {
            return Ok(());
        }
        let m = method.as_str();
        if route
            .methods
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(m))
        {
            return Ok(());
        }
        Err(route.methods.join(", "))
    }

    /// Upstream base URL for a request path: longest matching [`PandaConfig::routes`] entry, else [`PandaConfig::upstream`].
    pub fn effective_upstream_base(&self, path: &str) -> &str {
        self.effective_route_for_path(path)
            .map(|r| r.upstream.as_str())
            .unwrap_or(self.upstream.as_str())
    }

    /// TPM budget (tokens per minute) for an ingress path (per-route [`RouteConfig::tpm_limit`] or global).
    pub fn effective_tpm_budget_tokens_per_minute(&self, path: &str) -> u64 {
        self.effective_route_for_path(path)
            .and_then(|r| r.tpm_limit)
            .unwrap_or(self.tpm.budget_tokens_per_minute)
    }

    /// Whether semantic cache may be used for this ingress path.
    pub fn effective_semantic_cache_enabled_for_path(&self, path: &str) -> bool {
        let global_on = self.semantic_cache.enabled;
        match self.effective_route_for_path(path).and_then(|r| r.semantic_cache) {
            None => global_on,
            Some(false) => false,
            Some(true) => global_on,
        }
    }

    /// Adapter provider for an ingress path (`openai` or `anthropic`).
    pub fn effective_adapter_provider(&self, path: &str) -> &str {
        self.effective_route_for_path(path)
            .and_then(|r| r.adapter_type.as_deref())
            .unwrap_or(self.adapter.provider.as_str())
    }

    /// When set, only these MCP server names are used for tools on this path (`Some(&[])` disables tools).
    pub fn effective_mcp_server_names(&self, path: &str) -> Option<&[String]> {
        self.effective_route_for_path(path)
            .and_then(|r| r.mcp_servers.as_ref())
            .map(|v| v.as_slice())
    }

    /// True when the default and every route `upstream` parses as an HTTP URI.
    pub fn all_upstream_uris_valid(&self) -> bool {
        if self.upstream.parse::<http::Uri>().is_err() {
            return false;
        }
        self.routes
            .iter()
            .all(|r| r.upstream.parse::<http::Uri>().is_ok())
    }

    /// Effective Redis URL: YAML, then env `PANDA_REDIS_URL`.
    pub fn effective_redis_url(&self) -> Option<String> {
        self.tpm
            .redis_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("PANDA_REDIS_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
    }

    /// Redis for hierarchical budgets: explicit `budget_hierarchy.redis_url`, else TPM/env Redis.
    pub fn effective_budget_hierarchy_redis_url(&self) -> Option<String> {
        self.budget_hierarchy
            .redis_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.effective_redis_url())
    }

    /// Effective semantic-cache Redis URL: YAML, then env `PANDA_SEMANTIC_CACHE_REDIS_URL`, then `PANDA_REDIS_URL`.
    pub fn effective_semantic_cache_redis_url(&self) -> Option<String> {
        self.semantic_cache
            .redis_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("PANDA_SEMANTIC_CACHE_REDIS_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("PANDA_REDIS_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that mutate process environment variables.
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn rejects_empty_listen() {
        let err = PandaConfig::from_yaml_str("listen: ''\nupstream: 'http://localhost'\n")
            .unwrap_err();
        assert!(err.to_string().contains("listen"));
    }

    #[test]
    fn parses_minimal() {
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n",
        )
        .unwrap();
        assert!(cfg.listen_addr().is_ok());
    }

    #[test]
    fn listen_addr_honors_panda_listen_override() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:9'\nupstream: 'http://127.0.0.1:11434'\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var("PANDA_LISTEN_OVERRIDE", "127.0.0.1:8088");
        }
        let a = cfg.listen_addr().unwrap();
        assert_eq!(a.port(), 8088);
        unsafe {
            std::env::remove_var("PANDA_LISTEN_OVERRIDE");
        }
    }

    #[test]
    fn rejects_bad_trusted_header_name() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:1'\n\
             trusted_gateway:\n  subject_header: 'bad name'\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid trusted_gateway"));
    }

    #[test]
    fn plugins_defaults_are_non_zero() {
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n",
        )
        .unwrap();
        assert!(cfg.plugins.max_request_body_bytes > 0);
        assert!(cfg.plugins.execution_timeout_ms > 0);
        assert!(!cfg.plugins.fail_closed);
        assert!(!cfg.plugins.hot_reload);
        assert!(cfg.plugins.reload_interval_ms > 0);
        assert!(cfg.plugins.reload_debounce_ms > 0);
        assert!(cfg.plugins.max_reloads_per_minute > 0);
        assert!(!cfg.tpm.enforce_budget);
        assert!(cfg.tpm.budget_tokens_per_minute > 0);
        assert!(cfg.tpm.retry_after_seconds.is_none());
        assert!(cfg.tpm.redis_unavailable_degraded_limits);
        assert!(cfg.tpm.redis_command_error_degraded_limits);
        assert_eq!(cfg.tpm.redis_degraded_limit_ratio, 0.5);
        assert!(!cfg.observability.compliance_export.enabled);
        assert_eq!(cfg.observability.compliance_export.mode, "off");
        assert!(!cfg.console_oidc.enabled);
        assert!(!cfg.budget_hierarchy.enabled);
        assert!(!cfg.model_failover.enabled);
        assert!(!cfg.model_failover.allow_failover_after_first_byte);
        assert!(cfg.model_failover.embeddings_path_prefix.is_none());
        assert!(cfg.model_failover.responses_path_prefix.is_none());
        assert_eq!(cfg.observability.admin_auth_header, "x-panda-admin-secret");
        assert!(cfg.observability.admin_secret_env.is_none());
        assert!(!cfg.identity.require_jwt);
        assert!(cfg.identity.jwks_url.is_none());
        assert_eq!(cfg.identity.jwks_cache_ttl_seconds, 3600);
        assert!(cfg.identity.accepted_issuers.is_empty());
        assert!(cfg.identity.accepted_audiences.is_empty());
        assert!(cfg.identity.required_scopes.is_empty());
        assert!(cfg.identity.route_scope_rules.is_empty());
        assert!(!cfg.identity.enable_token_exchange);
        assert_eq!(cfg.identity.agent_token_secret_env, "PANDA_AGENT_TOKEN_HS256_SECRET");
        assert_eq!(cfg.identity.agent_token_ttl_seconds, 300);
        assert!(cfg.identity.agent_token_scopes.is_empty());
        assert!(!cfg.prompt_safety.enabled);
        assert!(!cfg.prompt_safety.shadow_mode);
        assert!(cfg.prompt_safety.deny_patterns.is_empty());
        assert!(!cfg.pii.enabled);
        assert!(!cfg.pii.shadow_mode);
        assert!(cfg.pii.redact_patterns.is_empty());
        assert_eq!(cfg.pii.replacement, "[REDACTED]");
        assert!(!cfg.mcp.enabled);
        assert!(cfg.mcp.fail_open);
        assert_eq!(cfg.mcp.tool_timeout_ms, 30_000);
        assert_eq!(cfg.mcp.max_tool_payload_bytes, 1_048_576);
        assert_eq!(cfg.mcp.max_tool_rounds, 4);
        assert_eq!(cfg.mcp.stream_probe_bytes, 16 * 1024);
        assert_eq!(cfg.mcp.probe_window_seconds, 60);
        assert!(!cfg.mcp.advertise_tools);
        assert_eq!(cfg.mcp.proof_of_intent_mode, "off");
        assert!(cfg.mcp.intent_tool_policies.is_empty());
        assert!(cfg.mcp.servers.is_empty());
        assert!(!cfg.mcp.hitl.enabled);
        assert!(!cfg.rate_limit_fallback.enabled);
        assert!(!cfg.context_management.enabled);
        assert!(!cfg.semantic_cache.enabled);
        assert_eq!(cfg.semantic_cache.backend, "memory");
        assert!(cfg.semantic_cache.redis_url.is_none());
        assert_eq!(cfg.semantic_cache.similarity_threshold, 0.92);
        assert_eq!(cfg.semantic_cache.max_entries, 10_000);
        assert_eq!(cfg.semantic_cache.ttl_seconds, 300);
        assert_eq!(cfg.adapter.provider, "openai");
        assert_eq!(cfg.adapter.anthropic_version, "2023-06-01");
        assert!(cfg.routes.is_empty());
        assert!(cfg.server.is_none());
    }

    #[test]
    fn rejects_mcp_enabled_without_servers() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("mcp.servers"));
    }

    #[test]
    fn accepts_mcp_enabled_with_one_server() {
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  servers:\n    - name: demo\n",
        )
        .unwrap();
        assert!(cfg.mcp.enabled);
        assert_eq!(cfg.mcp.servers.len(), 1);
        assert_eq!(cfg.mcp.servers[0].name, "demo");
    }

    #[test]
    fn rejects_mcp_hitl_without_mcp_enabled() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  hitl:\n    enabled: true\n    approval_url: 'https://a.example/ok'\n    tools: ['x.y']\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("mcp.hitl.enabled requires mcp.enabled"));
    }

    #[test]
    fn accepts_mcp_hitl_when_mcp_enabled() {
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  servers:\n    - name: demo\n  hitl:\n    enabled: true\n    approval_url: 'https://a.example/approve'\n    tools: ['demo.drop']\n",
        )
        .unwrap();
        assert!(cfg.mcp.hitl.enabled);
        assert_eq!(cfg.mcp.hitl.tools, vec!["demo.drop".to_string()]);
    }

    #[test]
    fn rejects_rate_limit_fallback_bad_provider() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             rate_limit_fallback:\n  enabled: true\n  provider: acme\n  upstream: 'https://api.example'\n  api_key_env: 'K'\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("rate_limit_fallback.provider"));
    }

    #[test]
    fn rejects_context_management_keep_recent_too_large() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             context_management:\n  enabled: true\n  max_messages: 10\n  keep_recent_messages: 10\n  summarizer_upstream: 'https://api.openai.com'\n  summarizer_model: 'gpt-4o-mini'\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("keep_recent_messages"));
    }

    #[test]
    fn auth_block_merges_jwks_url_and_enforce_on_all_routes() {
        let cfg = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             auth:\n  type: jwt\n  jwks_url: 'https://issuer.example/.well-known/jwks.json'\n  enforce_on_all_routes: true\n",
        )
        .unwrap();
        assert!(cfg.identity.require_jwt);
        assert_eq!(
            cfg.identity.jwks_url.as_deref(),
            Some("https://issuer.example/.well-known/jwks.json")
        );
    }

    #[test]
    fn rejects_invalid_mcp_proof_of_intent_mode() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  proof_of_intent_mode: strict\n  servers:\n    - name: demo\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("proof_of_intent_mode"));
    }

    #[test]
    fn rejects_empty_mcp_command_when_set() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  servers:\n    - name: demo\n      command: ''\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("command"));
    }

    #[test]
    fn rejects_invalid_mcp_max_tool_rounds() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  max_tool_rounds: 0\n  servers:\n    - name: demo\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("mcp.max_tool_rounds"));
    }

    #[test]
    fn rejects_invalid_mcp_stream_probe_bytes() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  stream_probe_bytes: 0\n  servers:\n    - name: demo\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("mcp.stream_probe_bytes"));
    }

    #[test]
    fn rejects_invalid_mcp_probe_window_seconds() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             mcp:\n  enabled: true\n  probe_window_seconds: 0\n  servers:\n    - name: demo\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("mcp.probe_window_seconds"));
    }

    #[test]
    fn rejects_invalid_semantic_cache_threshold() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             semantic_cache:\n  enabled: true\n  similarity_threshold: 1.2\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("semantic_cache.similarity_threshold"));
    }

    #[test]
    fn rejects_invalid_semantic_cache_backend() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             semantic_cache:\n  enabled: true\n  backend: dragonfly\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("semantic_cache.backend"));
    }

    #[test]
    fn rejects_invalid_adapter_provider() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             adapter:\n  provider: gemini\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("adapter.provider"));
    }

    #[test]
    fn parses_routes_and_effective_upstream_longest_prefix() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://default.example'
routes:
  - path_prefix: /v1
    upstream: 'http://v1.example'
  - path_prefix: /v1/chat
    upstream: 'http://chat.example'
"#,
        )
        .unwrap();
        assert_eq!(cfg.effective_upstream_base("/other"), "http://default.example");
        assert_eq!(cfg.effective_upstream_base("/v1/models"), "http://v1.example");
        assert_eq!(cfg.effective_upstream_base("/v1/chat/completions"), "http://chat.example");
    }

    #[test]
    fn rejects_duplicate_route_path_prefix() {
        let err = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /api
    upstream: 'http://a.example'
  - path_prefix: /api
    upstream: 'http://b.example'
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unique"));
    }

    #[test]
    fn rejects_route_path_prefix_without_leading_slash() {
        let err = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: api
    upstream: 'http://a.example'
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("routes.path_prefix"));
    }

    #[test]
    fn rejects_invalid_route_upstream_url() {
        let err = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /x
    upstream: 'http://['
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("routes.upstream"));
    }

    #[test]
    fn server_block_port_sets_listen() {
        let cfg = PandaConfig::from_yaml_str(
            r#"server:
  port: 8080
  address: "127.0.0.1"
upstream: 'http://127.0.0.1:1'
"#,
        )
        .unwrap();
        assert_eq!(cfg.listen, "127.0.0.1:8080");
    }

    #[test]
    fn server_port_does_not_override_top_level_listen() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:9000'
server:
  port: 8080
  address: "127.0.0.1"
upstream: 'http://127.0.0.1:1'
"#,
        )
        .unwrap();
        assert_eq!(cfg.listen, "127.0.0.1:9000");
    }

    #[test]
    fn server_listen_empty_falls_back_to_server_port() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: ''
server:
  listen: ''
  port: 3000
upstream: 'http://127.0.0.1:1'
"#,
        )
        .unwrap();
        assert_eq!(cfg.listen, "127.0.0.1:3000");
    }

    #[test]
    fn route_path_alias_and_per_route_overrides() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://default'
semantic_cache:
  enabled: true
mcp:
  enabled: true
  servers:
    - name: a
routes:
  - path: /v1/chat
    upstream: 'http://chat'
    rate_limit:
      rps: 50
    tpm_limit: 9000
    semantic_cache: false
    mcp_servers: [a]
    type: anthropic
"#,
        )
        .unwrap();
        assert_eq!(cfg.effective_upstream_base("/v1/chat/x"), "http://chat");
        assert_eq!(cfg.effective_tpm_budget_tokens_per_minute("/v1/chat"), 9000);
        assert!(!cfg.effective_semantic_cache_enabled_for_path("/v1/chat"));
        assert_eq!(cfg.effective_adapter_provider("/v1/chat"), "anthropic");
        assert_eq!(
            cfg.effective_mcp_server_names("/v1/chat").map(|s| s.to_vec()),
            Some(vec!["a".to_string()])
        );
    }

    #[test]
    fn route_methods_normalize_and_check_ingress() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /api
    upstream: 'http://api.example'
    methods: [GET, POST, PUT, PATCH, DELETE]
"#,
        )
        .unwrap();
        let r = cfg.effective_route_for_path("/api/items").unwrap();
        assert_eq!(r.methods, vec!["GET", "POST", "PUT", "PATCH", "DELETE"]);
        assert!(cfg.check_ingress_method("/api", &http::Method::GET).is_ok());
        assert!(cfg.check_ingress_method("/api/x", &http::Method::DELETE).is_ok());
        assert!(cfg.check_ingress_method("/other", &http::Method::PATCH).is_ok());
        assert!(cfg
            .check_ingress_method("/api", &http::Method::OPTIONS)
            .is_err());
    }

    #[test]
    fn route_methods_rejects_invalid_token() {
        let err = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /api
    upstream: 'http://api.example'
    methods: ["BAD METHOD"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid HTTP method"));
    }
}
