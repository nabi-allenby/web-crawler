use psl::Psl;
use url::Url;

/// Normalizes a URL to its host, uppercased, with the protocol returned separately.
///
/// The path, query and fragment are discarded: downstream consumers resolve the
/// result via DNS, so anything beyond the host makes the name unresolvable. An
/// explicit non-default port is preserved, since `registered_domain` strips it.
///
/// Returns (normalized_name, protocol).
///
/// # Examples
/// - `"https://www.Google.com"` -> `("GOOGLE.COM", "HTTPS://")`
/// - `"http://example.org"` -> `("EXAMPLE.ORG", "HTTP://")`
/// - `"https://en.wikipedia.org/"` -> `("EN.WIKIPEDIA.ORG", "HTTPS://")`
/// - `"https://example.com/a/b?q=1#f"` -> `("EXAMPLE.COM", "HTTPS://")`
pub fn normalize_url(url: &str) -> (String, String) {
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
            let proto = if u.scheme() == "https" {
                "HTTPS://"
            } else {
                "HTTP://"
            };

            // url lowercases hosts; Url::port() is None for the scheme default.
            let mut host = u.host_str().unwrap_or_default().to_uppercase();
            if let Some(port) = u.port() {
                host = format!("{host}:{port}");
            }

            let name = host.strip_prefix("WWW.").unwrap_or(&host).to_string();
            (name, proto.to_string())
        }
        // Not parseable as a URL at all (e.g. an empty or malformed href). Fall back
        // to prefix stripping so the caller still gets a best-effort name, and let
        // DNS resolution reject it downstream.
        None => {
            let upper = trimmed.to_uppercase();
            let (stripped, proto) = if let Some(rest) = upper.strip_prefix("HTTPS://") {
                (rest, "HTTPS://")
            } else if let Some(rest) = upper.strip_prefix("HTTP://") {
                (rest, "HTTP://")
            } else {
                (upper.as_str(), "HTTP://")
            };
            let host = stripped.split(['/', '?', '#']).next().unwrap_or(stripped);
            let name = host.strip_prefix("WWW.").unwrap_or(host).to_string();
            (name, proto.to_string())
        }
    }
}

/// Extracts the registered domain (eTLD+1) from a normalized name.
///
/// The input should be an uppercase normalized name (no protocol, no `www.`).
/// Ports are stripped before lookup. Returns uppercase eTLD+1.
///
/// # Examples
/// - `"EXAMPLE.COM"` -> `Some("EXAMPLE.COM")`
/// - `"BLOG.EXAMPLE.CO.UK"` -> `Some("EXAMPLE.CO.UK")`
/// - `"EXAMPLE.COM:8080"` -> `Some("EXAMPLE.COM")`
/// - `"COM"` (bare TLD) -> `None`
pub fn registered_domain(normalized_name: &str) -> Option<String> {
    // Strip port if present
    let host = normalized_name.split(':').next().unwrap_or(normalized_name);
    // psl requires lowercase input
    let lower = host.to_lowercase();
    let domain = psl::List.domain(lower.as_bytes())?;
    let domain_str = std::str::from_utf8(domain.as_bytes()).ok()?;
    Some(domain_str.to_uppercase())
}

