use std::collections::{HashMap, HashSet};

use neo4rs::{query, Graph};

use crate::config::Config;
use shared::crawler::{self, PageData};
use shared::dns;
use shared::error::CrawlerError;
use shared::url_normalize;

/// Represents a URL job fetched from Neo4j.
pub struct UrlJob {
    pub name: String,
    pub http_type: String,
    pub requested_depth: i64,
    pub current_depth: i64,
    pub attempts: Option<i64>,
    pub crawl_id: String,
    pub targeted: bool,
    pub target_domain: String,
}

/// Represents a child node to be created in Neo4j.
struct ChildNode {
    name: String,
    ip: String,
    domain: String,
    http_type: String,
    requested_depth: i64,
    current_depth: i64,
    request_time: String,
    crawl_id: String,
    targeted: bool,
    target_domain: String,
}

/// Atomically fetches and claims a single URL job from Neo4j.
/// Prioritises PENDING jobs, then falls back to stale IN-PROGRESS jobs
/// (stuck longer than `stale_timeout` minutes) so each feeder reclaims
/// exactly one job at a time instead of bulk-resetting all stale work.
pub async fn fetch_job(graph: &Graph, stale_timeout: i64) -> Result<Option<UrlJob>, anyhow::Error> {
    let mut result = graph
        .execute(
            query(
                "MATCH (n:URL) \
                 WHERE n.current_depth <> n.requested_depth \
                   AND ( \
                     n.job_status = 'PENDING' \
                     OR (n.job_status = 'IN-PROGRESS' \
                         AND n.claimed_at IS NOT NULL \
                         AND datetime() > n.claimed_at + duration({minutes: $timeout})) \
                   ) \
                 WITH n LIMIT 1 \
                 SET n.job_status = 'IN-PROGRESS', n.claimed_at = datetime() \
                 RETURN n",
            )
            .param("timeout", stale_timeout),
        )
        .await?;

    match result.next().await? {
        Some(row) => {
            let node: neo4rs::Node = row.get("n")?;
            Ok(Some(UrlJob {
                name: node.get("name")?,
                http_type: node.get("http_type")?,
                requested_depth: node.get("requested_depth")?,
                current_depth: node.get("current_depth")?,
                attempts: node.get::<i64>("attempts").ok(),
                crawl_id: node.get("crawl_id").unwrap_or_default(),
                targeted: node.get::<bool>("targeted").unwrap_or(false),
                target_domain: node.get::<String>("target_domain").unwrap_or_default(),
            }))
        }
        None => Ok(None),
    }
}


/// Updates a job's status and attempts counter in Neo4j.
async fn update_job_status(
    graph: &Graph,
    job: &UrlJob,
    status: &str,
    attempts: Option<i64>,
) -> Result<(), anyhow::Error> {
    let q = query(
        "MATCH (n:URL {name: $name, http_type: $http_type, current_depth: $current_depth, crawl_id: $crawl_id}) \
         WHERE n.job_status <> 'CANCELLED' \
         SET n.job_status = $status, n.attempts = $attempts",
    )
    .param("name", job.name.as_str())
    .param("http_type", job.http_type.as_str())
    .param("current_depth", job.current_depth)
    .param("crawl_id", job.crawl_id.as_str())
    .param("status", status)
    .param("attempts", attempts.unwrap_or(0));

    graph.run(q).await?;
    Ok(())
}

/// Attempts to fetch a URL's HTML content. Implements retry logic with proper error matching.
async fn validate_job(
    graph: &Graph,
    client: &reqwest::Client,
    config: &Config,
    job: &mut UrlJob,
) -> Result<Option<PageData>, anyhow::Error> {
    let full_url = format!("{}{}", job.http_type, job.name);

    match crawler::get_page_data(client, &full_url).await {
        Ok(page_data) => Ok(Some(page_data)),
        Err(e) => {
            let attempts = job.attempts.unwrap_or(0) + 1;
            job.attempts = Some(attempts);

            tracing::warn!("Request failed: {} -- Attempts: {} -- Error: {}", full_url, attempts, e);

            // 4xx errors are permanent — fail immediately without retry
            let is_permanent = matches!(e, CrawlerError::HttpStatus { status, .. } if (400..500).contains(&status));

            if is_permanent || attempts >= config.max_attempts {
                if !is_permanent {
                    tracing::error!(
                        "Failure limit reached! Giving up on {} after {} attempts.",
                        full_url,
                        attempts
                    );
                }
                update_job_status(graph, job, "FAILED", Some(attempts)).await?;
            } else {
                // Reset to PENDING so other feeders can retry
                update_job_status(graph, job, "PENDING", Some(attempts)).await?;
            }

            Ok(None)
        }
    }
}

