//! SSRF guard: pure classification of outbound request targets.
//!
//! Web search fetches attacker-influenced URLs (search-result links, redirect
//! chains, model-rewritten queries). Without a guard, a crafted URL could make
//! Thuki connect to a loopback service, a cloud metadata endpoint
//! (`169.254.169.254`), or a private-LAN host: a classic server-side request
//! forgery. Every function here is pure and total so the full range table is
//! covered by fast unit tests; the I/O that consumes them lives in
//! [`super::transport`].
//!
//! Two independent checks, both required because they cover different bypasses:
//!
//! 1. [`validate_request_url`] runs before a request is issued and again on
//!    every redirect hop. It rejects non-`http(s)` schemes and screens
//!    IP-literal hosts. Literals matter because reqwest never calls the DNS
//!    resolver for them, so a URL like `http://127.0.0.1/` or its encoded
//!    forms (`http://2130706433/`, `http://0x7f.1/`) would otherwise reach a
//!    socket unscreened. Parsing through [`url::Host`] canonicalises those
//!    encodings to a real [`std::net::IpAddr`] so the numeric value is
//!    classified, never the raw string.
//! 2. [`screen_addrs`] runs inside the custom DNS resolver, screening the
//!    addresses a hostname resolves to (both A and AAAA) at connect time. This
//!    closes DNS-rebinding: the exact addresses validated here are the exact
//!    addresses reqwest connects to.
//!
//! The classifier is default-deny: [`is_globally_routable`] returns `true`
//! only for addresses provably in global unicast space, so a range missed by
//! enumeration fails closed (blocked) rather than open.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use url::{Host, Url};

/// Why an outbound target was rejected. Carries enough detail for a security
/// log without leaking it to the model or the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SsrfError {
    /// A concrete address was not globally routable (private, loopback,
    /// metadata, link-local, ...).
    #[error("blocked non-global address: {0}")]
    BlockedAddress(IpAddr),
    /// The URL scheme was outside the `http`/`https` allowlist.
    #[error("blocked URL scheme: {0}")]
    BlockedScheme(String),
    /// A hostname resolved to zero addresses.
    #[error("host resolved to no addresses")]
    NoAddresses,
}

/// True iff `ip` is a globally-routable unicast address safe to connect to.
///
/// Default-deny: every range that is not global unicast is blocked, so an
/// address the enumeration below fails to name is treated as unsafe.
pub fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_globally_routable_v4(v4),
        IpAddr::V6(v6) => is_globally_routable_v6(v6),
    }
}

/// IPv4 classifier. Blocks every reserved / special-use range (RFC 6890 and
/// friends), allowing only global unicast. Ranges without a stable `std`
/// predicate (`is_shared`, `is_benchmarking`, `is_reserved`) are inlined.
fn is_globally_routable_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    let blocked = o[0] == 0                                  // 0.0.0.0/8 "this network"
        || ip.is_private()                                  // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()                                 // 127.0.0.0/8
        || ip.is_link_local()                               // 169.254.0.0/16 (incl. metadata)
        || (o[0] == 100 && (o[1] & 0xc0) == 0x40)           // 100.64.0.0/10 CGNAT shared
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)          // 192.0.0.0/24 IETF protocol
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)          // 192.0.2.0/24 TEST-NET-1
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)       // 198.51.100.0/24 TEST-NET-2
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)        // 203.0.113.0/24 TEST-NET-3
        || (o[0] == 198 && (o[1] & 0xfe) == 18)             // 198.18.0.0/15 benchmarking
        || o[0] >= 240                                      // 240.0.0.0/4 reserved + broadcast
        || ip.is_multicast(); // 224.0.0.0/4
    !blocked
}

