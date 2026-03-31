//! HTTP reverse proxy with streaming bodies (SSE-friendly).
//!
//! [`panda_config::PandaConfig`] supplies the upstream base URL; this crate does not read YAML.

mod gateway;
mod sse;
mod tls;
mod tpm;
mod upstream;

pub use gateway::RequestContext;

use std::convert::Infallible;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use panda_config::PandaConfig;
use panda_wasm::{HookFailure, PluginRuntime, RuntimeReason};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_rustls::TlsAcceptor;
use tpm::TpmCounters;

type BoxBody = UnsyncBoxBody<bytes::Bytes, hyper::Error>;
type HttpClient = Client<HttpsConnector<HttpConnector>, BoxBody>;

/// Shared state for each connection handler.
pub struct ProxyState {
    pub config: Arc<PandaConfig>,
    pub client: HttpClient,
    pub tpm: Arc<TpmCounters>,
    pub bpe: Option<Arc<tiktoken_rs::CoreBPE>>,
    /// Hot-swappable plugin runtime and metrics.
    plugins: Option<Arc<PluginManager>>,
}

#[derive(Default)]
struct PluginMetrics {
    counts: std::sync::Mutex<HashMap<String, u64>>,
}

impl PluginMetrics {
    fn inc(&self, key: String) {
        if let Ok(mut g) = self.counts.lock() {
            let n = g.entry(key).or_insert(0);
            *n += 1;
        }
    }