/// Filters a list of candidate URLs against the database, returning only those
/// that don't already exist within this crawl. Scoped by crawl_id so
/// independent crawls don't interfere with each other.
async fn filter_new_urls(
    graph: &Graph,
    candidates: &HashSet<String>,
    crawl_id: &str,
) -> Result<HashSet<String>, anyhow::Error> {
    let candidate_list: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let mut result = graph
        .execute(
            query(
                "UNWIND $urls AS url \
                 OPTIONAL MATCH (n:URL {crawl_id: $crawl_id}) \
                 WHERE (n.http_type + n.name) = url \
                 WITH url, n \
                 WHERE n IS NULL \
                 RETURN url",
            )
            .param("urls", candidate_list)
            .param("crawl_id", crawl_id),
        )
        .await?;

    let mut new_urls = HashSet::new();
    while let Some(row) = result.next().await? {
        let url: String = row.get("url")?;
        new_urls.insert(url);
    }
    Ok(new_urls)
}

/// Creates child URL nodes and Lead relationships in a single transaction.
/// Uses MERGE to prevent duplicates when concurrent jobs discover the same URLs.
async fn batch_create_children(
    graph: &Graph,
    parent: &UrlJob,
    children: &[ChildNode],
) -> Result<(), anyhow::Error> {
    let mut txn = graph.start_txn().await?;

    for child in children {
        txn.run(
            query(
                "MATCH (p:URL {name: $pname, http_type: $phttp, current_depth: $pdepth, crawl_id: $crawl_id}) \
                 MERGE (c:URL {name: $name, http_type: $http_type, crawl_id: $crawl_id}) \
                 ON CREATE SET c.ip = $ip, c.domain = $domain, \
                     c.job_status = CASE WHEN $cur_depth = $req_depth THEN 'COMPLETED' ELSE 'PENDING' END, \
                     c.requested_depth = $req_depth, \
                     c.current_depth = $cur_depth, c.request_time = $req_time, \
                     c.targeted = $targeted, c.target_domain = $target_domain \
                 MERGE (p)-[:Lead]->(c)",
            )
            .param("pname", parent.name.as_str())
            .param("phttp", parent.http_type.as_str())
            .param("pdepth", parent.current_depth)
            .param("crawl_id", child.crawl_id.as_str())
            .param("name", child.name.as_str())
            .param("ip", child.ip.as_str())
            .param("domain", child.domain.as_str())
            .param("http_type", child.http_type.as_str())
            .param("req_depth", child.requested_depth)
            .param("cur_depth", child.current_depth)
            .param("req_time", child.request_time.as_str())
            .param("targeted", child.targeted)
            .param("target_domain", child.target_domain.as_str()),
        )
        .await?;
    }

    txn.commit().await?;
    Ok(())
}

/// Checks if this job's crawl has been cancelled.
async fn is_cancelled(graph: &Graph, job: &UrlJob) -> Result<bool, anyhow::Error> {
    let mut result = graph
        .execute(
            query(
                "MATCH (n:URL {name: $name, http_type: $http_type, crawl_id: $crawl_id}) \
                 RETURN n.job_status AS status",
            )
            .param("name", job.name.as_str())
            .param("http_type", job.http_type.as_str())
            .param("crawl_id", job.crawl_id.as_str()),
        )
        .await?;

    match result.next().await? {
        Some(row) => {
            let status: String = row.get("status")?;
            Ok(status == "CANCELLED")
        }
        None => Ok(false),
    }
}

/// Best-effort attempt to mark a job as FAILED in Neo4j.
/// Used when feeding() returns an unrecoverable error so the job
/// doesn't stay stuck in IN-PROGRESS forever.
pub async fn mark_failed(graph: &Graph, job: &UrlJob) {
    if let Err(e) = update_job_status(graph, job, "FAILED", job.attempts).await {
        tracing::error!("Failed to mark job {} as FAILED: {}", job.name, e);
    }
}

/// Best-effort reset of a job back to PENDING on graceful shutdown.
/// Allows another feeder to pick it up immediately instead of waiting
/// for the stale reclaimer.
pub async fn reset_to_pending(graph: &Graph, job: &UrlJob) {
    let result = graph
        .run(
            query(
                "MATCH (n:URL {name: $name, http_type: $http_type, crawl_id: $crawl_id}) \
                 WHERE n.job_status = 'IN-PROGRESS' \
                 SET n.job_status = 'PENDING', n.claimed_at = NULL",
            )
            .param("name", job.name.as_str())
            .param("http_type", job.http_type.as_str())
            .param("crawl_id", job.crawl_id.as_str()),
        )
        .await;

    if let Err(e) = result {
        tracing::error!("Failed to reset job {} to PENDING: {}", job.name, e);
    }
}

