mod config;
mod health;
mod job;

use std::time::Duration;

use hickory_resolver::TokioResolver;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

use config::Config;
use shared::{neo4j_client, schema};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("feeder=info")),
        )
        .init();

    // Load config
    let _ = dotenvy::dotenv();
    let config = Config::from_env();

    // Create shared resources
    let mut graph =
        neo4j_client::connect(&config.neo4j_uri, &config.neo4j_username, &config.neo4j_password)
            .await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.http_timeout_secs))
        .user_agent("Mozilla/5.0 (compatible; WebCrawler/1.0)")
        .build()?;

    let resolver = TokioResolver::builder_tokio()?.build();

    tracing::info!("Feeder started, connecting to {}", config.neo4j_uri);

    // Start health server in background
    health::spawn_health_server();

    // Ensure database schema (indexes)
    schema::ensure_schema(&graph).await?;

    // Graceful shutdown: watch channel signals the main loop to stop
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received shutdown signal, finishing current job...");
        let _ = shutdown_tx.send(true);
    });

    // Track the current in-progress job so we can reset it on shutdown
    let mut current_job: Option<job::UrlJob> = None;

    // Exponential backoff: starts at poll_min_ms, doubles on idle, resets on work found
    let mut backoff_ms = config.poll_min_ms;

    // Main loop
    loop {
        // Check for shutdown before starting a new job
        if *shutdown_rx.borrow() {
            tracing::info!("Shutting down gracefully");
            break;
        }

        // Health check loop with reconnection
        while !neo4j_client::health_check(&graph).await {
            if *shutdown_rx.borrow() {
                tracing::info!("Shutting down during reconnection");
                break;
            }
            if let Some(new_graph) = neo4j_client::restore_connection(
                &config.neo4j_uri,
                &config.neo4j_username,
                &config.neo4j_password,
            )
            .await
            {
                graph = new_graph;
            } else {
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(config.poll_max_ms);
            }
        }

        // Exit reconnection loop on shutdown
        if *shutdown_rx.borrow() {
            tracing::info!("Shutting down gracefully");
            break;
        }

        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;

        // Fetch a pending job (also reclaims one stale job if no pending work)
        let mut url_job = match job::fetch_job(&graph, config.stale_timeout_minutes).await {
            Ok(Some(j)) => {
                backoff_ms = config.poll_min_ms; // Reset on work found
                j
            }
            Ok(None) => {
                backoff_ms = (backoff_ms * 2).min(config.poll_max_ms);
                continue;
            }
            Err(e) => {
                tracing::error!("Failed to fetch job: {}", e);
                backoff_ms = (backoff_ms * 2).min(config.poll_max_ms);
                continue;
            }
        };

        // Track this job so shutdown can reset it.
        // We keep current_job set even after processing completes because
        // reset_to_pending is safe (only acts on IN-PROGRESS status).
        current_job = Some(job::UrlJob {
            name: url_job.name.clone(),
            http_type: url_job.http_type.clone(),
            requested_depth: url_job.requested_depth,
            current_depth: url_job.current_depth,
            attempts: url_job.attempts,
            crawl_id: url_job.crawl_id.clone(),
            targeted: url_job.targeted,
            target_domain: url_job.target_domain.clone(),
        });

        // Check for shutdown after claiming but before processing.
        // If we break here, the post-loop code resets the job to PENDING.
        if *shutdown_rx.borrow() {
            tracing::info!("Shutdown requested after claiming job {}", url_job.name);
            break;
        }

        // Process the job
        match job::feeding(&graph, &client, &resolver, &config, &mut url_job).await {
            Ok(true) => {
                tracing::info!("Feed Cycle Completed for: {}", url_job.name);
            }
            Ok(false) => {
                tracing::error!("Something went wrong during feeding :(");
            }
            Err(e) => {
                tracing::error!("Feed error: {}", e);
                job::mark_failed(&graph, &url_job).await;
            }
        }
    }

    // Reset any in-progress job before exiting
    if let Some(ref stale_job) = current_job {
        tracing::info!("Resetting in-progress job {} on shutdown", stale_job.name);
        job::reset_to_pending(&graph, stale_job).await;
    }

    Ok(())
}
