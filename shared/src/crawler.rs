use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use url::Url;

use crate::error::CrawlerError;

static ANCHOR_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").unwrap());

/// Upper bound on links taken from a single page. A page-level crawl of a large
/// site can expose thousands of anchors per page; past this point extra links
/// mostly add cost, not coverage.
pub const MAX_LINKS_PER_PAGE: usize = 500;

/// Path extensions that never yield an HTML page worth crawling. Checked before
/// the fetch so we don't spend a request (and a Neo4j node) on a binary.
const SKIPPED_EXTENSIONS: &[&str] = &[
    "pdf", "png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp", "zip", "gz", "tar", "tgz",
    "rar", "7z", "mp3", "mp4", "webm", "avi", "mov", "css", "js", "mjs", "woff", "woff2", "ttf",
    "eot", "xml", "rss", "atom", "json", "csv", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "exe",
    "dmg", "apk",
];

/// True when the URL's path ends in an extension from `SKIPPED_EXTENSIONS`.
fn has_skipped_extension(url: &Url) -> bool {
    let path = url.path();
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    match last_segment.rsplit_once('.') {
        Some((_, ext)) => SKIPPED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

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

    // Only HTML has links to follow. Reject before reading the body so a PDF or
    // image link found in an anchor costs a HEAD-sized request, not a download.
    // A missing header is allowed through: plenty of small servers omit it.
    if let Some(ct) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        let ct = ct.to_str().unwrap_or_default();
        let is_html = ct.starts_with("text/html") || ct.starts_with("application/xhtml");
        if !is_html {
            return Err(CrawlerError::NotHtml {
                url: url.to_string(),
                content_type: ct.to_string(),
            });
        }
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
/// Only returns http/https URLs whose path does not end in a known non-page
/// extension, and at most `MAX_LINKS_PER_PAGE` of them in document order.
pub fn extract_urls(html: &str, base_url: &str) -> Vec<String> {
    let base = match Url::parse(base_url) {
        Ok(u) => u,
        Err(_) => return Vec::new(),
    };

    let document = Html::parse_document(html);

    let urls: Vec<String> = document
        .select(&ANCHOR_SELECTOR)
        .filter_map(|el| el.value().attr("href"))
        .filter_map(|href| base.join(href).ok())
        .filter(|url| url.scheme() == "http" || url.scheme() == "https")
        .filter(|url| !has_skipped_extension(url))
        .map(|url| url.to_string())
        .take(MAX_LINKS_PER_PAGE + 1)
        .collect();

    if urls.len() > MAX_LINKS_PER_PAGE {
        tracing::warn!(
            "Page {} has more than {} links; truncating",
            base_url,
            MAX_LINKS_PER_PAGE
        );
    }
    urls.into_iter().take(MAX_LINKS_PER_PAGE).collect()
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
        let html = r#"<a href="//cdn.example.com/page">CDN</a>"#;
        let urls = extract_urls(html, "https://example.com/page");
        assert_eq!(urls, vec!["https://cdn.example.com/page"]);
    }

    #[test]
    fn test_extract_urls_skips_non_page_extensions() {
        let html = r#"
            <a href="/report.pdf">PDF</a>
            <a href="/logo.PNG">Image</a>
            <a href="/app.js">Script</a>
            <a href="/archive.tar.gz">Archive</a>
            <a href="/about">About</a>
            <a href="/docs/index.html">Docs</a>
            <a href="/v1.2/notes">Dotted dir</a>
        "#;
        let urls = extract_urls(html, "https://example.com/");
        assert_eq!(
            urls,
            vec![
                "https://example.com/about",
                "https://example.com/docs/index.html",
                "https://example.com/v1.2/notes",
            ]
        );
    }

    #[test]
    fn test_extract_urls_extension_check_ignores_query() {
        // The extension lives in the path, not the query string.
        let html = r#"<a href="/download?file=a.pdf">Q</a> <a href="/x.pdf?v=2">P</a>"#;
        let urls = extract_urls(html, "https://example.com/");
        assert_eq!(urls, vec!["https://example.com/download?file=a.pdf"]);
    }

    #[test]
    fn test_extract_urls_caps_at_max_links() {
        let html: String = (0..MAX_LINKS_PER_PAGE + 50)
            .map(|i| format!(r#"<a href="/p/{i}">{i}</a>"#))
            .collect();
        let urls = extract_urls(&html, "https://example.com/");
        assert_eq!(urls.len(), MAX_LINKS_PER_PAGE);
        assert_eq!(urls[0], "https://example.com/p/0");
        assert_eq!(urls[MAX_LINKS_PER_PAGE - 1], format!("https://example.com/p/{}", MAX_LINKS_PER_PAGE - 1));
    }
}
