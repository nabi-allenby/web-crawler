use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use url::Url;

use crate::error::CrawlerError;

static ANCHOR_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").unwrap());

pub struct PageData {
    pub html: String,
    pub elapsed: Duration,
}

/// Fetches a URL and returns its HTML content and elapsed time.
/// Returns typed errors for timeout, HTTP status, request failure, and body read failure.
pub async fn get_page_data(client: &Client, url: &str) -> Result<PageData, CrawlerError> {
    let start = Instant::now();

    let response = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            CrawlerError::HttpTimeout {
                url: url.to_string(),
            }
        } else {
            CrawlerError::HttpRequest {
                url: url.to_string(),
                source: e,
            }
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(CrawlerError::HttpStatus {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }

    let html = response.text().await.map_err(|e| CrawlerError::HttpBodyRead {
        url: url.to_string(),
        source: e,
    })?;

    Ok(PageData {
        html,
        elapsed: start.elapsed(),
    })
}

/// Extracts URLs from `<a href="...">` tags in HTML content.
/// Resolves relative URLs against the given base URL.
/// Only returns URLs with http or https schemes.
pub fn extract_urls(html: &str, base_url: &str) -> Vec<String> {
    let base = match Url::parse(base_url) {
        Ok(u) => u,
        Err(_) => return Vec::new(),
    };

    let document = Html::parse_document(html);

    document
        .select(&ANCHOR_SELECTOR)
        .filter_map(|el| el.value().attr("href"))
        .filter_map(|href| base.join(href).ok())
        .filter(|url| url.scheme() == "http" || url.scheme() == "https")
        .map(|url| url.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://example.com/page";

    #[test]
    fn test_extract_urls_basic() {
        let html = r#"<a href="https://google.com">link</a> <a href="http://example.org">other</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(
            urls,
            vec!["https://google.com/", "http://example.org/"]
        );
    }

    #[test]
    fn test_extract_urls_preserves_paths() {
        let html = r#"<a href="https://example.com/path/to/page">link</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(urls, vec!["https://example.com/path/to/page"]);
    }

    #[test]
    fn test_extract_urls_empty() {
        assert!(extract_urls("no urls here", BASE).is_empty());
    }

    #[test]
    fn test_extract_urls_no_anchor_tags() {
        let html = "<p>https://example.com</p>";
        assert!(extract_urls(html, BASE).is_empty());
    }

    #[test]
    fn test_extract_urls_multiple() {
        let html = r#"<a href="https://a.com">A</a> <a href="https://b.com">B</a> <a href="http://c.org">C</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(
            urls,
            vec!["https://a.com/", "https://b.com/", "http://c.org/"]
        );
    }

    #[test]
    fn test_extract_urls_with_hyphens_and_dots() {
        let html = r#"<a href="https://my-site.co.uk">1</a> <a href="http://sub.example-domain.com">2</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(
            urls,
            vec!["https://my-site.co.uk/", "http://sub.example-domain.com/"]
        );
    }

    #[test]
    fn test_extract_urls_with_ports() {
        let html = r#"<a href="https://example.com:8080/path">1</a> <a href="http://localhost:3000">2</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(
            urls,
            vec!["https://example.com:8080/path", "http://localhost:3000/"]
        );
    }

    #[test]
    fn test_extract_urls_relative_path() {
        let html = r#"<a href="/about">About</a>"#;
        let urls = extract_urls(html, "https://example.com/index.html");
        assert_eq!(urls, vec!["https://example.com/about"]);
    }

    #[test]
    fn test_extract_urls_relative_sibling() {
        let html = r#"<a href="contact.html">Contact</a>"#;
        let urls = extract_urls(html, "https://example.com/pages/index.html");
        assert_eq!(urls, vec!["https://example.com/pages/contact.html"]);
    }

    #[test]
    fn test_extract_urls_with_query_and_fragment() {
        let html = r#"<a href="https://example.com/search?q=rust#results">Search</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(urls, vec!["https://example.com/search?q=rust#results"]);
    }

    #[test]
    fn test_extract_urls_skips_non_http() {
        let html = r#"<a href="mailto:user@example.com">Email</a> <a href="ftp://files.example.com">FTP</a> <a href="https://example.com">Web</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(urls, vec!["https://example.com/"]);
    }

    #[test]
    fn test_extract_urls_skips_javascript() {
        let html = r#"<a href="javascript:void(0)">Click</a> <a href="https://real.com">Real</a>"#;
        let urls = extract_urls(html, BASE);
        assert_eq!(urls, vec!["https://real.com/"]);
    }

    #[test]
    fn test_extract_urls_invalid_base() {
        let html = r#"<a href="/about">About</a>"#;
        assert!(extract_urls(html, "not-a-url").is_empty());
    }

    #[test]
    fn test_extract_urls_protocol_relative() {
        let html = r#"<a href="//cdn.example.com/lib.js">CDN</a>"#;
        let urls = extract_urls(html, "https://example.com/page");
        assert_eq!(urls, vec!["https://cdn.example.com/lib.js"]);
    }
}
