use psl::Psl;
use url::{form_urlencoded, Url};

/// A URL split into the two things the crawler needs from it: a stable page
/// identity (`name`) and a DNS-resolvable host (`host`).
///
/// The two used to be the same string, which forced a choice between "nodes are
/// hosts" (so DNS works) and "nodes are pages" (so the graph is useful). Keeping
/// them separate lets a node be a page while DNS, `registered_domain` and the
/// targeted-crawl filter keep operating on the host alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedUrl {
    /// Page identity used as the Neo4j MERGE key and graph node id, without
    /// protocol: `"example.com/docs/Intro?page=2"`. The host part is lowercase
    /// and the path keeps its case, so `http_type + name` is a fetchable URL.
    pub name: String,
    /// Uppercased host with `www.` stripped and any non-default port kept:
    /// `"EXAMPLE.COM"` or `"EXAMPLE.COM:8080"`. Feed this to DNS and
    /// `registered_domain`, never `name`.
    pub host: String,
    /// `"HTTPS://"` or `"HTTP://"`.
    pub http_type: String,
}

/// Query parameters that carry no page identity (ad/campaign tracking). Dropped
/// so `?utm_source=x` and the bare URL collapse to one node.
const TRACKING_PARAMS: &[&str] = &["fbclid", "gclid", "msclkid", "yclid", "_ga", "mc_cid", "mc_eid"];

fn is_tracking_param(key: &str) -> bool {
    key.starts_with("utm_") || TRACKING_PARAMS.contains(&key)
}

/// Normalizes a URL into a page identity plus its host and protocol.
///
/// - Host: lowercased in `name`, uppercased in `host`; `www.` stripped; explicit
///   non-default port preserved.
/// - Path: case preserved, trailing slashes removed, so `/docs` and `/docs/` are
///   one page and `/About` and `/about` are two.
/// - Query: tracking parameters dropped, remaining pairs sorted, so parameter
///   order does not create duplicate nodes.
/// - Fragment: dropped; it never reaches the server.
///
/// # Examples
/// - `"https://www.Google.com"` -> name `"google.com"`, host `"GOOGLE.COM"`, `"HTTPS://"`
/// - `"https://en.wikipedia.org/wiki/Rust"` -> name `"en.wikipedia.org/wiki/Rust"`, host `"EN.WIKIPEDIA.ORG"`
/// - `"https://example.com/a/?b=2&a=1#f"` -> name `"example.com/a?a=1&b=2"`
pub fn normalize_url(url: &str) -> NormalizedUrl {
    let trimmed = url.trim();

    // Scheme-relative ("//host/path") and scheme-less ("host/path") inputs are not
    // absolute URLs, so retry with a default scheme before giving up.
    let parsed = Url::parse(trimmed)
        .ok()
        .filter(|u| u.host_str().is_some())
        .or_else(|| {
            let rest = trimmed.strip_prefix("//").unwrap_or(trimmed);
            Url::parse(&format!("http://{rest}"))
                .ok()
                .filter(|u| u.host_str().is_some())
        });

    match parsed {
        Some(u) => {
            let http_type = if u.scheme() == "https" {
                "HTTPS://"
            } else {
                "HTTP://"
            };

            // url lowercases hosts; Url::port() is None for the scheme default.
            let mut host = u.host_str().unwrap_or_default().to_string();
            host = host.strip_prefix("www.").unwrap_or(&host).to_string();
            if let Some(port) = u.port() {
                host = format!("{host}:{port}");
            }

            let path = u.path().trim_end_matches('/');

            let mut params: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| !is_tracking_param(k))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            params.sort();
            let query = if params.is_empty() {
                String::new()
            } else {
                let encoded = form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&params)
                    .finish();
                format!("?{encoded}")
            };

            NormalizedUrl {
                name: format!("{host}{path}{query}"),
                host: host.to_uppercase(),
                http_type: http_type.to_string(),
            }
        }
        // Not parseable as a URL at all (e.g. an empty or malformed href). Fall back
        // to prefix stripping so the caller still gets a best-effort name, and let
        // DNS resolution reject it downstream.
        None => {
            let lower = trimmed.to_lowercase();
            let (stripped, http_type) = if let Some(rest) = lower.strip_prefix("https://") {
                (rest, "HTTPS://")
            } else if let Some(rest) = lower.strip_prefix("http://") {
                (rest, "HTTP://")
            } else {
                (lower.as_str(), "HTTP://")
            };
            let host = stripped.split(['/', '?', '#']).next().unwrap_or(stripped);
            let host = host.strip_prefix("www.").unwrap_or(host);
            NormalizedUrl {
                name: host.to_string(),
                host: host.to_uppercase(),
                http_type: http_type.to_string(),
            }
        }
    }
}

