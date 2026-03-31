use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use redis::AsyncCommands;
use sha2::{Digest, Sha256};

pub struct SemanticCache {
    backend: SemanticCacheBackend,
}

enum SemanticCacheBackend {
    Memory(MemorySemanticCache),
    Redis(RedisSemanticCache),
}

struct MemorySemanticCache {
    max_entries: usize,
    ttl: Duration,
    similarity_threshold: f32,
    inner: Mutex<Inner>,
}

struct RedisSemanticCache {
    ttl_seconds: u64,
    redis: redis::aio::ConnectionManager,
}

struct Inner {
    map: HashMap<String, (Instant, Vec<u8>)>,
    order: VecDeque<String>,
}

impl SemanticCache {
    pub async fn connect(
        backend: &str,
        max_entries: usize,
        ttl: Duration,
        similarity_threshold: f32,
        redis_url: Option<&str>,
    ) -> anyhow::Result<Self> {
        match backend {
            "memory" => Ok(Self::new_memory(max_entries, ttl, similarity_threshold)),
            "redis" => {
                let url = redis_url
                    .filter(|u| !u.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("semantic cache backend=redis requires redis_url or PANDA_SEMANTIC_CACHE_REDIS_URL"))?;
                let client = redis::Client::open(url)?;
                let redis = redis::aio::ConnectionManager::new(client).await?;
                Ok(Self {
                    backend: SemanticCacheBackend::Redis(RedisSemanticCache {
                        ttl_seconds: ttl.as_secs().max(1),
                        redis,
                    }),
                })
            }
            _ => anyhow::bail!("unsupported semantic cache backend: {backend}"),
        }
    }

    pub fn new_memory(max_entries: usize, ttl: Duration, similarity_threshold: f32) -> Self {
        Self {
            backend: SemanticCacheBackend::Memory(MemorySemanticCache {
                max_entries,
                ttl,
                similarity_threshold,
                inner: Mutex::new(Inner {
                    map: HashMap::new(),
                    order: VecDeque::new(),
                }),
            }),
        }
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        match &self.backend {
            SemanticCacheBackend::Memory(mem) => mem.get(key),
            SemanticCacheBackend::Redis(redis_backend) => redis_backend.get(key).await,
        }
    }

    pub async fn put(&self, key: String, value: Vec<u8>) {
        match &self.backend {
            SemanticCacheBackend::Memory(mem) => mem.put(key, value),
            SemanticCacheBackend::Redis(redis_backend) => redis_backend.put(key, value).await,
        }
    }
}

impl MemorySemanticCache {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let now = Instant::now();
        let mut g = self.inner.lock().expect("semantic cache mutex poisoned");
        if let Some((ts, v)) = g.map.get(key).cloned() {
            if now.duration_since(ts) <= self.ttl {
                return Some(v);
            }
            g.map.remove(key);
            remove_all_from_order(&mut g.order, key);
        }
        // Similarity fallback (MVP): choose best unexpired key by Jaccard token overlap.
        let (model, tools_sig, _msgs_sig) = parse_cache_key_contract(key)?;
        let mut best: Option<(String, f32, Vec<u8>)> = None;
        let mut expired = Vec::new();
        for (k, (ts, v)) in &g.map {
            if now.duration_since(*ts) > self.ttl {
                expired.push(k.clone());
                continue;
            }
            let Some((k_model, k_tools_sig, _)) = parse_cache_key_contract(k) else {
                continue;
            };
            // Hard compatibility gate before any semantic fallback.
            if k_model != model || k_tools_sig != tools_sig {
                continue;
            }
            let sim = jaccard_similarity(key, k);
            if sim >= self.similarity_threshold {
                match best {
                    Some((_, cur, _)) if sim <= cur => {}
                    _ => best = Some((k.clone(), sim, v.clone())),
                }
            }
        }
        for k in expired {
            g.map.remove(&k);
            remove_all_from_order(&mut g.order, &k);
        }
        best.map(|(_, _, v)| v)
    }

    fn put(&self, key: String, value: Vec<u8>) {
        let now = Instant::now();
        let mut g = self.inner.lock().expect("semantic cache mutex poisoned");
        if g.map.contains_key(&key) {
            remove_all_from_order(&mut g.order, &key);
        }
        g.map.insert(key.clone(), (now, value));
        g.order.push_back(key);
        while g.map.len() > self.max_entries {
            if let Some(k) = g.order.pop_front() {
                g.map.remove(&k);
            } else {
                break;
            }
        }
    }
}

