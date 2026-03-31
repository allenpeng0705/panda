//! HTTP reverse proxy with streaming bodies (SSE-friendly).
//!
//! [`panda_config::PandaConfig`] supplies the upstream base URL; this crate does not read YAML.

mod upstream;

use std::convert::Infallible;
use std::sync::Arc;

use http::header::{self, HeaderMap};
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
use tokio::net::TcpListener;

type BoxBody = UnsyncBoxBody<bytes::Bytes, hyper::Error>;
type HttpClient = Client<HttpsConnector<HttpConnector>, BoxBody>;

/// Shared state for each connection handler.
pub struct ProxyState {
    pub config: Arc<PandaConfig>,
    pub client: HttpClient,
}

/// Run until SIGINT (Ctrl+C). Binds per `config.listen`.
pub async fn run(config: Arc<PandaConfig>) -> anyhow::Result<()> {
    let client = build_http_client()?;
    let state = Arc::new(ProxyState { config, client });

    let addr = state.config.listen_addr()?;
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    eprintln!("panda listening on http://{local}");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutdown signal received");
                break;
            }
            accept = listener.accept() => {
                let (stream, _) = accept?;
                let io = TokioIo::new(stream);
                let st = Arc::clone(&state);
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
        Err(e) => {
            eprintln!("upstream error: {e:#}");
            Ok(text_response(
                StatusCode::BAD_GATEWAY,
                "bad gateway: upstream request failed",
            ))
        }
    }
}

async fn forward_to_upstream(req: Request<Incoming>, state: &ProxyState) -> anyhow::Result<Response<BoxBody>> {
    let uri = upstream::join_upstream_uri(&state.config.upstream, req.uri())?;

    let (mut parts, body) = req.into_parts();
    parts.uri = uri;
    let mut headers = HeaderMap::new();
    upstream::filter_request_headers(&parts.headers, &mut headers);
    parts.headers = headers;

    let boxed_req_body = body.map_err(|e| e).boxed_unsync();
    let req_up = Request::from_parts(parts, boxed_req_body);

    let resp = state.client.request(req_up).await?;
    let (parts, body) = resp.into_parts();

    let mut out_headers = HeaderMap::new();
    upstream::filter_response_headers(&parts.headers, &mut out_headers);

    let boxed_resp_body = body.map_err(|e| e).boxed_unsync();
    let mut out = Response::builder()
        .status(parts.status)
        .body(boxed_resp_body)?;
    *out.headers_mut() = out_headers;
    Ok(out)
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
}
