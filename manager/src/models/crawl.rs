use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CrawlRequest {
    pub url: String,
    pub depth: i64,
    #[serde(default)]
    pub targeted: Option<bool>,
}

#[derive(Serialize)]
pub struct CrawlResponse {
    pub crawl_id: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct CrawlProgress {
    pub crawl_id: String,
    pub status: String,
    pub total: i64,
    pub completed: i64,
    pub pending: i64,
    pub in_progress: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub root_url: String,
    pub requested_depth: i64,
    pub targeted: bool,
}

#[derive(Serialize)]
pub struct CrawlListItem {
    pub crawl_id: String,
    pub root_url: String,
    pub requested_depth: i64,
    pub status: String,
    pub total: i64,
    pub completed: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub targeted: bool,
}

#[derive(Serialize)]
pub struct CrawlListResponse {
    pub crawls: Vec<CrawlListItem>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize)]
pub struct CrawlStats {
    pub crawl_id: String,
    pub total_urls: i64,
    pub unique_domains: i64,
    pub max_depth_reached: i64,
    pub status_counts: StatusCounts,
}

#[derive(Serialize)]
pub struct StatusCounts {
    pub pending: i64,
    pub in_progress: i64,
    pub completed: i64,
    pub failed: i64,
    pub cancelled: i64,
}