/// Main processing pipeline for a single job.
///
/// Orchestrates: validate -> IN-PROGRESS -> extract -> dedup -> DNS -> create -> COMPLETED
pub async fn feeding(
    graph: &Graph,
    client: &reqwest::Client,
    resolver: &hickory_resolver::TokioResolver,
    config: &Config,
    job: &mut UrlJob,
) -> Result<bool, anyhow::Error> {
    // Check for cancellation before starting work
    if is_cancelled(graph, job).await? {
        tracing::info!("Job {} cancelled, skipping", job.name);
        return Ok(false);
    }

    // Step 1: Validate (fetch HTML) — job is already IN-PROGRESS from fetch_job()
    let page_data = match validate_job(graph, client, config, job).await? {
        Some(pd) => pd,
        None => return Ok(false),
    };

    // Step 2: Extract URLs from HTML and normalize once
    let full_url = format!("{}{}", job.http_type, job.name);
    let extracted_urls = crawler::extract_urls(&page_data.html, &full_url);
    let mut normalized_map: HashMap<String, (String, String)> = HashMap::new();
    for url in &extracted_urls {
        let (norm_name, http_type) = url_normalize::normalize_url(url);
        let upper_key = format!("{}{}", http_type, norm_name).to_uppercase();
        normalized_map.entry(upper_key).or_insert((norm_name, http_type));
    }

    // Step 2b: Filter by target domain when targeted
    if job.targeted && !job.target_domain.is_empty() {
        normalized_map.retain(|_, (norm_name, _)| {
            url_normalize::is_same_registered_domain(norm_name, &job.target_domain)
        });
    }

    // Step 3: Deduplicate against existing DB nodes (server-side)
    let upper_urls: HashSet<String> = normalized_map.keys().cloned().collect();
    let new_urls = filter_new_urls(graph, &upper_urls, &job.crawl_id).await?;

    if new_urls.is_empty() {
        tracing::warn!("No new URLs found in: {}", job.name);
        update_job_status(graph, job, "COMPLETED", job.attempts).await?;
        return Ok(true);
    }

    // Step 4: DNS resolve in parallel, build child list
    let normalized: HashSet<(String, String)> = new_urls
        .iter()
        .filter_map(|key| normalized_map.get(key).cloned())
        .collect();

    let request_time = format!("{:?}", page_data.elapsed);
    let requested_depth = job.requested_depth;
    let current_depth = job.current_depth;
    let crawl_id = job.crawl_id.clone();

    let targeted = job.targeted;
    let target_domain = job.target_domain.clone();

    let dns_futures: Vec<_> = normalized
        .iter()
        .map(|(name, http_type)| {
            let name = name.clone();
            let http_type = http_type.clone();
            let req_time = request_time.clone();
            let cid = crawl_id.clone();
            let td = target_domain.clone();
            async move {
                match dns::get_network_stats(resolver, &name, config.max_dns_depth).await {
                    Ok(stats) => Some(ChildNode {
                        name,
                        ip: stats.ip,
                        domain: stats.domain,
                        http_type,
                        requested_depth,
                        current_depth: current_depth + 1,
                        request_time: req_time,
                        crawl_id: cid,
                        targeted,
                        target_domain: td,
                    }),
                    Err(e) => {
                        tracing::error!("URL: {} -- FAILED: {}", name, e);
                        None
                    }
                }
            }
        })
        .collect();

    let children: Vec<ChildNode> = futures::future::join_all(dns_futures)
        .await
        .into_iter()
        .flatten()
        .collect();

    if children.is_empty() {
        update_job_status(graph, job, "FAILED", job.attempts).await?;
        return Ok(false);
    }

    // Step 6: Re-check cancellation before creating children.
    // This closes the race window where a cancel fires after the initial
    // is_cancelled check but before we persist new PENDING children.
    if is_cancelled(graph, job).await? {
        tracing::info!("Job {} cancelled during processing, skipping child creation", job.name);
        return Ok(false);
    }

    // Step 7: Batch-create nodes and relationships
    batch_create_children(graph, job, &children).await?;

    // Step 8: Mark COMPLETED
    update_job_status(graph, job, "COMPLETED", job.attempts).await?;
    Ok(true)
}