/// Extracts the registered domain (eTLD+1) from a normalized host.
///
/// The input should be an uppercase host (no protocol, no `www.`), i.e.
/// `NormalizedUrl::host`. Ports are stripped before lookup. Returns uppercase eTLD+1.
///
/// # Examples
/// - `"EXAMPLE.COM"` -> `Some("EXAMPLE.COM")`
/// - `"BLOG.EXAMPLE.CO.UK"` -> `Some("EXAMPLE.CO.UK")`
/// - `"EXAMPLE.COM:8080"` -> `Some("EXAMPLE.COM")`
/// - `"COM"` (bare TLD) -> `None`
pub fn registered_domain(host: &str) -> Option<String> {
    // Strip port if present
    let host = host.split(':').next().unwrap_or(host);
    // psl requires lowercase input
    let lower = host.to_lowercase();
    let domain = psl::List.domain(lower.as_bytes())?;
    let domain_str = std::str::from_utf8(domain.as_bytes()).ok()?;
    Some(domain_str.to_uppercase())
}

/// Checks if a normalized host belongs to the same registered domain as the target.
///
/// Both inputs should be uppercase. The target should already be a registered domain
/// (output of `registered_domain()`).
pub fn is_same_registered_domain(host: &str, target_domain: &str) -> bool {
    match registered_domain(host) {
        Some(rd) => rd == target_domain,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(url: &str) -> NormalizedUrl {
        normalize_url(url)
    }

    // --- host handling ---

    #[test]
    fn test_normalize_https_with_www() {
        let n = norm("https://www.Google.com");
        assert_eq!(n.name, "google.com");
        assert_eq!(n.host, "GOOGLE.COM");
        assert_eq!(n.http_type, "HTTPS://");
    }

    #[test]
    fn test_normalize_http_no_www() {
        let n = norm("http://example.org");
        assert_eq!(n.name, "example.org");
        assert_eq!(n.host, "EXAMPLE.ORG");
        assert_eq!(n.http_type, "HTTP://");
    }

    #[test]
    fn test_normalize_preserves_subdomains() {
        let n = norm("https://api.sub.example.com");
        assert_eq!(n.name, "api.sub.example.com");
        assert_eq!(n.host, "API.SUB.EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_preserves_www_in_subdomain() {
        let n = norm("https://subdomain.www.example.com");
        assert_eq!(n.host, "SUBDOMAIN.WWW.EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_host_is_case_insensitive() {
        assert_eq!(norm("https://EN.Wikipedia.ORG").name, "en.wikipedia.org");
        assert_eq!(norm("https://EN.Wikipedia.ORG").host, "EN.WIKIPEDIA.ORG");
    }

    #[test]
    fn test_normalize_preserves_explicit_port() {
        let n = norm("http://example.com:8080/path");
        assert_eq!(n.name, "example.com:8080/path");
        assert_eq!(n.host, "EXAMPLE.COM:8080");
    }

    #[test]
    fn test_normalize_drops_default_port() {
        let n = norm("https://example.com:443/path");
        assert_eq!(n.name, "example.com/path");
        assert_eq!(n.host, "EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_protocol_relative() {
        // Wikipedia's homepage links look like href="//en.wikipedia.org/".
        let n = norm("//en.wikipedia.org/");
        assert_eq!(n.name, "en.wikipedia.org");
        assert_eq!(n.host, "EN.WIKIPEDIA.ORG");
        assert_eq!(n.http_type, "HTTP://");
    }

    #[test]
    fn test_normalize_scheme_less_with_path() {
        let n = norm("example.com/some/page");
        assert_eq!(n.name, "example.com/some/page");
        assert_eq!(n.http_type, "HTTP://");
    }

    // --- path handling: nodes are pages ---

    #[test]
    fn test_normalize_keeps_path() {
        let n = norm("https://simondev.io/courses");
        assert_eq!(n.name, "simondev.io/courses");
        assert_eq!(n.host, "SIMONDEV.IO");
    }

    #[test]
    fn test_normalize_path_case_preserved() {
        // Paths are case-sensitive on most servers; uppercasing them would both
        // merge distinct pages and produce URLs that 404 when refetched.
        assert_eq!(norm("https://en.wikipedia.org/wiki/Rust").name, "en.wikipedia.org/wiki/Rust");
        assert_ne!(norm("https://x.com/About").name, norm("https://x.com/about").name);
    }

    #[test]
    fn test_normalize_strips_trailing_slash() {
        assert_eq!(norm("https://en.wikipedia.org/").name, "en.wikipedia.org");
        assert_eq!(norm("https://example.com/docs/").name, "example.com/docs");
        assert_eq!(norm("https://example.com/docs/").name, norm("https://example.com/docs").name);
    }

    #[test]
    fn test_normalize_strips_www_with_path() {
        assert_eq!(norm("https://www.example.com/path").name, "example.com/path");
    }

    // --- query and fragment ---

    #[test]
    fn test_normalize_drops_fragment() {
        assert_eq!(norm("https://example.com/a#section").name, "example.com/a");
        assert_eq!(norm("https://example.com/a?q=1#frag").name, "example.com/a?q=1");
    }

    #[test]
    fn test_normalize_keeps_query() {
        assert_eq!(norm("https://example.com/search?q=rust").name, "example.com/search?q=rust");
    }

    #[test]
    fn test_normalize_sorts_query_params() {
        let a = norm("https://example.com/a?b=2&a=1");
        let b = norm("https://example.com/a?a=1&b=2");
        assert_eq!(a.name, "example.com/a?a=1&b=2");
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn test_normalize_drops_tracking_params() {
        let n = norm("https://example.com/p?utm_source=x&utm_medium=y&id=7&fbclid=abc");
        assert_eq!(n.name, "example.com/p?id=7");
        // A URL with only tracking params collapses to the bare page.
        assert_eq!(norm("https://example.com/p?utm_source=x").name, "example.com/p");
    }

    #[test]
    fn test_normalize_query_value_case_preserved() {
        assert_eq!(norm("https://example.com/s?q=Rust").name, "example.com/s?q=Rust");
    }

    // --- round trip: name must be refetchable ---

    #[test]
    fn test_normalize_round_trips_to_fetchable_url() {
        for input in [
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "http://example.com:8080/Path/To/Page?b=2&a=1#x",
            "https://www.example.co.uk/",
        ] {
            let n = norm(input);
            let rebuilt = format!("{}{}", n.http_type, n.name);
            let parsed = Url::parse(&rebuilt).unwrap_or_else(|_| panic!("{rebuilt} not a URL"));
            assert_eq!(parsed.host_str().unwrap().to_uppercase(), n.host.split(':').next().unwrap());
        }
    }

    #[test]
    fn test_normalize_is_idempotent() {
        let first = norm("https://www.Example.com/Docs/?b=2&a=1&utm_source=x#top");
        let again = norm(&format!("{}{}", first.http_type, first.name));
        assert_eq!(first, again);
    }

    // --- host feeds the domain helpers ---

    #[test]
    fn test_normalize_host_feeds_registered_domain() {
        let n = norm("https://en.wikipedia.org/wiki/Rust");
        assert_eq!(registered_domain(&n.host), Some("WIKIPEDIA.ORG".to_string()));
        assert!(is_same_registered_domain(&n.host, "WIKIPEDIA.ORG"));
    }

    #[test]
    fn test_registered_domain_simple() {
        assert_eq!(registered_domain("EXAMPLE.COM"), Some("EXAMPLE.COM".to_string()));
    }

    #[test]
    fn test_registered_domain_subdomain() {
        assert_eq!(registered_domain("BLOG.EXAMPLE.COM"), Some("EXAMPLE.COM".to_string()));
    }

    #[test]
    fn test_registered_domain_deep_subdomain() {
        assert_eq!(registered_domain("A.B.C.EXAMPLE.COM"), Some("EXAMPLE.COM".to_string()));
    }

    #[test]
    fn test_registered_domain_co_uk() {
        assert_eq!(registered_domain("BLOG.EXAMPLE.CO.UK"), Some("EXAMPLE.CO.UK".to_string()));
    }

    #[test]
    fn test_registered_domain_with_port() {
        assert_eq!(registered_domain("EXAMPLE.COM:8080"), Some("EXAMPLE.COM".to_string()));
    }

    #[test]
    fn test_registered_domain_bare_tld() {
        assert_eq!(registered_domain("COM"), None);
    }

    #[test]
    fn test_registered_domain_bare_public_suffix() {
        assert_eq!(registered_domain("GITHUB.IO"), None);
    }

    #[test]
    fn test_registered_domain_localhost() {
        assert_eq!(registered_domain("LOCALHOST"), None);
    }

    #[test]
    fn test_is_same_registered_domain_match() {
        assert!(is_same_registered_domain("BLOG.EXAMPLE.COM", "EXAMPLE.COM"));
    }

    #[test]
    fn test_is_same_registered_domain_exact() {
        assert!(is_same_registered_domain("EXAMPLE.COM", "EXAMPLE.COM"));
    }

    #[test]
    fn test_is_same_registered_domain_no_match() {
        assert!(!is_same_registered_domain("GOOGLE.COM", "EXAMPLE.COM"));
    }

    #[test]
    fn test_is_same_registered_domain_with_port() {
        assert!(is_same_registered_domain("API.EXAMPLE.COM:3000", "EXAMPLE.COM"));
    }

    #[test]
    fn test_is_same_registered_domain_co_uk() {
        assert!(is_same_registered_domain("SHOP.EXAMPLE.CO.UK", "EXAMPLE.CO.UK"));
    }
}