    fn snapshot(&self) -> Vec<(String, u64)> {
        self.counts
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct ReloadState {
    last_success_epoch_ms: Option<u128>,
    last_error: Option<String>,
    reload_count: u64,
    skipped_debounce: u64,
    skipped_rate_limit: u64,
    reload_timestamps: VecDeque<SystemTime>,
}

struct PluginManager {
    dir: PathBuf,
    runtime: RwLock<Arc<PluginRuntime>>,
    fingerprint: std::sync::Mutex<u64>,
    metrics: PluginMetrics,
    reload_interval: Duration,
    reload_debounce: Duration,
    max_reloads_per_minute: usize,
    last_change_seen: std::sync::Mutex<Option<SystemTime>>,
    reload_state: std::sync::Mutex<ReloadState>,
}

impl PluginManager {
    fn new(
        dir: PathBuf,
        runtime: Arc<PluginRuntime>,
        reload_interval: Duration,
        reload_debounce: Duration,
        max_reloads_per_minute: usize,
    ) -> anyhow::Result<Self> {
        let fp = dir_fingerprint(&dir)?;
        Ok(Self {
            dir,
            runtime: RwLock::new(runtime),
            fingerprint: std::sync::Mutex::new(fp),
            metrics: PluginMetrics::default(),
            reload_interval,
            reload_debounce,
            max_reloads_per_minute,
            last_change_seen: std::sync::Mutex::new(None),
            reload_state: std::sync::Mutex::new(ReloadState::default()),
        })
    }

    async fn runtime_snapshot(&self) -> Arc<PluginRuntime> {
        let g = self.runtime.read().await;
        Arc::clone(&*g)
    }

    fn spawn_hot_reload(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(this.reload_interval).await;
                if let Err(e) = this.reload_if_changed().await {
                    eprintln!("panda: wasm hot-reload scan failed: {e:#}");
                }
            }
        });
    }

    async fn reload_if_changed(&self) -> anyhow::Result<()> {
        let now = dir_fingerprint(&self.dir)?;
        let current_fp = *self
            .fingerprint
            .lock()
            .map_err(|e| anyhow::anyhow!("plugin fingerprint lock: {e}"))?;
        if current_fp == now {
            return Ok(());
        }
        let now_time = SystemTime::now();
        {
            let mut last = self
                .last_change_seen
                .lock()
                .map_err(|e| anyhow::anyhow!("plugin last_change lock: {e}"))?;
            if let Some(prev) = *last {
                if now_time
                    .duration_since(prev)
                    .unwrap_or_else(|_| Duration::from_secs(0))
                    < self.reload_debounce
                {
                    if let Ok(mut rs) = self.reload_state.lock() {
                        rs.skipped_debounce += 1;
                    }
                    return Ok(());
                }
            }
            *last = Some(now_time);
        }
        {
            let mut rs = self
                .reload_state
                .lock()
                .map_err(|e| anyhow::anyhow!("plugin reload_state lock: {e}"))?;
            while let Some(front) = rs.reload_timestamps.front() {
                if now_time
                    .duration_since(*front)
                    .unwrap_or_else(|_| Duration::from_secs(0))
                    > Duration::from_secs(60)
                {
                    rs.reload_timestamps.pop_front();
                } else {
                    break;
                }
            }
            if rs.reload_timestamps.len() >= self.max_reloads_per_minute {
                rs.skipped_rate_limit += 1;
                rs.last_error = Some("reload throttled: max_reloads_per_minute".to_string());
                return Ok(());
            }
        }
        let next = PluginRuntime::load_optional(Some(self.dir.as_path()))?
            .ok_or_else(|| anyhow::anyhow!("plugins directory unexpectedly missing"))?;
        let next = Arc::new(next);
        {
            let mut w = self.runtime.write().await;
            *w = Arc::clone(&next);
        }
        *self
            .fingerprint
            .lock()
            .map_err(|e| anyhow::anyhow!("plugin fingerprint lock: {e}"))? = now;
        if let Ok(mut rs) = self.reload_state.lock() {
            rs.reload_count += 1;
            rs.last_error = None;
            rs.last_success_epoch_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis());
            rs.reload_timestamps.push_back(now_time);
        }
        eprintln!("panda: wasm runtime hot-reloaded (plugins={})", next.plugin_count());
        Ok(())
    }

    fn record_allow_all(&self, runtime: &PluginRuntime, hook: &str) {
        for name in runtime.plugin_names() {
            self.metrics.inc(format!("{name}|{hook}|allow"));
        }
    }

    fn record_policy_reject(&self, plugin: &str, code: &str, hook: &str) {
        self.metrics.inc(format!("{plugin}|{hook}|reject|{code}"));
    }

    fn record_runtime(&self, plugin: &str, reason: RuntimeReason, hook: &str) {
        self.metrics
            .inc(format!("{plugin}|{hook}|runtime|{reason:?}"));
    }

    fn record_timeout(&self, hook: &str) {
        self.metrics.inc(format!("_all|{hook}|timeout"));
    }

    fn metrics_prometheus_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("# HELP panda_plugin_events_total Plugin hook events by plugin/hook/outcome".to_string());
        lines.push("# TYPE panda_plugin_events_total counter".to_string());
        for (k, v) in self.metrics.snapshot() {
            let parts: Vec<&str> = k.split('|').collect();
            let plugin = parts.first().copied().unwrap_or("_all");
            let hook = parts.get(1).copied().unwrap_or("unknown");
            let outcome = parts.get(2).copied().unwrap_or("unknown");
            let detail = parts.get(3).copied().unwrap_or("");
            lines.push(format!(
                "panda_plugin_events_total{{plugin=\"{}\",hook=\"{}\",outcome=\"{}\",detail=\"{}\"}} {}",
                plugin, hook, outcome, detail, v
            ));
        }
        if let Ok(rs) = self.reload_state.lock() {
            lines.push("# HELP panda_plugin_reload_total Successful runtime hot-reloads".to_string());
            lines.push("# TYPE panda_plugin_reload_total counter".to_string());
            lines.push(format!("panda_plugin_reload_total {}", rs.reload_count));
            lines.push("# HELP panda_plugin_reload_skipped_total Skipped reload scans".to_string());
            lines.push("# TYPE panda_plugin_reload_skipped_total counter".to_string());
            lines.push(format!(
                "panda_plugin_reload_skipped_total{{reason=\"debounce\"}} {}",
                rs.skipped_debounce
            ));
            lines.push(format!(
                "panda_plugin_reload_skipped_total{{reason=\"rate_limit\"}} {}",
                rs.skipped_rate_limit
            ));
        }
        lines.join("\n")
    }

    fn status_json(&self) -> serde_json::Value {
        let (reload_count, skipped_debounce, skipped_rate_limit, last_success_epoch_ms, last_error) =
            self.reload_state
                .lock()
                .map(|r| {
                    (
                        r.reload_count,
                        r.skipped_debounce,
                        r.skipped_rate_limit,
                        r.last_success_epoch_ms,
                        r.last_error.clone(),
                    )
                })
                .unwrap_or((0, 0, 0, None, Some("reload_state_lock_error".to_string())));
        serde_json::json!({
            "directory": self.dir.display().to_string(),
            "reload_count": reload_count,
            "skipped_debounce": skipped_debounce,
            "skipped_rate_limit": skipped_rate_limit,
            "last_success_epoch_ms": last_success_epoch_ms,
            "last_error": last_error,
        })
    }
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    #[allow(dead_code)]
    sub: Option<String>,
    #[allow(dead_code)]
    iss: Option<String>,
    #[allow(dead_code)]
    aud: Option<serde_json::Value>,
    #[allow(dead_code)]
    exp: usize,
}

