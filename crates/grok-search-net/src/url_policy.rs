use std::net::{IpAddr, ToSocketAddrs};

use grok_search_types::{GrokSearchError, Result};
use url::{Host, Url};

pub fn validate_public_http_url(raw: &str) -> Result<Url> {
    let parsed = validate_http_url(raw)?;
    if !url_is_public(&parsed) {
        return Err(GrokSearchError::SecurityPolicy(
            "url must resolve to a public http or https address".to_string(),
        ));
    }
    Ok(parsed)
}

pub fn validate_http_url(raw: &str) -> Result<Url> {
    let parsed = Url::parse(raw).map_err(|_| {
        GrokSearchError::InvalidParams(
            "url must be an absolute http or https URL with a host".to_string(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(GrokSearchError::InvalidParams(
            "url must be an absolute http or https URL with a host".to_string(),
        ));
    }
    Ok(parsed)
}

pub fn url_is_private_or_local(url: &Url) -> bool {
    !url_is_public(url)
}

pub fn url_is_public(url: &Url) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") {
                return false;
            }
            resolve_host_ips(&domain)
                .map(|ips| !ips.is_empty() && ips.into_iter().all(ip_is_public))
                .unwrap_or(false)
        }
        Host::Ipv4(ip) => ip_is_public(IpAddr::V4(ip)),
        Host::Ipv6(ip) => ip_is_public(IpAddr::V6(ip)),
    }
}

fn resolve_host_ips(host: &str) -> Option<Vec<IpAddr>> {
    let addrs = (host, 0).to_socket_addrs().ok()?;
    Some(addrs.map(|addr| addr.ip()).collect())
}

fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 224)
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.segments()[0] & 0xfe00 == 0xfc00
                || ip.segments()[0] & 0xffc0 == 0xfe80)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_local_urls() {
        for raw in [
            "http://127.0.0.1/page",
            "http://localhost/page",
            "http://10.0.0.1/page",
            "http://172.16.0.1/page",
            "http://192.168.0.1/page",
            "http://169.254.1.1/page",
            "http://[::1]/page",
            "http://[fc00::1]/page",
        ] {
            assert!(validate_public_http_url(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn accepts_public_ip_urls() {
        assert!(validate_public_http_url("https://93.184.216.34/page").is_ok());
    }
}
