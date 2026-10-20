//! The address a request came from, for rate limits. Proxy headers are trusted only when the
//! operator says a proxy is in front (FORUM_TRUST_PROXY=true).

use std::net::SocketAddr;

use axum::http::HeaderMap;

pub fn client_ip(headers: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> String {
    let forwarded = if trust_proxy {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|ip| !ip.is_empty() && ip.len() <= 45)
    } else {
        None
    };
    match forwarded {
        Some(ip) => ip.to_owned(),
        None => peer.ip().to_string(),
    }
}
