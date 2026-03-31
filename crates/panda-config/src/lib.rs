//! GitOps-style configuration: parse and validate YAML without starting the server.
//!
//! Kept separate from the proxy so unit tests can cover invalid files and defaults
//! without binding to a network stack.

use std::path::Path;

use http::header::HeaderName;
use serde::Deserialize;

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
}

fn default_tpm_budget_tokens_per_minute() -> u64 {
    60_000
}

impl Default for TpmConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            enforce_budget: false,
            budget_tokens_per_minute: default_tpm_budget_tokens_per_minute(),
            retry_after_seconds: None,
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

/// Optional identity controls for Phase 3 entry.
#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    /// If true, require a bearer JWT on proxied requests.
    #[serde(default)]
    pub require_jwt: bool,
    /// Env var name containing HS256 secret (default: PANDA_JWT_HS256_SECRET).
    #[serde(default = "default_jwt_hs256_secret_env")]
    pub jwt_hs256_secret_env: String,
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

/// Top-level config as loaded from disk (e.g. `panda.yaml`).
#[derive(Debug, Clone, Deserialize)]
pub struct PandaConfig {
    pub listen: String,
    pub upstream: String,
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
}

impl PandaConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        Self::from_yaml_str(&raw)
    }

    pub fn from_yaml_str(raw: &str) -> anyhow::Result<Self> {
        let cfg: Self = serde_yaml::from_str(raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn listen_addr(&self) -> anyhow::Result<std::net::SocketAddr> {
        Ok(self.listen.parse()?)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.listen.trim().is_empty() {
            anyhow::bail!("`listen` must not be empty");
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
        if self.identity.require_jwt && self.identity.jwt_hs256_secret_env.trim().is_empty() {
            anyhow::bail!("identity.jwt_hs256_secret_env must be non-empty when require_jwt=true");
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
        }
        if self.semantic_cache.enabled {
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
        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(cfg.observability.admin_auth_header, "x-panda-admin-secret");
        assert!(cfg.observability.admin_secret_env.is_none());
        assert!(!cfg.identity.require_jwt);
        assert!(cfg.identity.accepted_issuers.is_empty());
        assert!(cfg.identity.accepted_audiences.is_empty());
        assert!(cfg.identity.required_scopes.is_empty());
        assert!(cfg.identity.route_scope_rules.is_empty());
        assert!(!cfg.identity.enable_token_exchange);
        assert_eq!(cfg.identity.agent_token_secret_env, "PANDA_AGENT_TOKEN_HS256_SECRET");
        assert_eq!(cfg.identity.agent_token_ttl_seconds, 300);
        assert!(cfg.identity.agent_token_scopes.is_empty());
        assert!(!cfg.prompt_safety.enabled);
        assert!(cfg.prompt_safety.deny_patterns.is_empty());
        assert!(!cfg.pii.enabled);
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
        assert!(!cfg.semantic_cache.enabled);
        assert_eq!(cfg.semantic_cache.similarity_threshold, 0.92);
        assert_eq!(cfg.semantic_cache.max_entries, 10_000);
        assert_eq!(cfg.semantic_cache.ttl_seconds, 300);
        assert_eq!(cfg.adapter.provider, "openai");
        assert_eq!(cfg.adapter.anthropic_version, "2023-06-01");
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
    fn rejects_invalid_adapter_provider() {
        let err = PandaConfig::from_yaml_str(
            "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:11434'\n\
             adapter:\n  provider: gemini\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("adapter.provider"));
    }
}
