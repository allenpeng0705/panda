//! MCP **Streamable HTTP** transport helpers ([spec 2025-03-26](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#streamable-http)):
//! in-memory session IDs, Origin checks, and GET listener SSE (keepalive).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::stream::{self, StreamExt};
use http::header::{self, HeaderMap, HeaderValue};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::{Response, StatusCode};
use tokio::time::MissedTickBehavior;
use tokio_stream::wrappers::IntervalStream;

use crate::{text_response, BoxBody};

/// HTTP header carrying the MCP session id (case-insensitive; canonical form per spec examples).
pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

#[derive(Debug)]
pub struct McpStreamableSessionStore {
    inner: Mutex<HashMap<String, Instant>>,
    ttl: Duration,
}

impl Default for McpStreamableSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpStreamableSessionStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(24 * 3600),
        }
    }

    fn prune(&self, map: &mut HashMap<String, Instant>) {
        let now = Instant::now();
        map.retain(|_, t| now.duration_since(*t) < self.ttl);
    }

    pub fn create_session(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut g = self.inner.lock().expect("mcp session mutex");
        self.prune(&mut g);
        g.insert(id.clone(), Instant::now());
        id
    }

    pub fn remove_session(&self, id: &str) -> bool {
        let mut g = self.inner.lock().expect("mcp session mutex");
        self.prune(&mut g);
        g.remove(id).is_some()
    }

    /// Returns false if unknown or expired session.
    pub fn validate_and_touch(&self, id: &str) -> bool {
        let mut g = self.inner.lock().expect("mcp session mutex");
        self.prune(&mut g);
        if g.contains_key(id) {
            g.insert(id.to_string(), Instant::now());
            true
        } else {
            false
        }
    }
}

pub(crate) fn read_session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::string::ToString::to_string)
}

/// When `Origin` is present, require it to match `Host` (scheme-relative compare on host[:port]).
/// Absent `Origin` is allowed (non-browser MCP clients).
pub(crate) fn validate_origin_for_streamable<B>(
    req: &http::Request<B>,
) -> Result<(), &'static str> {
    let Some(origin_raw) = req.headers().get(header::ORIGIN) else {
        return Ok(());
    };
    let Ok(origin) = origin_raw.to_str() else {
        return Err("invalid Origin header encoding");
    };
    let origin = origin.trim();
    if origin.is_empty() || origin.eq_ignore_ascii_case("null") {
        return Ok(());
    }
    let after_scheme = if let Some(idx) = origin.find("://") {
        &origin[idx + 3..]
    } else {
        origin
    };
    let origin_host_port = after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .split('?')
        .next()
        .unwrap_or(after_scheme)
        .to_ascii_lowercase();

    let Some(host_raw) = req.headers().get(header::HOST) else {
        return Err("missing Host header for Origin validation");
    };
    let Ok(host) = host_raw.to_str() else {
        return Err("invalid Host header encoding");
    };
    let host_norm = host.trim().to_ascii_lowercase();
    if origin_host_port != host_norm {
        return Err("Origin does not match Host");
    }
    Ok(())
}

/// Long-lived SSE stream for GET listener (keepalive only; server-push can be added later).
pub(crate) fn mcp_streamable_get_listener_response(
    last_event_id: Option<&str>,
) -> Response<BoxBody> {
    let _ = last_event_id;
    let mut interval = tokio::time::interval(Duration::from_secs(20));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let open = stream::once(async {
        Ok::<_, Infallible>(Frame::data(Bytes::from_static(b": mcp-listener\n\n")))
    });
    let pings = IntervalStream::new(interval)
        .map(|_| Ok::<_, Infallible>(Frame::data(Bytes::from_static(b": ping\n\n"))));
    let stream = open.chain(pings);
    let body = StreamBody::new(stream)
        .map_err(|never: Infallible| match never {})
        .boxed_unsync();
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(body)
        .unwrap()
}

pub(crate) fn mcp_origin_rejection_response() -> Response<BoxBody> {
    text_response(
        StatusCode::FORBIDDEN,
        "Origin header rejected (MCP Streamable HTTP DNS rebinding protection)",
    )
}

pub(crate) fn mcp_accept_missing_stream_response() -> Response<BoxBody> {
    text_response(
        StatusCode::NOT_ACCEPTABLE,
        "Accept must include text/event-stream for MCP GET listener",
    )
}

pub(crate) fn mcp_session_header_missing_response() -> Response<BoxBody> {
    text_response(
        StatusCode::BAD_REQUEST,
        "Mcp-Session-Id header required for this MCP request",
    )
}

pub(crate) fn mcp_session_unknown_response() -> Response<BoxBody> {
    text_response(StatusCode::NOT_FOUND, "unknown or expired Mcp-Session-Id")
}
