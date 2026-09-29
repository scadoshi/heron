//! Rate-limit key extraction.

use axum::extract::ConnectInfo;
use std::net::{IpAddr, SocketAddr};
use tower_governor::{GovernorError, key_extractor::KeyExtractor};

/// Header Cloudflare sets to the client's address.
const CF_CONNECTING_IP: &str = "cf-connecting-ip";

/// Keys the rate limit on the client's address behind Cloudflare.
///
/// Every request through a Cloudflare Tunnel arrives from `127.0.0.1`, so keying on
/// the TCP peer would put all clients in one bucket. Cloudflare sets
/// `CF-Connecting-IP` to the client's address and overwrites whatever the client
/// sent, and the origin is not reachable from the internet except through the
/// tunnel, so the header is trusted.
///
/// Without the header the key is the TCP peer. That is a request from the box itself.
#[derive(Debug, Clone)]
pub struct CfConnectingIpKeyExtractor;

impl KeyExtractor for CfConnectingIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, request: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(ip) = request
            .headers()
            .get(CF_CONNECTING_IP)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return Ok(ip);
        }
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip())
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn peer(ip: [u8; 4]) -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), 40000))
    }

    fn request(
        header: Option<&str>,
        peer: Option<ConnectInfo<SocketAddr>>,
    ) -> axum::http::Request<()> {
        let mut builder = axum::http::Request::builder();
        if let Some(header) = header {
            builder = builder.header(CF_CONNECTING_IP, header);
        }
        let mut request = builder.body(()).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(peer);
        }
        request
    }

    fn key(header: Option<&str>, peer: Option<ConnectInfo<SocketAddr>>) -> Option<IpAddr> {
        CfConnectingIpKeyExtractor
            .extract(&request(header, peer))
            .ok()
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn the_header_wins_over_the_peer() {
        assert_eq!(
            key(Some("203.0.113.7"), Some(peer([127, 0, 0, 1]))),
            Some(ip("203.0.113.7"))
        );
    }

    #[test]
    fn clients_behind_the_same_peer_get_separate_keys() {
        let tunnel = || Some(peer([127, 0, 0, 1]));
        assert_ne!(
            key(Some("203.0.113.7"), tunnel()),
            key(Some("198.51.100.4"), tunnel())
        );
    }

    #[test]
    fn the_header_is_trimmed_and_may_be_ipv6() {
        assert_eq!(key(Some(" 203.0.113.7 "), None), Some(ip("203.0.113.7")));
        assert_eq!(key(Some("2001:db8::1"), None), Some(ip("2001:db8::1")));
    }

    #[test]
    fn falls_back_to_the_peer_without_a_usable_header() {
        assert_eq!(
            key(None, Some(peer([100, 64, 0, 9]))),
            Some(ip("100.64.0.9"))
        );
        assert_eq!(
            key(Some("not-an-ip"), Some(peer([100, 64, 0, 9]))),
            Some(ip("100.64.0.9"))
        );
    }

    #[test]
    fn no_header_and_no_peer_is_an_error() {
        assert_eq!(key(None, None), None);
    }
}
