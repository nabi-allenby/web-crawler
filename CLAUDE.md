# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

### Rust
```bash
cargo check --workspace          # Fast compilation check
cargo build --release             # Release build (LTO + stripped)
cargo test --workspace            # Run all tests
cargo test -p shared              # Test single crate
cargo clippy --workspace -- -D warnings  # Lint (CI-strict)
```

### Frontend (from `frontend/`)
```bash
npm install                       # Install deps
npm run dev                       # Dev server on :3000 (proxies /api to :8080)
npm run build                     # Type-check + production build
npm run lint                      # ESLint
npm run type-check                # TypeScript check only
```

### Docker (from repo root, use minikube docker-env for local k8s)
```bash
docker build -t ghcr.io/bluedotiya/web-crawler/manager:latest -f manager/Dockerfile .
docker build -t ghcr.io/bluedotiya/web-crawler/feeder:latest -f feeder/Dockerfile .
docker build -t ghcr.io/bluedotiya/web-crawler/frontend:latest -f frontend/Dockerfile .
```

## Architecture

Three services communicate through a shared Neo4j database (no direct inter-service HTTP):

- **manager** — Axum HTTP server (port 8080). REST API at `/api/v1/crawls/*` + WebSocket for live progress. Creates ROOT nodes and initial URL children when a crawl is submitted.
- **feeder** — Background workers (8 replicas). Poll Neo4j for PENDING URLs, fetch HTML, extract links, create child nodes. Atomic job claiming prevents worker conflicts.
- **frontend** — React SPA (Vite/TypeScript/Tailwind). Served by nginx in production, proxied via Vite in dev. Uses React Query for polling and WebSocket for real-time updates.
- **shared** — Rust library crate used by both manager and feeder. Contains: crawler (HTTP fetch + URL extraction), dns (resolution with iterative domain shortening), neo4j_client, url_normalize, schema (indexes/constraints), error types.

### Data Flow
1. User submits URL + depth (1-5) via frontend → POST `/api/v1/crawls`
2. Manager normalizes URL, resolves DNS, creates ROOT + child URL nodes in Neo4j
3. Feeder workers atomically claim PENDING URLs, fetch HTML, extract/deduplicate links, create children
4. Frontend polls progress via REST (5s) or WebSocket (2s), displays force-graph visualization

### Neo4j Data Model
- **ROOT** node (one per crawl, unique on `crawl_id`) — the seed URL
- **URL** nodes — discovered links with `job_status` (PENDING/IN-PROGRESS/COMPLETED/FAILED/CANCELLED)
- **Lead** edges — parent → child link relationships
- All nodes scoped by `crawl_id` for isolation between crawls

## Key Conventions

- **Conventional commits** required on PR titles: `feat:`, `fix:`, `chore:`, etc. (enforced by CI). Breaking changes use `!` suffix (e.g., `feat!:`). Drives automated semver + per-service tagging.
- **Pre-commit hooks**: `cargo check`, `cargo clippy -D warnings`, `cargo test`, frontend lint+typecheck. Install: `pip install pre-commit && pre-commit install`
- **Workspace dependency gotcha**: `default-features = false` in `[workspace.dependencies]` is ignored by Cargo. Each member crate must set it explicitly.
- **TLS in containers**: Use `rustls-tls-webpki-roots` (bundles CAs in binary). Avoid `native-tls` or `native-roots` in slim Docker images.
- **HTTP clients** in both feeder and manager must set `.user_agent(...)` to avoid 403 responses.
- **TypeScript**: Strict mode enabled, no unused locals/parameters. Path alias `@/` → `./src/`.
- **Docker images** must use full GHCR path (`ghcr.io/bluedotiya/web-crawler/{service}:tag`) to match k8s deployment specs.

## API Routes (manager)

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/api/v1/crawls` | Create new crawl |
| GET | `/api/v1/crawls` | List crawls (filter/pagination) |
| GET | `/api/v1/crawls/{id}` | Get crawl progress |
| DELETE | `/api/v1/crawls/{id}` | Cancel crawl |
| GET | `/api/v1/crawls/{id}/graph` | Graph data (nodes + edges) |
| GET | `/api/v1/crawls/{id}/stats` | Crawl statistics |
| GET | `/api/v1/crawls/{id}/ws` | WebSocket for live updates |
| GET | `/livez`, `/readyz` | Health probes |

## Project Layout

```
shared/src/          → lib.rs, crawler.rs, dns.rs, neo4j_client.rs, url_normalize.rs, schema.rs, error.rs
manager/src/         → main.rs, config.rs, routes/{crawl,status,graph,ws}.rs, services/{crawl,graph}_service.rs
feeder/src/          → main.rs, config.rs, job.rs
frontend/src/        → App.tsx, pages/{Dashboard,CrawlList,CrawlDetail,NewCrawl}.tsx, components/GraphView.tsx, lib/api.ts, hooks/useWebSocket.ts
web-crawler/         → Helm parent chart (neo4j, manager, feeder, frontend subcharts)
docs/                → architecture.md, api-reference.md, neo4j-graph-model.md, deployment.md, development.md
```