/// Run until SIGINT (Ctrl+C). Binds per `config.listen` (HTTPS if `config.tls` is set).
pub async fn run(config: Arc<PandaConfig>) -> anyhow::Result<()> {
    let client = build_http_client()?;
    let tpm = Arc::new(TpmCounters::connect(config.effective_redis_url().as_deref()).await?);
    let bpe = match tiktoken_rs::cl100k_base() {
        Ok(b) => Some(Arc::new(b)),
        Err(e) => {
            eprintln!("tiktoken cl100k_base disabled: {e}");
            None
        }
    };

    let plugins = PluginRuntime::load_optional(
        config.plugins.directory.as_deref().map(std::path::Path::new),
    )?;
    let plugins = if let Some(p) = plugins {
        p.smoke_test();
        eprintln!("panda: wasm plugins loaded: {}", p.plugin_count());
        let dir = PathBuf::from(
            config
                .plugins
                .directory
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("plugins.directory missing"))?,
        );
        let mgr = Arc::new(PluginManager::new(
            dir,
            Arc::new(p),
            Duration::from_millis(config.plugins.reload_interval_ms),
            Duration::from_millis(config.plugins.reload_debounce_ms),
            config.plugins.max_reloads_per_minute as usize,
        )?);
        if config.plugins.hot_reload {
            mgr.spawn_hot_reload();
            eprintln!(
                "panda: wasm hot-reload enabled interval_ms={}",
                config.plugins.reload_interval_ms
            );
        }
        Some(mgr)
    } else {
        None
    };

    let state = Arc::new(ProxyState {
        config: Arc::clone(&config),
        client,
        tpm,
        bpe,
        plugins,
    });

    if let Some(ref tls_cfg) = config.tls {
        let tls = tls::server_config(tls_cfg)?;
        let acceptor = TlsAcceptor::from(tls);
        run_tls(state, acceptor).await
    } else {
        run_plain(state).await
    }
}

async fn run_plain(state: Arc<ProxyState>) -> anyhow::Result<()> {
    let addr = state.config.listen_addr()?;
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    eprintln!("panda listening on http://{local}");
    accept_loop(listener, state, None).await
}

async fn run_tls(state: Arc<ProxyState>, acceptor: TlsAcceptor) -> anyhow::Result<()> {
    let addr = state.config.listen_addr()?;
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    eprintln!("panda listening on https://{local} (TLS)");
    accept_loop(listener, state, Some(acceptor)).await
}

async fn accept_loop(
    listener: TcpListener,
    state: Arc<ProxyState>,
    tls: Option<TlsAcceptor>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutdown signal received");
                break;
            }
            r = listener.accept() => {
                let (stream, _) = r?;
                let st = Arc::clone(&state);
                if let Some(acc) = tls.clone() {
                    tokio::spawn(async move {
                        let stream = match acc.accept(stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("tls handshake failed: {e}");
                                return;
                            }
                        };
                        let io = TokioIo::new(stream);
                        let svc = service_fn(move |req| {
                            let st = Arc::clone(&st);
                            dispatch(req, st)
                        });
                        if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                            eprintln!("connection error: {e}");
                        }
                    });
                } else {
                    let io = TokioIo::new(stream);
                    tokio::spawn(async move {
                        let svc = service_fn(move |req| {
                            let st = Arc::clone(&st);
                            dispatch(req, st)
                        });
                        if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                            eprintln!("connection error: {e}");
                        }
                    });
                }
            }
        }
    }
    Ok(())
}

