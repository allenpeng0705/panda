//! Prompt / completion counters: in-process and optional Redis (`INCRBY`).

use std::collections::HashMap;
use std::sync::Mutex;

use redis::AsyncCommands;

pub struct TpmCounters {
    mem: Mutex<HashMap<String, (u64, u64)>>,
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
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let key = format!("panda:tpm:v1:prompt:{bucket}");
            let _: Result<i64, _> = conn.incr(&key, n as i64).await;
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
}