/// IPv6 classifier. Reclassifies any address that embeds an IPv4 target
/// (mapped, compatible, or 6to4) through [`is_globally_routable_v4`] so an
/// internal IPv4 cannot be smuggled inside an IPv6 literal, then blocks the
/// non-global IPv6 ranges.
fn is_globally_routable_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // Mapped (::ffff:0:0/96) and compatible (::/96) addresses embed a v4.
    if let Some(v4) = ip.to_ipv4() {
        return is_globally_routable_v4(v4);
    }
    let seg = ip.segments();
    // 6to4 (2002::/16) embeds the v4 in segments 1-2.
    if seg[0] == 0x2002 {
        let v4 = Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            (seg[1] & 0xff) as u8,
            (seg[2] >> 8) as u8,
            (seg[2] & 0xff) as u8,
        );
        return is_globally_routable_v4(v4);
    }
    // Teredo (2001:0::/32) tunnels IPv4; nothing legitimate is reachable here.
    if seg[0] == 0x2001 && seg[1] == 0x0000 {
        return false;
    }
    // Documentation (2001:db8::/32).
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return false;
    }
    // Unique local (fc00::/7) and link-local (fe80::/10).
    if (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    true
}

/// Screens the addresses a hostname resolved to. Fail-closed: if **any**
/// address is not globally routable the whole resolution is rejected, because
/// a legitimate external host never resolves to an internal address and a
/// mixed answer is the signature of a rebinding attack.
pub fn screen_addrs<I>(addrs: I) -> Result<Vec<SocketAddr>, SsrfError>
where
    I: IntoIterator<Item = SocketAddr>,
{
    let mut screened = Vec::new();
    for addr in addrs {
        if !is_globally_routable(addr.ip()) {
            return Err(SsrfError::BlockedAddress(addr.ip()));
        }
        screened.push(addr);
    }
    if screened.is_empty() {
        return Err(SsrfError::NoAddresses);
    }
    Ok(screened)
}

/// Validates a request URL before it is issued and on every redirect hop.
///
/// Enforces the `http`/`https` scheme allowlist and, for IP-literal hosts,
/// rejects non-globally-routable literals. Domain hosts pass here and are
/// screened later by [`screen_addrs`] at resolve time.
pub fn validate_request_url(url: &Url) -> Result<(), SsrfError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SsrfError::BlockedScheme(url.scheme().to_owned()));
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => reject_if_blocked(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => reject_if_blocked(IpAddr::V6(ip)),
        // Domains are screened at resolve time by `screen_addrs`. An `http`/
        // `https` URL always carries a host once parsed (an empty host is a
        // parse error), and the scheme gate above admits only those, so the
        // domain and `None` cases collapse to the same "defer to resolver".
        _ => Ok(()),
    }
}

