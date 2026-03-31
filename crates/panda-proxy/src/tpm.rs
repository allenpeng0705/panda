//! Prompt / completion counters: in-process and optional Redis (`INCRBY`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;

pub struct TpmCounters {
    mem: Mutex<HashMap<String, (u64, u64)>>,
    mem_prompt_window: Mutex<HashMap<String, (std::time::Instant, u64)>>,
    redis: Option<redis::aio::ConnectionManager>,
}

impl TpmCounters {
    pub async fn connect(effective_redis_url: Option<&str>) -> anyhow::Result<Self> {
        let redis = if let Some(url) = effective_redis_url.filter(|u| !u.trim().is_empty()) {
            let client = redis::Client::open(url)?;
            Some(redis::aio::ConnectionManager::new(client).await?)
        } else {
            None
        };
        Ok(Self {
            mem: Mutex::new(HashMap::new()),
            mem_prompt_window: Mutex::new(HashMap::new()),
            redis,
        })
    }

    pub async fn add_prompt_tokens(&self, bucket: &str, n: u64) {
        if n == 0 {
            return;
        }
        {
            let mut g = self.mem.lock().expect("tpm mutex poisoned");
            g.entry(bucket.to_string()).or_default().0 += n;
        }
        {
            let now = std::time::Instant::now();
            let mut g = self.mem_prompt_window.lock().expect("tpm mutex poisoned");
            let e = g.entry(bucket.to_string()).or_insert((now, 0));
            if now.duration_since(e.0) >= std::time::Duration::from_secs(60) {
                *e = (now, 0);
            }
            e.1 = e.1.saturating_add(n);
        }
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let key = format!("panda:tpm:v1:prompt:{bucket}");
            let _: Result<i64, _> = conn.incr(&key, n as i64).await;
            let minute = unix_minute();
            let window_key = format!("panda:tpm:v1:prompt_window:{bucket}:{minute}");
            let _: Result<i64, _> = conn.incr(&window_key, n as i64).await;
            let _: Result<bool, _> = conn.expire(&window_key, 120).await;
        }
    }

    pub async fn add_completion_tokens(&self, bucket: &str, n: u64) {
        if n == 0 {
            return;
        }
        {
            let mut g = self.mem.lock().expect("tpm mutex poisoned");
            g.entry(bucket.to_string()).or_default().1 += n;
        }
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let key = format!("panda:tpm:v1:completion:{bucket}");
            let _: Result<i64, _> = conn.incr(&key, n as i64).await;
        }
    }

    /// Returns true when adding `n` would exceed `limit_per_minute`.
    pub async fn would_exceed_prompt_budget(&self, bucket: &str, n: u64, limit_per_minute: u64) -> bool {
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let minute = unix_minute();
            let window_key = format!("panda:tpm:v1:prompt_window:{bucket}:{minute}");
            let cur: Result<Option<u64>, _> = conn.get(&window_key).await;
            if let Ok(current) = cur {
                return current.unwrap_or(0).saturating_add(n) > limit_per_minute;
            }
        }
        let now = std::time::Instant::now();
        let mut g = self.mem_prompt_window.lock().expect("tpm mutex poisoned");
        let e = g.entry(bucket.to_string()).or_insert((now, 0));
        if now.duration_since(e.0) >= std::time::Duration::from_secs(60) {
            *e = (now, 0);
        }
        e.1.saturating_add(n) > limit_per_minute
    }
}

fn unix_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 60)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::TpmCounters;

    #[tokio::test]
    async fn in_memory_prompt_budget_enforced() {
        let counters = TpmCounters::connect(None).await.unwrap();
        assert!(!counters.would_exceed_prompt_budget("u1", 50, 100).await);
        counters.add_prompt_tokens("u1", 50).await;
        assert!(counters.would_exceed_prompt_budget("u1", 51, 100).await);
        assert!(!counters.would_exceed_prompt_budget("u1", 50, 100).await);
    }
}