fn ensure_rustls_ring_provider() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("install rustls ring crypto provider");
    });
}

fn build_http_client() -> anyhow::Result<HttpClient> {
    ensure_rustls_ring_provider();

    let mut http = HttpConnector::new();
    http.set_connect_timeout(Some(std::time::Duration::from_secs(30)));
    http.enforce_http(false);

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);

    Ok(Client::builder(TokioExecutor::new()).build(https))
}

async fn dispatch(req: Request<Incoming>, state: Arc<ProxyState>) -> Result<Response<BoxBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path();

    if method == hyper::Method::GET && path == "/health" {
        return Ok(text_response(StatusCode::OK, "ok"));
    }
    if method == hyper::Method::GET && path == "/ready" {
        return Ok(text_response(StatusCode::OK, "ready"));
    }
    if method == hyper::Method::GET && path == "/metrics" {
        let body = state
            .plugins
            .as_ref()
            .map(|p| p.metrics_prometheus_text())
            .unwrap_or_else(|| "# panda plugins disabled\n".to_string());
        return Ok(text_with_content_type(
            StatusCode::OK,
            body,
            "text/plain; version=0.0.4; charset=utf-8",
        ));
    }
    if method == hyper::Method::GET && path == "/plugins/status" {
        let json = state
            .plugins
            .as_ref()
            .map(|p| p.status_json())
            .unwrap_or_else(|| serde_json::json!({"plugins_enabled": false}));
        return Ok(json_response(StatusCode::OK, json));
    }

    if let Err(resp) = enforce_jwt_if_required(&req, &state.config) {
        return Ok(resp);
    }

    match forward_to_upstream(req, state.as_ref()).await {
        Ok(resp) => Ok(resp),
        Err(e) => Ok(proxy_error_response(e)),
    }
}

fn enforce_jwt_if_required(req: &Request<Incoming>, cfg: &PandaConfig) -> Result<(), Response<BoxBody>> {
    if !cfg.identity.require_jwt {
        return Ok(());
    }
    if let Err(msg) = validate_bearer_jwt(req.headers(), cfg) {
        return Err(text_response(StatusCode::UNAUTHORIZED, msg));
    }
    Ok(())
}

fn validate_bearer_jwt(headers: &HeaderMap, cfg: &PandaConfig) -> Result<(), &'static str> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let token = auth
        .strip_prefix("Bearer ")
        .filter(|t| !t.trim().is_empty())
        .ok_or("unauthorized: missing bearer token")?;
    let secret = std::env::var(&cfg.identity.jwt_hs256_secret_env)
        .map_err(|_| "unauthorized: jwt secret not configured")?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    if !cfg.identity.accepted_issuers.is_empty() {
        validation.set_issuer(&cfg.identity.accepted_issuers);
    }
    if !cfg.identity.accepted_audiences.is_empty() {
        validation.set_audience(&cfg.identity.accepted_audiences);
    }
    if decode::<JwtClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation).is_err() {
        return Err("unauthorized: invalid bearer token");
    }
    Ok(())
}

enum ProxyError {
    PolicyReject(String),
    Upstream(anyhow::Error),
}

impl From<anyhow::Error> for ProxyError {
    fn from(value: anyhow::Error) -> Self {
        Self::Upstream(value)
    }
}

fn estimate_prompt_tokens_hint(headers: &HeaderMap) -> u64 {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| n.saturating_div(4))
        .unwrap_or(0)
}

fn tpm_bucket_key(ctx: &RequestContext) -> String {
    match (&ctx.subject, &ctx.tenant) {
        (Some(s), Some(t)) => format!("{s}@tenant:{t}"),
        (Some(s), None) => s.clone(),
        (None, Some(t)) => format!("anonymous@tenant:{t}"),
        (None, None) => "anonymous".to_string(),
    }
}

fn is_sse(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_lowercase().starts_with("text/event-stream"))
}

