use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use uuid::Uuid;

use crate::models::crawl::{CrawlRequest, CrawlResponse};
use crate::services::crawl_service;
use crate::state::AppState;
use shared::error::CrawlerError;
use shared::{crawler, dns, url_normalize};

/// Map CrawlerError to appropriate HTTP status code.
fn crawler_error_to_status(err: &CrawlerError) -> StatusCode {
    match err {
        CrawlerError::HttpTimeout { .. } => StatusCode::GATEWAY_TIMEOUT,
        CrawlerError::HttpStatus { status, .. } if *status == 404 => StatusCode::NOT_FOUND,
        CrawlerError::HttpStatus { .. }
        | CrawlerError::HttpRequest { .. }
        | CrawlerError::HttpBodyRead { .. } => StatusCode::BAD_GATEWAY,
        CrawlerError::DnsResolution { .. } => StatusCode::BAD_GATEWAY,
        CrawlerError::Neo4jConnection(_) | CrawlerError::Neo4jQuery(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

const MAX_CRAWL_DEPTH: i64 = 5;

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

    // 1. Normalize root URL
    let (root_name, http_type) = url_normalize::normalize_url(&req.url);
    let targeted = req.targeted.unwrap_or(false);

    // 1b. Compute target domain for targeted crawls
    let target_domain = if targeted {
        match url_normalize::registered_domain(&root_name) {
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
        root_name,
        req.depth
    );

    // 5. DNS resolve root URL
    let root_stats =
        match dns::get_network_stats(&state.resolver, &root_name, state.config.max_dns_depth).await
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

    // 6. Resolve DNS for each extracted URL in parallel
    let request_time = format!("{:?}", page_data.elapsed);

    // 6a. Normalize extracted URLs and filter by target domain if targeted
    let normalized_urls: Vec<(String, String)> = extracted_urls
        .iter()
        .map(|url| url_normalize::normalize_url(url))
        .filter(|(norm_name, _)| {
            !targeted || url_normalize::is_same_registered_domain(norm_name, &target_domain)
        })
        .collect();

    let dns_futures: Vec<_> = normalized_urls
        .iter()
        .map(|(norm_name, child_http_type)| {
            let norm_name = norm_name.clone();
            let child_http_type = child_http_type.clone();
            let resolver = &state.resolver;
            let max_depth = state.config.max_dns_depth;
            async move {
                match dns::get_network_stats(resolver, &norm_name, max_depth).await {
                    Ok(stats) => Some((norm_name, stats.ip, stats.domain, child_http_type)),
                    Err(_) => None,
                }
            }
        })
        .collect();

    let children: Vec<(String, String, String, String)> =
        futures::future::join_all(dns_futures)
            .await
            .into_iter()
            .flatten()
            .collect();

    // 7. Create ROOT + children in Neo4j with crawl_id
    let params = crawl_service::CreateCrawlParams {
        crawl_id: &crawl_id,
        root_name: &root_name,
        root_ip: &root_stats.ip,
        root_domain: &root_stats.domain,
        http_type: &http_type,
        depth: req.depth,
        request_time: &request_time,
        children: &children,
        targeted,
        target_domain: &target_domain,
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