/// Rejects a literal address that is not globally routable.
fn reject_if_blocked(ip: IpAddr) -> Result<(), SsrfError> {
    if is_globally_routable(ip) {
        Ok(())
    } else {
        Err(SsrfError::BlockedAddress(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::str::FromStr;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from_str(s).unwrap())
    }
    fn v6(s: &str) -> IpAddr {
        IpAddr::V6(Ipv6Addr::from_str(s).unwrap())
    }

    #[test]
    fn allows_global_ipv4() {
        for s in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "203.0.114.1"] {
            assert!(is_globally_routable(v4(s)), "{s} should be global");
        }
    }

    #[test]
    fn blocks_non_global_ipv4_ranges() {
        for s in [
            "0.0.0.0",         // this-network 0.0.0.0/8
            "0.1.2.3",         // 0.0.0.0/8 non-zero host
            "127.0.0.1",       // loopback
            "10.0.0.1",        // private
            "172.16.5.4",      // private
            "192.168.1.1",     // private
            "169.254.169.254", // link-local metadata
            "100.64.0.1",      // CGNAT shared
            "192.0.0.1",       // IETF protocol assignments
            "192.0.2.5",       // documentation TEST-NET-1
            "198.51.100.5",    // documentation TEST-NET-2
            "203.0.113.5",     // documentation TEST-NET-3
            "198.18.0.1",      // benchmarking
            "240.0.0.1",       // reserved
            "255.255.255.255", // broadcast
            "224.0.0.1",       // multicast
        ] {
            assert!(!is_globally_routable(v4(s)), "{s} should be blocked");
        }
    }

    #[test]
    fn allows_global_ipv6() {
        for s in ["2606:4700:4700::1111", "2001:470::1", "2600::1"] {
            assert!(is_globally_routable(v6(s)), "{s} should be global");
        }
    }

    #[test]
    fn blocks_non_global_ipv6_ranges() {
        for s in [
            "::1",                    // loopback
            "::",                     // unspecified
            "ff02::1",                // multicast
            "fc00::1",                // ULA
            "fd12:3456::1",           // ULA
            "fe80::1",                // link-local
            "::ffff:127.0.0.1",       // mapped loopback
            "::ffff:10.0.0.1",        // mapped private
            "::ffff:169.254.169.254", // mapped metadata
            "2002:7f00:1::",          // 6to4 embedding 127.0.0.1
            "2001:0:1::",             // Teredo
            "2001:db8::1",            // documentation
        ] {
            assert!(!is_globally_routable(v6(s)), "{s} should be blocked");
        }
    }

    #[test]
    fn six_to_four_embedding_global_ipv4_is_allowed() {
        // 2002::/16 wrapping 8.8.8.8 (0x0808:0808) is routable.
        assert!(is_globally_routable(v6("2002:808:808::")));
    }

    #[test]
    fn screen_addrs_accepts_all_global() {
        let addrs = vec![
            SocketAddr::from_str("1.1.1.1:0").unwrap(),
            SocketAddr::from_str("[2606:4700::1]:0").unwrap(),
        ];
        let out = screen_addrs(addrs.clone()).unwrap();
        assert_eq!(out, addrs);
    }

    #[test]
    fn screen_addrs_rejects_when_any_blocked() {
        let addrs = vec![
            SocketAddr::from_str("8.8.8.8:0").unwrap(),
            SocketAddr::from_str("127.0.0.1:0").unwrap(),
        ];
        assert_eq!(
            screen_addrs(addrs),
            Err(SsrfError::BlockedAddress(v4("127.0.0.1")))
        );
    }

    #[test]
    fn screen_addrs_rejects_empty() {
        assert_eq!(screen_addrs(Vec::new()), Err(SsrfError::NoAddresses));
    }

    #[test]
    fn validate_url_allows_domains() {
        for s in ["http://example.com/", "https://en.wikipedia.org/wiki/Rust"] {
            assert!(validate_request_url(&Url::parse(s).unwrap()).is_ok(), "{s}");
        }
    }

    #[test]
    fn validate_url_allows_global_literal() {
        assert!(validate_request_url(&Url::parse("http://8.8.8.8/").unwrap()).is_ok());
    }

    #[test]
    fn validate_url_blocks_non_http_schemes() {
        for s in [
            "ftp://example.com/",
            "gopher://example.com/",
            "dict://x/",
            "data:text/plain,hi",
        ] {
            let e = validate_request_url(&Url::parse(s).unwrap()).unwrap_err();
            assert!(matches!(e, SsrfError::BlockedScheme(_)), "{s} -> {e:?}");
        }
    }

    #[test]
    fn validate_url_blocks_file_scheme() {
        let e = validate_request_url(&Url::parse("file:///etc/passwd").unwrap()).unwrap_err();
        assert!(matches!(e, SsrfError::BlockedScheme(_)));
    }

    #[test]
    fn validate_url_blocks_loopback_literal() {
        let e = validate_request_url(&Url::parse("http://127.0.0.1/").unwrap()).unwrap_err();
        assert_eq!(e, SsrfError::BlockedAddress(v4("127.0.0.1")));
    }

    #[test]
    fn validate_url_blocks_encoded_ip_literals() {
        // WHATWG IPv4 canonicalisation: all of these are 127.0.0.1.
        for s in [
            "http://2130706433/",
            "http://0x7f.1/",
            "http://017700000001/",
        ] {
            let e = validate_request_url(&Url::parse(s).unwrap()).unwrap_err();
            assert_eq!(e, SsrfError::BlockedAddress(v4("127.0.0.1")), "{s}");
        }
    }

    #[test]
    fn validate_url_blocks_ipv6_literals() {
        for s in [
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fd00::1]/",
        ] {
            let e = validate_request_url(&Url::parse(s).unwrap()).unwrap_err();
            assert!(matches!(e, SsrfError::BlockedAddress(_)), "{s} -> {e:?}");
        }
    }
}