async fn forward_to_upstream(req: Request<Incoming>, state: &ProxyState) -> Result<Response<BoxBody>, ProxyError> {
    let uri = upstream::join_upstream_uri(&state.config.upstream, req.uri()).map_err(ProxyError::Upstream)?;

    let (mut parts, body) = req.into_parts();
    parts.uri = uri;
    let mut headers = HeaderMap::new();
    upstream::filter_request_headers(&parts.headers, &mut headers);
    if let Some(ref plugins) = state.plugins {
        let runtime = plugins.runtime_snapshot().await;
        match apply_wasm_headers_with_timeout(
            Arc::clone(&runtime),
            headers.clone(),
            state.config.plugins.execution_timeout_ms,
        )
        .await
        {
            Ok((next_headers, applied)) => {
                headers = next_headers;
                plugins.record_allow_all(runtime.as_ref(), "headers");
                if applied > 0 {
                    eprintln!("panda: wasm request headers applied: {applied}");
                }
            }
            Err(e) => {
                record_wasm_error_metrics(plugins, &e, "headers");
                if state.config.plugins.fail_closed {
                    return Err(proxy_error_from_wasm(e));
                }
                eprintln!("panda: wasm headers hook fail-open: {e:?}");
            }
        }
    }

    let correlation_id = gateway::ensure_correlation_id(
        &mut headers,
        &state.config.observability.correlation_header,
    )
    .map_err(ProxyError::Upstream)?;

    let secret = gateway::trusted_gateway_secret_from_env();
    let mut ctx = gateway::apply_trusted_gateway(&mut headers, &state.config.trusted_gateway, secret.as_deref());
    ctx.correlation_id = correlation_id;

    let est = estimate_prompt_tokens_hint(&headers);
    state
        .tpm
        .add_prompt_tokens(&tpm_bucket_key(&ctx), est)
        .await;

    log_request_context(&ctx);

    parts.headers = headers;

    let boxed_req_body: BoxBody = if let Some(ref plugins) = state.plugins {
        let runtime = plugins.runtime_snapshot().await;
        let content_len_ok = parts
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|n| n <= state.config.plugins.max_request_body_bytes);
        if content_len_ok {
            let collected = body
                .collect()
                .await
                .map_err(|e| ProxyError::Upstream(anyhow::anyhow!("collect body: {e}")))?;
            let original = collected.to_bytes();
            let next = match apply_wasm_body_with_timeout(
                Arc::clone(&runtime),
                original.to_vec(),
                state.config.plugins.max_request_body_bytes,
                state.config.plugins.execution_timeout_ms,
            )
            .await
            {
                Ok(b) => {
                    plugins.record_allow_all(runtime.as_ref(), "body");
                    b
                }
                Err(e) => {
                    record_wasm_error_metrics(plugins, &e, "body");
                    if state.config.plugins.fail_closed {
                        return Err(proxy_error_from_wasm(e));
                    }
                    eprintln!("panda: wasm body hook fail-open: {e:?}");
                    original.to_vec()
                }
            };
            Full::new(bytes::Bytes::from(next))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed_unsync()
        } else {
            body.map_err(|e| e).boxed_unsync()
        }
    } else {
        body.map_err(|e| e).boxed_unsync()
    };
    let req_up = Request::from_parts(parts, boxed_req_body);

    let resp = state
        .client
        .request(req_up)
        .await
        .map_err(|e| ProxyError::Upstream(anyhow::anyhow!("upstream request: {e}")))?;
    let (parts, body) = resp.into_parts();

    let mut out_headers = HeaderMap::new();
    upstream::filter_response_headers(&parts.headers, &mut out_headers);

    let corr_name = HeaderName::from_bytes(state.config.observability.correlation_header.as_bytes())
        .map_err(|_| ProxyError::Upstream(anyhow::anyhow!("correlation header name")))?;
    out_headers.insert(
        corr_name,
        HeaderValue::from_str(&ctx.correlation_id)
            .map_err(|_| ProxyError::Upstream(anyhow::anyhow!("correlation id header value")))?,
    );

    let bucket = tpm_bucket_key(&ctx);
    let body_in: BoxBody = if is_sse(&out_headers) {
        if let Some(ref bpe) = state.bpe {
            sse::SseCountingBody::new(body, Arc::clone(&state.tpm), bucket, Arc::clone(bpe))
                .map_err(|e| e)
                .boxed_unsync()
        } else {
            body.map_err(|e| e).boxed_unsync()
        }
    } else {
        body.map_err(|e| e).boxed_unsync()
    };

    let mut out = Response::builder()
        .status(parts.status)
        .body(body_in)
        .map_err(|e| ProxyError::Upstream(anyhow::anyhow!("response build: {e}")))?;
    *out.headers_mut() = out_headers;
    Ok(out)
}

