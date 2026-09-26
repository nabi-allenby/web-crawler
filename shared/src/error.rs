use thiserror::Error;

#[derive(Error, Debug)]
pub enum CrawlerError {
    #[error("Neo4j connection failed: {0}")]
    Neo4jConnection(#[from] neo4rs::Error),

    #[error("DNS resolution failed for {domain} after {attempts} attempts")]
    DnsResolution { domain: String, attempts: u32 },

    #[error("HTTP request failed for {url}: {source}")]
    HttpRequest {
        url: String,
        source: reqwest::Error,
    },

    #[error("HTTP {status} response from {url}")]
    HttpStatus {
        url: String,
        status: u16,
    },

    #[error("HTTP request timed out for {url}")]
    HttpTimeout { url: String },

    #[error("Failed to read response body from {url}: {source}")]
    HttpBodyRead {
        url: String,
        source: reqwest::Error,
    },

    #[error("Non-HTML response from {url}: {content_type}")]
    NotHtml { url: String, content_type: String },

    #[error("Neo4j query failed: {0}")]
    Neo4jQuery(String),
}
