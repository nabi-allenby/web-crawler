use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use uuid::Uuid;

use crate::models::crawl::{CrawlRequest, CrawlResponse};
use crate::services::crawl_service::{self, ChildUrl};
use crate::state::AppState;
use shared::error::CrawlerError;
use shared::url_normalize::NormalizedUrl;
use shared::{crawler, dns, url_normalize};

/// Map CrawlerError to appropriate HTTP status code.
fn crawler_error_to_status(err: &CrawlerError) -> StatusCode {
    match err {
        CrawlerError::HttpTimeout { .. } => StatusCode::GATEWAY_TIMEOUT,
        CrawlerError::HttpStatus { status, .. } if *status == 404 => StatusCode::NOT_FOUND,
        CrawlerError::HttpStatus { .. }
        | CrawlerError::HttpRequest { .. }
        | CrawlerError::HttpBodyRead { .. } => StatusCode::BAD_GATEWAY,
        // The root URL must be a page; there is nothing to crawl from a PDF.
        CrawlerError::NotHtml { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        CrawlerError::DnsResolution { .. } => StatusCode::BAD_GATEWAY,
        CrawlerError::Neo4jConnection(_) | CrawlerError::Neo4jQuery(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

const MAX_CRAWL_DEPTH: i64 = 5;
/// Page-level crawling can discover thousands of nodes per level, so every crawl
/// carries a node budget. These bound what a request may ask for.
const DEFAULT_MAX_PAGES: i64 = 1_000;
const MAX_MAX_PAGES: i64 = 10_000;

/// POST /api/v1/crawls — Submit a new crawl.
pub async fn create_crawl(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CrawlRequest>,
) -> impl IntoResponse {
    // 0. Validate depth
    if req.depth < 1 || req.depth > MAX_CRAWL_DEPTH {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("depth must be between 1 and {}", MAX_CRAWL_DEPTH)})),
        )
            .into_response();
    }

    // 0b. Validate page budget
    let max_pages = req.max_pages.unwrap_or(DEFAULT_MAX_PAGES);
    if !(1..=MAX_MAX_PAGES).contains(&max_pages) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("max_pages must be between 1 and {}", MAX_MAX_PAGES)})),
        )
            .into_response();
    }

    // 1. Normalize root URL
    let root = url_normalize::normalize_url(&req.url);
    let targeted = req.targeted.unwrap_or(false);

    // 1b. Compute target domain for targeted crawls
    let target_domain = if targeted {
        match url_normalize::registered_domain(&root.host) {
            Some(rd) => rd,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Cannot determine registered domain for targeted crawl (bare public suffix or invalid host)"})),
                )
                    .into_response();
            }
        }
    } else {
        String::new()
    };

    // 2. Fetch page HTML
    let page_data = match crawler::get_page_data(&state.client, &req.url).await {
        Ok(pd) => pd,
        Err(e) => {
            let status = crawler_error_to_status(&e);
            tracing::error!("Failed to fetch URL: {}", e);
            return (status, Json(json!({"error": e.to_string()}))).into_response();
        }
    };

    // 3. Extract URLs from HTML
    let extracted_urls = crawler::extract_urls(&page_data.html, &req.url);

    // 4. Generate unique crawl ID
    let crawl_id = Uuid::new_v4().to_string();

    tracing::info!(
        "Starting crawl {} for {} at depth {}",
        crawl_id,
        root.name,
        req.depth
    );

    // 5. DNS resolve root host
    let root_stats =
        match dns::get_network_stats(&state.resolver, &root.host, state.config.max_dns_depth).await
        {
            Ok(stats) => stats,
            Err(e) => {
                tracing::error!("Root DNS failed: {}", e);
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };

    let request_time = format!("{:?}", page_data.elapsed);

    // 6a. Normalize extracted URLs, collapse duplicates, filter by target domain
    let mut candidate_pages: HashMap<String, NormalizedUrl> = HashMap::new();
    for url in &extracted_urls {
        let n = url_normalize::normalize_url(url);
        if targeted && !url_normalize::is_same_registered_domain(&n.host, &target_domain) {
            continue;
        }
        candidate_pages
            .entry(format!("{}{}", n.http_type, n.name))
            .or_insert(n);
    }

    // 6b. Resolve each distinct host once and fan out to its pages
    let resolved = dns::resolve_hosts(
        &state.resolver,
        candidate_pages.values().map(|n| n.host.as_str()),
        state.config.max_dns_depth,
    )
    .await;

    let children: Vec<ChildUrl> = candidate_pages
        .values()
        .filter_map(|n| {
            let stats = resolved.get(&n.host)?;
            Some(ChildUrl {
                name: n.name.clone(),
                host: n.host.clone(),
                ip: stats.ip.clone(),
                domain: stats.domain.clone(),
                http_type: n.http_type.clone(),
            })
        })
        .collect();

    // DNS failures above collapse to None and vanish. Report them: a crawl that
    // creates no children is indistinguishable in the graph from one that simply
    // had nothing to crawl, so the logs are the only place the cause is visible.
    let candidates = candidate_pages.len();
    let dropped = candidates - children.len();
    if dropped > 0 {
        tracing::warn!(
            "Crawl {}: dropped {}/{} candidate links (DNS resolution failed)",
            crawl_id,
            dropped,
            candidates
        );
    }
    if children.is_empty() {
        tracing::warn!(
            "Crawl {}: no child URLs created from {} extracted links ({} passed the domain filter); crawl will report as failed",
            crawl_id,
            extracted_urls.len(),
            candidates
        );
    }

    // 7. Create ROOT + children in Neo4j with crawl_id
    let params = crawl_service::CreateCrawlParams {
        crawl_id: &crawl_id,
        root_name: &root.name,
        root_host: &root.host,
        root_ip: &root_stats.ip,
        root_domain: &root_stats.domain,
        http_type: &root.http_type,
        depth: req.depth,
        request_time: &request_time,
        children: &children,
        targeted,
        target_domain: &target_domain,
        max_pages,
    };
    if let Err(e) = crawl_service::create_crawl_graph(&state.graph, &params).await
    {
        tracing::error!("Failed to create graph: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Database error"})),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(json!(CrawlResponse {
            crawl_id,
            status: "running".to_string(),
        })),
    )
        .into_response()
}

/// DELETE /api/v1/crawls/:id — Cancel a running crawl.
pub async fn delete_crawl(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(crawl_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match crawl_service::cancel_crawl(&state.graph, &crawl_id).await {
        Ok(true) => (StatusCode::OK, Json(json!({"status": "cancelled", "crawl_id": crawl_id}))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "Crawl not found"}))),
        Err(e) => {
            tracing::error!("Failed to cancel crawl: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Database error"})),
            )
        }
    }
}