fn proxy_error_from_wasm(e: WasmCallError) -> ProxyError {
    match e {
        WasmCallError::Hook(HookFailure::PolicyReject { plugin, code }) => {
            ProxyError::PolicyReject(format!("plugin={plugin} code={code:?}"))
        }
        WasmCallError::Hook(HookFailure::Runtime { message, .. }) => {
            ProxyError::Upstream(anyhow::anyhow!("{message}"))
        }
        WasmCallError::Timeout(msg) => ProxyError::Upstream(anyhow::anyhow!("{msg}")),
        WasmCallError::Join(msg) => ProxyError::Upstream(anyhow::anyhow!("{msg}")),
    }
}

fn record_wasm_error_metrics(manager: &PluginManager, e: &WasmCallError, hook: &str) {
    match e {
        WasmCallError::Hook(HookFailure::PolicyReject { plugin, code }) => {
            manager.record_policy_reject(plugin, &format!("{code:?}"), hook);
        }
        WasmCallError::Hook(HookFailure::Runtime { plugin, reason, .. }) => {
            manager.record_runtime(plugin, *reason, hook);
        }
        WasmCallError::Timeout(_) => manager.record_timeout(hook),
        WasmCallError::Join(_) => manager.record_runtime("_all", RuntimeReason::Internal, hook),
    }
}

fn proxy_error_response(e: ProxyError) -> Response<BoxBody> {
    match e {
        ProxyError::PolicyReject(msg) => {
            eprintln!("policy reject: {msg}");
            text_response(StatusCode::FORBIDDEN, "forbidden: request rejected by policy")
        }
        ProxyError::Upstream(err) => {
            eprintln!("upstream error: {err:#}");
            text_response(StatusCode::BAD_GATEWAY, "bad gateway: upstream request failed")
        }
    }
}

async fn apply_wasm_headers_with_timeout(
    plugins: Arc<PluginRuntime>,
    headers: HeaderMap,
    timeout_ms: u64,
) -> Result<(HeaderMap, usize), WasmCallError> {
    tokio::time::timeout(Duration::from_millis(timeout_ms), tokio::task::spawn_blocking(move || {
        let mut h = headers;
        let n = plugins.apply_request_plugins_strict(&mut h)?;
        Ok::<_, HookFailure>((h, n))
    }))
    .await
    .map_err(|_| WasmCallError::Timeout("wasm request header hook timeout".to_string()))?
    .map_err(|e| WasmCallError::Join(format!("wasm request header hook join error: {e}")))?
    .map_err(WasmCallError::Hook)
}

async fn apply_wasm_body_with_timeout(
    plugins: Arc<PluginRuntime>,
    original: Vec<u8>,
    max_output_bytes: usize,
    timeout_ms: u64,
) -> Result<Vec<u8>, WasmCallError> {
    tokio::time::timeout(Duration::from_millis(timeout_ms), tokio::task::spawn_blocking(move || {
        let replacement = plugins.apply_request_body_plugins_strict(&original, max_output_bytes)?;
        Ok::<_, HookFailure>(replacement.unwrap_or(original))
    }))
    .await
    .map_err(|_| WasmCallError::Timeout("wasm request body hook timeout".to_string()))?
    .map_err(|e| WasmCallError::Join(format!("wasm request body hook join error: {e}")))?
    .map_err(WasmCallError::Hook)
}

#[derive(Debug)]
enum WasmCallError {
    Hook(HookFailure),
    Timeout(String),
    Join(String),
}

fn dir_fingerprint(dir: &Path) -> anyhow::Result<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
            continue;
        }
        let md = ent.metadata()?;
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .hash(&mut hasher);
        md.len().hash(&mut hasher);
        if let Ok(m) = md.modified() {
            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                d.as_secs().hash(&mut hasher);
                d.subsec_nanos().hash(&mut hasher);
            }
        }
    }
    Ok(hasher.finish())
}

