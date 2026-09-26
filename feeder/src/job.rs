use std::collections::{HashMap, HashSet};

use neo4rs::{query, Graph};

use crate::config::Config;
use shared::crawler::{self, PageData};
use shared::dns;
use shared::error::CrawlerError;
use shared::url_normalize::{self, NormalizedUrl};

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
    /// Node budget for the crawl; 0 means unlimited (crawls created before the cap).
    pub max_pages: i64,
}

/// Represents a child node to be created in Neo4j.
struct ChildNode {
    name: String,
    host: String,
    ip: String,
    domain: String,
    http_type: String,
    requested_depth: i64,
    current_depth: i64,
    request_time: String,
    crawl_id: String,
    targeted: bool,
    target_domain: String,
    max_pages: i64,
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
                max_pages: node.get::<i64>("max_pages").unwrap_or(0),
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

/// Result of fetching a job's URL.
enum FetchOutcome {
    /// HTML page with links to follow.
    Page(PageData),
    /// Fetched fine but not HTML (PDF, image, ...). A leaf: no children, no retry.
    Leaf,
    /// Fetch failed; status has already been updated to PENDING or FAILED.
    Failed,
}

/// Attempts to fetch a URL's HTML content. Implements retry logic with proper error matching.
async fn validate_job(
    graph: &Graph,
    client: &reqwest::Client,
    config: &Config,
    job: &mut UrlJob,
) -> Result<FetchOutcome, anyhow::Error> {
    let full_url = format!("{}{}", job.http_type, job.name);

    match crawler::get_page_data(client, &full_url).await {
        Ok(page_data) => Ok(FetchOutcome::Page(page_data)),
        Err(CrawlerError::NotHtml { content_type, .. }) => {
            tracing::info!("Not HTML ({}), treating as leaf: {}", content_type, full_url);
            update_job_status(graph, job, "COMPLETED", job.attempts).await?;
            Ok(FetchOutcome::Leaf)
        }
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

            Ok(FetchOutcome::Failed)
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
///
/// Enforces the crawl's page budget: only as many children are inserted as fit
/// under `max_pages`. The count is read in the same transaction, but feeders run
/// in parallel, so the cap is approximate (overshoot bounded by
/// feeders × links-per-page); good enough to keep a crawl from running away.
async fn batch_create_children(
    graph: &Graph,
    parent: &UrlJob,
    children: &[ChildNode],
) -> Result<(), anyhow::Error> {
    let mut txn = graph.start_txn().await?;

    let children = if parent.max_pages > 0 {
        let mut count_result = txn
            .execute(
                query("MATCH (u:URL {crawl_id: $crawl_id}) RETURN count(u) AS c")
                    .param("crawl_id", parent.crawl_id.as_str()),
            )
            .await?;
        let existing: i64 = match count_result.next(txn.handle()).await? {
            Some(row) => row.get("c")?,
            None => 0,
        };
        let remaining = usize::try_from(parent.max_pages - existing).unwrap_or(0);
        if remaining < children.len() {
            tracing::warn!(
                "Crawl {} at page budget ({}/{}); dropping {} of {} links from {}",
                parent.crawl_id,
                existing,
                parent.max_pages,
                children.len() - remaining,
                children.len(),
                parent.name
            );
        }
        &children[..remaining.min(children.len())]
    } else {
        children
    };

    for child in children {
        txn.run(
            query(
                "MATCH (p:URL {name: $pname, http_type: $phttp, current_depth: $pdepth, crawl_id: $crawl_id}) \
                 MERGE (c:URL {name: $name, http_type: $http_type, crawl_id: $crawl_id}) \
                 ON CREATE SET c.host = $host, c.ip = $ip, c.domain = $domain, \
                     c.job_status = CASE WHEN $cur_depth = $req_depth THEN 'COMPLETED' ELSE 'PENDING' END, \
                     c.requested_depth = $req_depth, \
                     c.current_depth = $cur_depth, c.request_time = $req_time, \
                     c.targeted = $targeted, c.target_domain = $target_domain, \
                     c.max_pages = $max_pages \
                 MERGE (p)-[:Lead]->(c)",
            )
            .param("max_pages", child.max_pages)
            .param("pname", parent.name.as_str())
            .param("phttp", parent.http_type.as_str())
            .param("pdepth", parent.current_depth)
            .param("crawl_id", child.crawl_id.as_str())
            .param("name", child.name.as_str())
            .param("host", child.host.as_str())
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
        FetchOutcome::Page(pd) => pd,
        FetchOutcome::Leaf => return Ok(true),
        FetchOutcome::Failed => return Ok(false),
    };

    // Step 2: Extract URLs from HTML and normalize once. Keyed by the exact
    // `http_type + name` so two hrefs to the same page collapse before dedup.
    let full_url = format!("{}{}", job.http_type, job.name);
    let extracted_urls = crawler::extract_urls(&page_data.html, &full_url);
    let mut normalized_map: HashMap<String, NormalizedUrl> = HashMap::new();
    for url in &extracted_urls {
        let n = url_normalize::normalize_url(url);
        let key = format!("{}{}", n.http_type, n.name);
        normalized_map.entry(key).or_insert(n);
    }

    // Step 2b: Filter by target domain when targeted
    if job.targeted && !job.target_domain.is_empty() {
        normalized_map
            .retain(|_, n| url_normalize::is_same_registered_domain(&n.host, &job.target_domain));
    }

    // Step 3: Deduplicate against existing DB nodes (server-side)
    let candidate_keys: HashSet<String> = normalized_map.keys().cloned().collect();
    let new_urls = filter_new_urls(graph, &candidate_keys, &job.crawl_id).await?;

    if new_urls.is_empty() {
        tracing::warn!("No new URLs found in: {}", job.name);
        update_job_status(graph, job, "COMPLETED", job.attempts).await?;
        return Ok(true);
    }

    // Step 4: Resolve each distinct host once, then fan the result out to every
    // page on that host.
    let new_pages: Vec<&NormalizedUrl> = new_urls
        .iter()
        .filter_map(|key| normalized_map.get(key))
        .collect();

    let resolved = dns::resolve_hosts(
        resolver,
        new_pages.iter().map(|n| n.host.as_str()),
        config.max_dns_depth,
    )
    .await;

    let request_time = format!("{:?}", page_data.elapsed);
    let children: Vec<ChildNode> = new_pages
        .into_iter()
        .filter_map(|n| {
            let stats = resolved.get(&n.host)?;
            Some(ChildNode {
                name: n.name.clone(),
                host: n.host.clone(),
                ip: stats.ip.clone(),
                domain: stats.domain.clone(),
                http_type: n.http_type.clone(),
                requested_depth: job.requested_depth,
                current_depth: job.current_depth + 1,
                request_time: request_time.clone(),
                crawl_id: job.crawl_id.clone(),
                targeted: job.targeted,
                target_domain: job.target_domain.clone(),
                max_pages: job.max_pages,
            })
        })
        .collect();

    if children.is_empty() {
        update_job_status(graph, job, "FAILED", job.attempts).await?;
        return Ok(false);
    }

    // Step 6: Batch-create nodes and relationships
    batch_create_children(graph, job, &children).await?;

    // Step 7: Mark COMPLETED
    update_job_status(graph, job, "COMPLETED", job.attempts).await?;
    Ok(true)
}
