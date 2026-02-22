use neo4rs::{query, Graph};

use crate::models::crawl::{CrawlListItem, CrawlProgress, CrawlStats, StatusCounts};

pub struct CreateCrawlParams<'a> {
    pub crawl_id: &'a str,
    pub root_name: &'a str,
    pub root_ip: &'a str,
    pub root_domain: &'a str,
    pub http_type: &'a str,
    pub depth: i64,
    pub request_time: &'a str,
    pub children: &'a [(String, String, String, String)],
    pub targeted: bool,
    pub target_domain: &'a str,
}

/// Create ROOT node and child URL nodes in a single transaction with crawl_id.
pub async fn create_crawl_graph(
    graph: &Graph,
    params: &CreateCrawlParams<'_>,
) -> Result<(), neo4rs::Error> {
    let mut txn = graph.start_txn().await?;

    // Create ROOT node with crawl_id
    txn.run(
        query(
            "CREATE (:ROOT {name: $name, ip: $ip, domain: $domain, http_type: $http_type, \
             requested_depth: $req_depth, current_depth: 0, request_time: $req_time, \
             crawl_id: $crawl_id, created_at: datetime(), \
             targeted: $targeted, target_domain: $target_domain})",
        )
        .param("name", params.root_name)
        .param("ip", params.root_ip)
        .param("domain", params.root_domain)
        .param("http_type", params.http_type)
        .param("req_depth", params.depth)
        .param("req_time", params.request_time)
        .param("crawl_id", params.crawl_id)
        .param("targeted", params.targeted)
        .param("target_domain", params.target_domain),
    )
    .await?;

    // Create child URL nodes and Lead relationships with crawl_id
    for (child_name, child_ip, child_domain, child_http_type) in params.children {
        txn.run(
            query(
                "MATCH (root:ROOT {crawl_id: $crawl_id}) \
                 MERGE (c:URL {name: $name, http_type: $http_type, crawl_id: $crawl_id}) \
                 ON CREATE SET c.ip = $ip, c.domain = $domain, \
                     c.job_status = CASE WHEN 1 = $req_depth THEN 'COMPLETED' ELSE 'PENDING' END, \
                     c.requested_depth = $req_depth, \
                     c.current_depth = 1, c.request_time = $req_time, \
                     c.targeted = $targeted, c.target_domain = $target_domain \
                 MERGE (root)-[:Lead]->(c)",
            )
            .param("crawl_id", params.crawl_id)
            .param("req_depth", params.depth)
            .param("name", child_name.as_str())
            .param("ip", child_ip.as_str())
            .param("domain", child_domain.as_str())
            .param("http_type", child_http_type.as_str())
            .param("req_time", params.request_time)
            .param("targeted", params.targeted)
            .param("target_domain", params.target_domain),
        )
        .await?;
    }

    txn.commit().await?;
    Ok(())
}

/// Get crawl progress by crawl_id.
pub async fn get_crawl_progress(
    graph: &Graph,
    crawl_id: &str,
) -> Result<Option<CrawlProgress>, anyhow::Error> {
    let mut result = graph
        .execute(
            query(
                "OPTIONAL MATCH (r:ROOT {crawl_id: $crawl_id}) \
                 WITH r \
                 OPTIONAL MATCH (u:URL {crawl_id: $crawl_id}) \
                 WITH r, \
                   count(u) AS total, \
                   sum(CASE WHEN u.job_status = 'COMPLETED' THEN 1 ELSE 0 END) AS completed, \
                   sum(CASE WHEN u.job_status = 'PENDING' THEN 1 ELSE 0 END) AS pending, \
                   sum(CASE WHEN u.job_status = 'IN-PROGRESS' THEN 1 ELSE 0 END) AS in_progress, \
                   sum(CASE WHEN u.job_status = 'FAILED' THEN 1 ELSE 0 END) AS failed, \
                   sum(CASE WHEN u.job_status = 'CANCELLED' THEN 1 ELSE 0 END) AS cancelled \
                 RETURN r.name AS root_url, r.requested_depth AS depth, r.http_type AS http_type, \
                   r.targeted AS targeted, \
                   total, completed, pending, in_progress, failed, cancelled",
            )
            .param("crawl_id", crawl_id),
        )
        .await?;

    match result.next().await? {
        Some(row) => {
            let root_url: Option<String> = row.get("root_url").ok();
            match root_url {
                Some(url) => {
                    let http_type: String = row.get("http_type").unwrap_or_default();
                    let total: i64 = row.get("total")?;
                    let completed: i64 = row.get("completed")?;
                    let pending: i64 = row.get("pending")?;
                    let in_progress: i64 = row.get("in_progress")?;
                    let failed: i64 = row.get("failed")?;
                    let cancelled: i64 = row.get("cancelled")?;
                    let depth: i64 = row.get("depth")?;

                    let status = if pending == 0 && in_progress == 0 {
                        if cancelled > 0 && completed == 0 {
                            "cancelled".to_string()
                        } else {
                            "completed".to_string()
                        }
                    } else {
                        "running".to_string()
                    };

                    let targeted: bool = row.get::<bool>("targeted").unwrap_or(false);

                    Ok(Some(CrawlProgress {
                        crawl_id: crawl_id.to_string(),
                        status,
                        total,
                        completed,
                        pending,
                        in_progress,
                        failed,
                        cancelled,
                        root_url: format!("{}{}", http_type, url),
                        requested_depth: depth,
                        targeted,
                    }))
                }
                None => Ok(None),
            }
        }
        None => Ok(None),
    }
}

