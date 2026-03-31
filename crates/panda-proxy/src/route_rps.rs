//! Per-route HTTP RPS limits ([`panda_config::RouteRateLimitConfig`]).
//!
//! Implementation: process-local fixed **1 second** windows per matching `path_prefix` (not
//! distributed across replicas). The mutex serializes check+increment so the count cannot race
//! past the cap within a window.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use panda_config::PandaConfig;

/// Longest-prefix route with `rate_limit` shares one fixed 1s window per `path_prefix` (process-local).
pub struct RouteRpsLimiters {
    limits: HashMap<String, u32>,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RouteRpsLimiters {
    pub fn from_config(cfg: &PandaConfig) -> Option<std::sync::Arc<Self>> {
        let mut limits = HashMap::new();
        for r in &cfg.routes {
            if let Some(ref rl) = r.rate_limit {
                limits.insert(r.path_prefix.clone(), rl.rps);
            }
        }
        if limits.is_empty() {
            return None;
        }
        Some(std::sync::Arc::new(Self {
            limits,
            windows: Mutex::new(HashMap::new()),
        }))
    }

    /// Returns `Err(rps)` when the matching route's per-second quota is exceeded.
    pub fn check(&self, cfg: &PandaConfig, ingress_path: &str) -> Result<(), u32> {
        let Some(route) = cfg.effective_route_for_path(ingress_path) else {
            return Ok(());
        };
        let Some(ref rl) = route.rate_limit else {
            return Ok(());
        };
        let Some(&cap) = self.limits.get(&route.path_prefix) else {
            return Ok(());
        };
        let mut g = self
            .windows
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let e = g.entry(route.path_prefix.clone()).or_insert((now, 0));
        if now.duration_since(e.0) >= Duration::from_secs(1) {
            e.0 = now;
            e.1 = 0;
        }
        if e.1 >= cap {
            return Err(rl.rps);
        }
        e.1 += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panda_config::PandaConfig;

    #[test]
    fn rps_enforces_cap_within_window() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /api
    upstream: 'http://127.0.0.1:2'
    rate_limit:
      rps: 2
"#,
        )
        .unwrap();
        let lim = RouteRpsLimiters::from_config(&cfg).expect("limiters");
        assert!(lim.check(&cfg, "/api/a").is_ok());
        assert!(lim.check(&cfg, "/api/b").is_ok());
        assert!(lim.check(&cfg, "/api/c").is_err());
    }

    #[test]
    fn rps_skips_when_no_matching_route() {
        let cfg = PandaConfig::from_yaml_str(
            r#"listen: '127.0.0.1:0'
upstream: 'http://127.0.0.1:1'
routes:
  - path_prefix: /api
    upstream: 'http://127.0.0.1:2'
    rate_limit:
      rps: 1
"#,
        )
        .unwrap();
        let lim = RouteRpsLimiters::from_config(&cfg).expect("limiters");
        assert!(lim.check(&cfg, "/other").is_ok());
        assert!(lim.check(&cfg, "/other").is_ok());
    }
}