fn log_request_context(ctx: &RequestContext) {
    if ctx.correlation_id.is_empty() {
        return;
    }
    if !ctx.trusted_hop && ctx.subject.is_none() && ctx.tenant.is_none() {
        eprintln!("panda req correlation_id={}", ctx.correlation_id);
        return;
    }
    eprintln!(
        "panda req correlation_id={} trusted={} subject={:?} tenant={:?} scopes={:?}",
        ctx.correlation_id, ctx.trusted_hop, ctx.subject, ctx.tenant, ctx.scopes
    );
}

fn text_response(status: StatusCode, msg: &str) -> Response<BoxBody> {
    text_with_content_type(
        status,
        msg.to_string(),
        "text/plain; charset=utf-8",
    )
}

fn text_with_content_type(status: StatusCode, msg: String, content_type: &'static str) -> Response<BoxBody> {
    let body = Full::new(bytes::Bytes::copy_from_slice(msg.as_bytes()))
        .map_err(|never: std::convert::Infallible| match never {})
        .boxed_unsync();
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            http::header::HeaderValue::from_static(content_type),
        )
        .body(body)
        .unwrap()
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<BoxBody> {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{\"error\":\"serialization\"}".to_string());
    text_with_content_type(status, body, "application/json; charset=utf-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration as StdDuration, SystemTime as StdSystemTime, UNIX_EPOCH};

    use hyper::body::Incoming as HyperIncoming;
    use hyper::service::service_fn;
    use hyper::Request;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use panda_wasm::{HookFailure, PolicyCode};

    #[test]
    fn config_roundtrip_for_listener() {
        let cfg = Arc::new(
            PandaConfig::from_yaml_str(
                "listen: '127.0.0.1:0'\nupstream: 'http://127.0.0.1:1'\n",
            )
            .unwrap(),
        );
        assert!(cfg.listen_addr().is_ok());
    }

    #[test]
    fn tpm_bucket_key_formats() {
        let mut a = RequestContext::default();
        assert_eq!(super::tpm_bucket_key(&a), "anonymous");
        a.subject = Some("u1".into());
        assert_eq!(super::tpm_bucket_key(&a), "u1");
        a.tenant = Some("t9".into());
        assert_eq!(super::tpm_bucket_key(&a), "u1@tenant:t9");
    }

    #[test]
    fn policy_reject_maps_to_403() {
        let err = proxy_error_from_wasm(WasmCallError::Hook(HookFailure::PolicyReject {
            plugin: "demo".to_string(),
            code: PolicyCode::Denied,
        }));
        let resp = proxy_error_response(err);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn runtime_failure_maps_to_502() {
        let err = proxy_error_from_wasm(WasmCallError::Hook(HookFailure::Runtime {
            plugin: "demo".to_string(),
            reason: RuntimeReason::Internal,
            message: "boom".to_string(),
        }));
        let resp = proxy_error_response(err);
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn end_to_end_fail_closed_policy_reject_returns_403() {
        let upstream_hits = StdArc::new(AtomicUsize::new(0));
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_hits_task = StdArc::clone(&upstream_hits);
        let upstream_task = tokio::spawn(async move {
            let (stream, _) = upstream_listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let svc = service_fn(move |_req: Request<HyperIncoming>| {
                let upstream_hits_task = StdArc::clone(&upstream_hits_task);
                async move {
                    upstream_hits_task.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(text_response(StatusCode::OK, "upstream-ok"))
                }
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        });

        let plugins_dir = tempfile::tempdir().unwrap();
        let reject_wasm = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "panda_abi_version") (result i32) i32.const 0)
                (func (export "panda_on_request") (result i32) i32.const 1)
            )"#,
        )
        .unwrap();
        std::fs::write(plugins_dir.path().join("reject.wasm"), reject_wasm).unwrap();

        let cfg = Arc::new(PandaConfig {
            listen: "127.0.0.1:0".to_string(),
            upstream: format!("http://{upstream_addr}"),
            trusted_gateway: Default::default(),
            observability: Default::default(),
            tpm: Default::default(),
            tls: None,
            plugins: panda_config::PluginsConfig {
                directory: Some(plugins_dir.path().display().to_string()),
                max_request_body_bytes: 262_144,
                execution_timeout_ms: 50,
                fail_closed: true,
                hot_reload: false,
                reload_interval_ms: 2_000,
                reload_debounce_ms: 500,
                max_reloads_per_minute: 30,
            },
            identity: Default::default(),
        });

        let runtime = PluginRuntime::load_optional(
            cfg.plugins.directory.as_deref().map(std::path::Path::new),
        )
        .unwrap()
        .map(Arc::new)
        .unwrap();

        let state = Arc::new(ProxyState {
            config: Arc::clone(&cfg),
            client: build_http_client().unwrap(),
            tpm: Arc::new(TpmCounters::connect(None).await.unwrap()),
            bpe: None,
            plugins: Some(Arc::new(
                PluginManager::new(
                    PathBuf::from(cfg.plugins.directory.as_deref().unwrap()),
                    runtime,
                    Duration::from_millis(cfg.plugins.reload_interval_ms),
                    Duration::from_millis(cfg.plugins.reload_debounce_ms),
                    cfg.plugins.max_reloads_per_minute as usize,
                )
                .unwrap(),
            )),
        });
        assert!(state.plugins.is_some());
        assert!(state.config.plugins.fail_closed);

        let panda_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let panda_addr = panda_listener.local_addr().unwrap();
        let st = Arc::clone(&state);
        let panda_task = tokio::spawn(async move {
            let (stream, _) = panda_listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let st = Arc::clone(&st);
                dispatch(req, st)
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        });

        let mut client = TcpStream::connect(panda_addr).await.unwrap();
        client
            .write_all(b"GET /v1/chat HTTP/1.1\r\nHost: panda\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.contains("403 Forbidden"), "{response}");

        panda_task.await.unwrap();
        upstream_task.abort();
        assert_eq!(upstream_hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn jwt_validation_rejects_missing_token() {
        let cfg = PandaConfig {
            listen: "127.0.0.1:0".to_string(),
            upstream: "http://127.0.0.1:1".to_string(),
            trusted_gateway: Default::default(),
            observability: Default::default(),
            tpm: Default::default(),
            tls: None,
            plugins: Default::default(),
            identity: panda_config::IdentityConfig {
                require_jwt: true,
                jwt_hs256_secret_env: "PANDA_TEST_JWT_SECRET".to_string(),
                accepted_issuers: vec![],
                accepted_audiences: vec![],
            },
        };
        let headers = HeaderMap::new();
        let err = validate_bearer_jwt(&headers, &cfg).unwrap_err();
        assert!(err.contains("missing bearer token"));
    }

    #[test]
    fn jwt_validation_accepts_valid_token() {
        #[derive(serde::Serialize)]
        struct Claims {
            sub: &'static str,
            iss: &'static str,
            aud: &'static str,
            exp: usize,
        }
        let secret_env = "PANDA_TEST_JWT_SECRET_VALID";
        // SAFETY: test-only process env setup.
        unsafe {
            std::env::set_var(secret_env, "test-secret");
        }
        let exp = (StdSystemTime::now() + StdDuration::from_secs(300))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let token = encode(
            &Header::default(),
            &Claims {
                sub: "u1",
                iss: "https://issuer.example",
                aud: "panda-gateway",
                exp,
            },
            &EncodingKey::from_secret("test-secret".as_bytes()),
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        let cfg = PandaConfig {
            listen: "127.0.0.1:0".to_string(),
            upstream: "http://127.0.0.1:1".to_string(),
            trusted_gateway: Default::default(),
            observability: Default::default(),
            tpm: Default::default(),
            tls: None,
            plugins: Default::default(),
            identity: panda_config::IdentityConfig {
                require_jwt: true,
                jwt_hs256_secret_env: secret_env.to_string(),
                accepted_issuers: vec!["https://issuer.example".to_string()],
                accepted_audiences: vec!["panda-gateway".to_string()],
            },
        };

        assert!(validate_bearer_jwt(&headers, &cfg).is_ok());
    }
}