/// List crawls with optional status filter and pagination.
pub async fn list_crawls(
    graph: &Graph,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<CrawlListItem>, i64), anyhow::Error> {
    // Build query based on filter
    let cypher = if status_filter.is_some() {
        "MATCH (r:ROOT) \
         OPTIONAL MATCH (u:URL {crawl_id: r.crawl_id}) \
         WITH r, count(u) AS total, \
           sum(CASE WHEN u.job_status = 'COMPLETED' THEN 1 ELSE 0 END) AS completed, \
           sum(CASE WHEN u.job_status = 'PENDING' THEN 1 ELSE 0 END) AS pending, \
           sum(CASE WHEN u.job_status = 'IN-PROGRESS' THEN 1 ELSE 0 END) AS in_progress, \
           sum(CASE WHEN u.job_status = 'FAILED' THEN 1 ELSE 0 END) AS failed, \
           sum(CASE WHEN u.job_status = 'CANCELLED' THEN 1 ELSE 0 END) AS cancelled \
         WITH r, total, completed, failed, cancelled, \
           CASE WHEN pending = 0 AND in_progress = 0 THEN \
             CASE WHEN cancelled > 0 AND completed = 0 THEN 'cancelled' ELSE 'completed' END \
           ELSE 'running' END AS status \
         WHERE status = $status \
         WITH count(*) AS total_count, collect({r: r, total: total, completed: completed, failed: failed, cancelled: cancelled, status: status}) AS items \
         UNWIND items[$offset..($offset + $limit)] AS item \
         RETURN item.r.crawl_id AS crawl_id, item.r.name AS root_url, \
           item.r.http_type AS http_type, item.r.requested_depth AS depth, \
           item.r.targeted AS targeted, \
           item.total AS total, item.completed AS completed, item.failed AS failed, item.cancelled AS cancelled, item.status AS status, \
           total_count"
    } else {
        "MATCH (r:ROOT) \
         OPTIONAL MATCH (u:URL {crawl_id: r.crawl_id}) \
         WITH r, count(u) AS total, \
           sum(CASE WHEN u.job_status = 'COMPLETED' THEN 1 ELSE 0 END) AS completed, \
           sum(CASE WHEN u.job_status = 'PENDING' THEN 1 ELSE 0 END) AS pending, \
           sum(CASE WHEN u.job_status = 'IN-PROGRESS' THEN 1 ELSE 0 END) AS in_progress, \
           sum(CASE WHEN u.job_status = 'FAILED' THEN 1 ELSE 0 END) AS failed, \
           sum(CASE WHEN u.job_status = 'CANCELLED' THEN 1 ELSE 0 END) AS cancelled \
         WITH r, total, completed, failed, cancelled, \
           CASE WHEN pending = 0 AND in_progress = 0 THEN \
             CASE WHEN cancelled > 0 AND completed = 0 THEN 'cancelled' ELSE 'completed' END \
           ELSE 'running' END AS status \
         WITH count(*) AS total_count, collect({r: r, total: total, completed: completed, failed: failed, cancelled: cancelled, status: status}) AS items \
         UNWIND items[$offset..($offset + $limit)] AS item \
         RETURN item.r.crawl_id AS crawl_id, item.r.name AS root_url, \
           item.r.http_type AS http_type, item.r.requested_depth AS depth, \
           item.r.targeted AS targeted, \
           item.total AS total, item.completed AS completed, item.failed AS failed, item.cancelled AS cancelled, item.status AS status, \
           total_count"
    };

    let mut q = query(cypher)
        .param("limit", limit)
        .param("offset", offset);

    if let Some(status) = status_filter {
        q = q.param("status", status);
    }

    let mut result = graph.execute(q).await?;
    let mut crawls = Vec::new();
    let mut total_count: i64 = 0;

    while let Some(row) = result.next().await? {
        total_count = row.get("total_count").unwrap_or(0);
        let http_type: String = row.get("http_type").unwrap_or_default();
        let root_url: String = row.get("root_url").unwrap_or_default();

        crawls.push(CrawlListItem {
            crawl_id: row.get("crawl_id")?,
            root_url: format!("{}{}", http_type, root_url),
            requested_depth: row.get("depth")?,
            status: row.get("status")?,
            total: row.get("total")?,
            completed: row.get("completed")?,
            failed: row.get("failed")?,
            cancelled: row.get("cancelled")?,
            targeted: row.get::<bool>("targeted").unwrap_or(false),
        });
    }

    Ok((crawls, total_count))
}

