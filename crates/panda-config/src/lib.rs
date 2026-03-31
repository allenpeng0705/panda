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
    }
}
