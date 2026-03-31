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
use std::sync::Arc;
use std::time::Duration;

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
use panda_config::PandaConfig;
use panda_wasm::{HookFailure, PluginRuntime};
use tokio::net::TcpListener;
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
    /// Loaded when `plugins.directory` is set; reserved for per-request Wasm hooks.
    #[allow(dead_code)]
    pub plugins: Option<Arc<PluginRuntime>>,
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
    if let Some(ref p) = plugins {
        p.smoke_test();
        eprintln!("panda: wasm plugins loaded: {}", p.plugin_count());
    }

    let state = Arc::new(ProxyState {
        config: Arc::clone(&config),
        client,
        tpm,
        bpe,
        plugins: plugins.map(Arc::new),
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

    match forward_to_upstream(req, state.as_ref()).await {
        Ok(resp) => Ok(resp),
        Err(e) => Ok(proxy_error_response(e)),
    }
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
        match apply_wasm_headers_with_timeout(
            Arc::clone(plugins),
            headers.clone(),
            state.config.plugins.execution_timeout_ms,
        )
        .await
        {
            Ok((next_headers, applied)) => {
                headers = next_headers;
                if applied > 0 {
                    eprintln!("panda: wasm request headers applied: {applied}");
                }
            }
            Err(e) => {
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
                Arc::clone(plugins),
                original.to_vec(),
                state.config.plugins.max_request_body_bytes,
                state.config.plugins.execution_timeout_ms,
            )
            .await
            {
                Ok(b) => b,
                Err(e) => {
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
        WasmCallError::Hook(HookFailure::Runtime(inner)) => ProxyError::Upstream(inner),
        WasmCallError::Timeout(msg) => ProxyError::Upstream(anyhow::anyhow!("{msg}")),
        WasmCallError::Join(msg) => ProxyError::Upstream(anyhow::anyhow!("{msg}")),
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
    let body = Full::new(bytes::Bytes::copy_from_slice(msg.as_bytes()))
        .map_err(|never: std::convert::Infallible| match never {})
        .boxed_unsync();
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(body)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hyper::body::Incoming as HyperIncoming;
    use hyper::service::service_fn;
    use hyper::Request;
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
        let err = proxy_error_from_wasm(WasmCallError::Hook(HookFailure::Runtime(anyhow::anyhow!(
            "boom"
        ))));
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
            },
        });

        let state = Arc::new(ProxyState {
            config: Arc::clone(&cfg),
            client: build_http_client().unwrap(),
            tpm: Arc::new(TpmCounters::connect(None).await.unwrap()),
            bpe: None,
            plugins: PluginRuntime::load_optional(
                cfg.plugins.directory.as_deref().map(std::path::Path::new),
            )
            .unwrap()
            .map(Arc::new),
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
}