/// Cancel a crawl by marking all its PENDING/IN-PROGRESS URLs as CANCELLED.
/// Uses a single atomic query to avoid race conditions with feeders.
pub async fn cancel_crawl(graph: &Graph, crawl_id: &str) -> Result<bool, anyhow::Error> {
    let mut result = graph
        .execute(
            query(
                "MATCH (r:ROOT {crawl_id: $crawl_id}) \
                 WITH r \
                 OPTIONAL MATCH (u:URL {crawl_id: $crawl_id}) \
                 WHERE u.job_status IN ['PENDING', 'IN-PROGRESS'] \
                 SET u.job_status = 'CANCELLED' \
                 RETURN r.crawl_id AS crawl_id",
            )
            .param("crawl_id", crawl_id),
        )
        .await?;

    Ok(result.next().await?.is_some())
}

/// Get aggregate statistics for a crawl.
pub async fn get_crawl_stats(
    graph: &Graph,
    crawl_id: &str,
) -> Result<Option<CrawlStats>, anyhow::Error> {
    let mut result = graph
        .execute(
            query(
                "OPTIONAL MATCH (r:ROOT {crawl_id: $crawl_id}) \
                 WITH r \
                 OPTIONAL MATCH (u:URL {crawl_id: $crawl_id}) \
                 WITH r, \
                   count(u) AS total_urls, \
                   count(DISTINCT u.domain) AS unique_domains, \
                   max(u.current_depth) AS max_depth, \
                   sum(CASE WHEN u.job_status = 'PENDING' THEN 1 ELSE 0 END) AS pending, \
                   sum(CASE WHEN u.job_status = 'IN-PROGRESS' THEN 1 ELSE 0 END) AS in_progress, \
                   sum(CASE WHEN u.job_status = 'COMPLETED' THEN 1 ELSE 0 END) AS completed, \
                   sum(CASE WHEN u.job_status = 'FAILED' THEN 1 ELSE 0 END) AS failed, \
                   sum(CASE WHEN u.job_status = 'CANCELLED' THEN 1 ELSE 0 END) AS cancelled \
                 RETURN r.crawl_id AS crawl_id, total_urls, unique_domains, max_depth, \
                   pending, in_progress, completed, failed, cancelled",
            )
            .param("crawl_id", crawl_id),
        )
        .await?;

    match result.next().await? {
        Some(row) => {
            let cid: Option<String> = row.get("crawl_id").ok();
            match cid {
                Some(_) => Ok(Some(CrawlStats {
                    crawl_id: crawl_id.to_string(),
                    total_urls: row.get("total_urls")?,
                    unique_domains: row.get("unique_domains")?,
                    max_depth_reached: row.get::<i64>("max_depth").unwrap_or(0),
                    status_counts: StatusCounts {
                        pending: row.get("pending")?,
                        in_progress: row.get("in_progress")?,
                        completed: row.get("completed")?,
                        failed: row.get("failed")?,
                        cancelled: row.get("cancelled")?,
                    },
                })),
                None => Ok(None),
            }
        }
        None => Ok(None),
    }
}
