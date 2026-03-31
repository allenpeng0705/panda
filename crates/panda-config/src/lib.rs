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
}

fn default_correlation_header() -> String {
    "x-request-id".to_string()
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            correlation_header: default_correlation_header(),
        }
    }
}

/// TPM backends: optional Redis for multi-instance totals (`PANDA_REDIS_URL` overrides `redis_url`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TpmConfig {
    #[serde(default)]
    pub redis_url: Option<String>,
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
#[derive(Debug, Clone, Deserialize, Default)]
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
}

fn default_jwt_hs256_secret_env() -> String {
    "PANDA_JWT_HS256_SECRET".to_string()
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
        assert!(!cfg.identity.require_jwt);
        assert!(cfg.identity.accepted_issuers.is_empty());
        assert!(cfg.identity.accepted_audiences.is_empty());
    }
}
