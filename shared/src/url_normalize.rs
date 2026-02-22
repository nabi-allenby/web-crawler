use psl::Psl;

/// Normalizes a URL by uppercasing, removing protocol and www prefix.
///
/// Returns (normalized_name, protocol).
///
/// # Examples
/// - `"https://www.Google.com"` -> `("GOOGLE.COM", "HTTPS://")`
/// - `"http://example.org"` -> `("EXAMPLE.ORG", "HTTP://")`
pub fn normalize_url(url: &str) -> (String, String) {
    let upper = url.to_uppercase();
    let (stripped, proto) = if let Some(rest) = upper.strip_prefix("HTTPS://") {
        (rest, "HTTPS://")
    } else if let Some(rest) = upper.strip_prefix("HTTP://") {
        (rest, "HTTP://")
    } else {
        (upper.as_str(), "HTTP://")
    };
    let name = stripped
        .strip_prefix("WWW.")
        .unwrap_or(stripped)
        .to_string();
    (name, proto.to_string())
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