impl RedisSemanticCache {
    async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let mut conn = self.redis.clone();
        let redis_key = semantic_cache_redis_key(key);
        let result: Result<Option<Vec<u8>>, _> = conn.get(redis_key).await;
        match result {
            Ok(v) => v,
            Err(e) => {
                eprintln!("panda semantic-cache(redis): get failed: {e}");
                None
            }
        }
    }

    async fn put(&self, key: String, value: Vec<u8>) {
        let mut conn = self.redis.clone();
        let redis_key = semantic_cache_redis_key(&key);
        let result: Result<(), _> = conn.set_ex(redis_key, value, self.ttl_seconds).await;
        if let Err(e) = result {
            eprintln!("panda semantic-cache(redis): set_ex failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SemanticCache;
    use std::time::Duration;

    #[tokio::test]
    async fn cache_put_get_roundtrip() {
        let c = SemanticCache::new_memory(10, Duration::from_secs(30), 0.92);
        c.put("k".to_string(), b"v".to_vec()).await;
        assert_eq!(c.get("k").await.as_deref(), Some(&b"v"[..]));
    }

    #[tokio::test]
    async fn cache_evicts_oldest_when_full() {
        let c = SemanticCache::new_memory(1, Duration::from_secs(30), 0.92);
        c.put("k1".to_string(), b"v1".to_vec()).await;
        c.put("k2".to_string(), b"v2".to_vec()).await;
        assert!(c.get("k1").await.is_none());
        assert_eq!(c.get("k2").await.as_deref(), Some(&b"v2"[..]));
    }

    #[tokio::test]
    async fn cache_similarity_hit() {
        let c = SemanticCache::new_memory(10, Duration::from_secs(30), 0.5);
        c.put(
            r#"{"model":"a","messages":"list users","tools":"t"}"#.to_string(),
            b"v".to_vec(),
        )
        .await;
        assert_eq!(
            c.get(r#"{"model":"a","messages":"list all users","tools":"t"}"#)
                .await
                .as_deref(),
            Some(&b"v"[..])
        );
    }

    #[tokio::test]
    async fn cache_similarity_respects_model_and_tools_contract() {
        let c = SemanticCache::new_memory(10, Duration::from_secs(30), 0.1);
        c.put(
            r#"{"model":"a","messages":"list users","tools":"t1"}"#.to_string(),
            b"v1".to_vec(),
        )
        .await;
        assert!(c
            .get(r#"{"model":"b","messages":"list users","tools":"t1"}"#)
            .await
            .is_none());
        assert!(c
            .get(r#"{"model":"a","messages":"list users","tools":"t2"}"#)
            .await
            .is_none());
    }
}

fn jaccard_similarity(a: &str, b: &str) -> f32 {
    use std::collections::HashSet;
    let ta: HashSet<String> = a
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let tb: HashSet<String> = b
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn remove_all_from_order(order: &mut VecDeque<String>, key: &str) {
    order.retain(|k| k != key);
}

fn parse_cache_key_contract(key: &str) -> Option<(String, String, String)> {
    let v: serde_json::Value = serde_json::from_str(key).ok()?;
    let model = v.get("model").and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let tools_sig = serde_json::to_string(v.get("tools").unwrap_or(&serde_json::Value::Null)).ok()?;
    let messages_sig =
        serde_json::to_string(v.get("messages").unwrap_or(&serde_json::Value::Null)).ok()?;
    Some((model, tools_sig, messages_sig))
}

fn semantic_cache_redis_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let hex = digest.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    format!("panda:semantic:v1:{hex}")
}