/// Checks if a normalized name belongs to the same registered domain as the target.
///
/// Both inputs should be uppercase. The target should already be a registered domain
/// (output of `registered_domain()`).
pub fn is_same_registered_domain(normalized_name: &str, target_domain: &str) -> bool {
    match registered_domain(normalized_name) {
        Some(rd) => rd == target_domain,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_https_with_www() {
        let (name, proto) = normalize_url("https://www.Google.com");
        assert_eq!(name, "GOOGLE.COM");
        assert_eq!(proto, "HTTPS://");
    }

    #[test]
    fn test_normalize_http_no_www() {
        let (name, proto) = normalize_url("http://example.org");
        assert_eq!(name, "EXAMPLE.ORG");
        assert_eq!(proto, "HTTP://");
    }

    #[test]
    fn test_normalize_https_no_www() {
        let (name, proto) = normalize_url("https://google.com");
        assert_eq!(name, "GOOGLE.COM");
        assert_eq!(proto, "HTTPS://");
    }

    #[test]
    fn test_normalize_preserves_subdomains() {
        let (name, proto) = normalize_url("https://api.sub.example.com");
        assert_eq!(name, "API.SUB.EXAMPLE.COM");
        assert_eq!(proto, "HTTPS://");
    }

    #[test]
    fn test_normalize_http_with_www() {
        let (name, proto) = normalize_url("http://www.example.com");
        assert_eq!(name, "EXAMPLE.COM");
        assert_eq!(proto, "HTTP://");
    }

    #[test]
    fn test_normalize_preserves_www_in_subdomain() {
        let (name, proto) = normalize_url("https://subdomain.www.example.com");
        assert_eq!(name, "SUBDOMAIN.WWW.EXAMPLE.COM");
        assert_eq!(proto, "HTTPS://");
    }

    // Regression: paths were previously kept in the normalized name, so every
    // extracted link became an unresolvable "HOST/PATH" and was silently dropped
    // at the DNS step. A crawl of wikipedia.org found 383 links and created zero
    // child nodes because of this.
    #[test]
    fn test_normalize_strips_trailing_slash() {
        let (name, proto) = normalize_url("https://en.wikipedia.org/");
        assert_eq!(name, "EN.WIKIPEDIA.ORG");
        assert_eq!(proto, "HTTPS://");
    }

    #[test]
    fn test_normalize_strips_path() {
        let (name, proto) = normalize_url("https://simondev.io/courses");
        assert_eq!(name, "SIMONDEV.IO");
        assert_eq!(proto, "HTTPS://");
    }

    #[test]
    fn test_normalize_strips_query_and_fragment() {
        let (name, _) = normalize_url("https://example.com/a/b?q=1&r=2#frag");
        assert_eq!(name, "EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_protocol_relative() {
        // Wikipedia's homepage links look like href="//en.wikipedia.org/".
        let (name, proto) = normalize_url("//en.wikipedia.org/");
        assert_eq!(name, "EN.WIKIPEDIA.ORG");
        assert_eq!(proto, "HTTP://");
    }

    #[test]
    fn test_normalize_scheme_less_with_path() {
        let (name, proto) = normalize_url("example.com/some/page");
        assert_eq!(name, "EXAMPLE.COM");
        assert_eq!(proto, "HTTP://");
    }

    #[test]
    fn test_normalize_strips_www_with_path() {
        let (name, _) = normalize_url("https://www.example.com/path");
        assert_eq!(name, "EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_preserves_explicit_port() {
        let (name, _) = normalize_url("http://example.com:8080/path");
        assert_eq!(name, "EXAMPLE.COM:8080");
    }

    #[test]
    fn test_normalize_drops_default_port() {
        let (name, _) = normalize_url("https://example.com:443/path");
        assert_eq!(name, "EXAMPLE.COM");
    }

    #[test]
    fn test_normalize_uppercases_mixed_case_path_host() {
        let (name, _) = normalize_url("https://EN.Wikipedia.ORG/wiki/Rust");
        assert_eq!(name, "EN.WIKIPEDIA.ORG");
    }

    #[test]
    fn test_normalize_output_feeds_registered_domain() {
        // The normalized name must be usable by registered_domain(), which is what
        // targeted crawls filter on. "EN.WIKIPEDIA.ORG/" previously yielded None.
        let (name, _) = normalize_url("https://en.wikipedia.org/wiki/Rust");
        assert_eq!(registered_domain(&name), Some("WIKIPEDIA.ORG".to_string()));
        assert!(is_same_registered_domain(&name, "WIKIPEDIA.ORG"));
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
