//! GitOps-style configuration: parse and validate YAML without starting the server.
//!
//! Kept separate from the proxy so unit tests can cover invalid files and defaults
//! without binding to a network stack.

use std::path::Path;

use serde::Deserialize;

/// Top-level config as loaded from disk (e.g. `panda.yaml`).
#[derive(Debug, Clone, Deserialize)]
pub struct PandaConfig {
    /// Socket address for the HTTP listener, e.g. `127.0.0.1:8080`.
    pub listen: String,
    /// Base URL for the upstream LLM HTTP API (OpenAI-compatible in later milestones).
    pub upstream: String,
}

impl PandaConfig {
    /// Load and parse YAML from `path`.
    pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        Self::from_yaml_str(&raw)
    }

    /// Parse YAML from memory (tests and tooling).
    pub fn from_yaml_str(raw: &str) -> anyhow::Result<Self> {
        let cfg: Self = serde_yaml::from_str(raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Parsed [`std::net::SocketAddr`] for `listen`.
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
        Ok(())
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
}
