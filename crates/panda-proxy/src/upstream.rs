//! Join client URIs with configured upstream base and filter hop-by-hop headers.

use http::header::{self, HeaderMap, HeaderName};
use hyper::Uri;

/// Append the request path and query to `upstream_base` (no trailing slash).
pub fn join_upstream_uri(upstream_base: &str, req_uri: &Uri) -> anyhow::Result<Uri> {
    let base = upstream_base.trim_end_matches('/');
    let suffix = req_uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let joined = format!("{base}{suffix}");
    joined
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid joined upstream URI: {e}"))
}

/// Request headers we never forward (hop-by-hop or set from target URI).
pub fn filter_request_headers(src: &HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src.iter() {
        if is_hop_by_hop_request(name) {
            continue;
        }
        if name == header::HOST {
            continue;
        }
        dst.append(name.clone(), value.clone());
    }
}

/// Strip hop-by-hop and framing headers from upstream response; Hyper sets framing for the client.
pub fn filter_response_headers(src: &HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src.iter() {
        if is_hop_by_hop_response(name) {
            continue;
        }
        dst.append(name.clone(), value.clone());
    }
}

fn is_hop_by_hop_request(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_hop_by_hop_response(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    ) || name == header::CONTENT_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_preserves_path_and_query() {
        let u = join_upstream_uri(
            "http://127.0.0.1:11434",
            &"/v1/chat/completions?x=1".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(u.to_string(), "http://127.0.0.1:11434/v1/chat/completions?x=1");
    }

    #[test]
    fn join_strips_trailing_slash_on_base() {
        let u = join_upstream_uri("https://api.openai.com/v1/", &"/chat/completions".parse().unwrap())
            .unwrap();
        assert_eq!(u.to_string(), "https://api.openai.com/v1/chat/completions");
    }
}
