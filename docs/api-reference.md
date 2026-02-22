# API Reference

Base URL: `/api/v1`

All endpoints return JSON. Error responses use the format `{"error": "message"}`.

## Crawls

### Create a Crawl

```
POST /api/v1/crawls
```

Start a new crawl from a given URL.

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `url` | string | Yes | The URL to crawl (must be http or https) |
| `depth` | integer | Yes | Maximum link depth to follow (1–5, where 1 = root only) |
| `targeted` | boolean | No | When `true`, only follow links within the same registered domain (eTLD+1) as the root URL. Defaults to `false`. |

**Example:**

```bash
curl -X POST http://localhost:8080/api/v1/crawls \
  -H 'Content-Type: application/json' \
  -d '{"url": "https://example.com", "depth": 2, "targeted": true}'
```

**Response:** `201 Created`

```json
{
  "crawl_id": "d262a3e7-19de-437f-b0a4-cf1d689b1caf",
  "status": "running"
}
```

**Error responses:**

| Status | Cause |
|--------|-------|
| `502 Bad Gateway` | Root URL DNS resolution failed or HTTP error from target |
| `504 Gateway Timeout` | Root URL request timed out |
| `404 Not Found` | Root URL returned HTTP 404 |
| `500 Internal Server Error` | Neo4j database error |

---

### List Crawls

```
GET /api/v1/crawls
```

List all crawls with optional filtering and pagination.

**Query parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `status` | string | — | Filter by status: `"running"`, `"completed"`, or `"cancelled"` |
| `limit` | integer | 20 | Max results per page (capped at 100) |
| `offset` | integer | 0 | Number of results to skip |

**Example:**

```bash
curl "http://localhost:8080/api/v1/crawls?status=running&limit=10"
```

**Response:** `200 OK`

```json
{
  "crawls": [
    {
      "crawl_id": "d262a3e7-19de-437f-b0a4-cf1d689b1caf",
      "root_url": "HTTPS://EXAMPLE.COM",
      "requested_depth": 2,
      "status": "completed",
      "total": 42,
      "completed": 40,
      "failed": 2,
      "cancelled": 0,
      "targeted": true
    }
  ],
  "total": 1,
  "offset": 0,
  "limit": 20
}
```

---

### Get Crawl Detail

```
GET /api/v1/crawls/:id
```

Get detailed progress for a specific crawl.

**Path parameters:**

| Parameter | Description |
|-----------|-------------|
| `id` | Crawl UUID |

**Example:**

```bash
curl http://localhost:8080/api/v1/crawls/d262a3e7-19de-437f-b0a4-cf1d689b1caf
```

**Response:** `200 OK`

```json
{
  "crawl_id": "d262a3e7-19de-437f-b0a4-cf1d689b1caf",
  "status": "running",
  "total": 786,
  "completed": 500,
  "pending": 200,
  "in_progress": 26,
  "failed": 60,
  "cancelled": 0,
  "root_url": "https://example.com",
  "requested_depth": 3,
  "targeted": false
}
```

**Status values:**
- `"running"` — at least one URL is PENDING or IN-PROGRESS
- `"completed"` — all URLs finished processing (mix of COMPLETED, FAILED, or CANCELLED)
- `"cancelled"` — all URLs finished and none completed successfully (all CANCELLED/FAILED)

**Error responses:**

| Status | Cause |
|--------|-------|
| `404 Not Found` | Crawl ID does not exist |
| `500 Internal Server Error` | Database error |

---

### Cancel a Crawl

```
DELETE /api/v1/crawls/:id
```

Cancel a running crawl. Sets all PENDING and IN-PROGRESS URLs to CANCELLED.

**Example:**

```bash
curl -X DELETE http://localhost:8080/api/v1/crawls/d262a3e7-19de-437f-b0a4-cf1d689b1caf
```

**Response:** `200 OK`

```json
{
  "status": "cancelled",
  "crawl_id": "d262a3e7-19de-437f-b0a4-cf1d689b1caf"
}
```

**Error responses:**

| Status | Cause |
|--------|-------|
| `404 Not Found` | Crawl ID does not exist |
| `500 Internal Server Error` | Database error |

---

### Get Crawl Graph

```
GET /api/v1/crawls/:id/graph
```

Get the full graph structure for visualization (nodes and edges).

**Example:**

```bash
curl http://localhost:8080/api/v1/crawls/d262a3e7-19de-437f-b0a4-cf1d689b1caf/graph
```

**Response:** `200 OK`

```json
{
  "nodes": [
    {
      "id": "HTTPS://EXAMPLE.COM",
      "label": "EXAMPLE.COM",
      "domain": "example.com",
      "depth": 0,
      "status": "root",
      "node_type": "ROOT"
    },
    {
      "id": "HTTPS://EXAMPLE.COM/ABOUT",
      "label": "EXAMPLE.COM/ABOUT",
      "domain": "example.com",
      "depth": 1,
      "status": "COMPLETED",
      "node_type": "URL"
    }
  ],
  "edges": [
    {
      "source": "HTTPS://EXAMPLE.COM",
      "target": "HTTPS://EXAMPLE.COM/ABOUT"
    }
  ]
}
```

**Error responses:**

| Status | Cause |
|--------|-------|
| `404 Not Found` | Crawl ID does not exist |
| `500 Internal Server Error` | Database error |

---

### Get Crawl Statistics

```
GET /api/v1/crawls/:id/stats
```

Get aggregate statistics for a crawl.

**Example:**

```bash
curl http://localhost:8080/api/v1/crawls/d262a3e7-19de-437f-b0a4-cf1d689b1caf/stats
```

**Response:** `200 OK`

```json
{
  "crawl_id": "d262a3e7-19de-437f-b0a4-cf1d689b1caf",
  "total_urls": 786,
  "unique_domains": 348,
  "max_depth_reached": 3,
  "status_counts": {
    "pending": 0,
    "in_progress": 0,
    "completed": 726,
    "failed": 60,
    "cancelled": 0
  }
}
```

**Error responses:**

| Status | Cause |
|--------|-------|
| `404 Not Found` | Crawl ID does not exist |
| `500 Internal Server Error` | Database error |

---

## WebSocket

### Crawl Progress Stream

```
GET /api/v1/crawls/:id/ws
```

WebSocket endpoint for real-time crawl progress. The server pushes updates every 2 seconds.

**Connection:**

```javascript
const ws = new WebSocket('ws://localhost:8080/api/v1/crawls/UUID/ws');

ws.onmessage = (event) => {
  const progress = JSON.parse(event.data);
  console.log(`${progress.completed}/${progress.total} completed`);
};
```

**Message format:** Same as [Get Crawl Detail](#get-crawl-detail) response.

**Connection lifecycle:**
1. Client connects via WebSocket upgrade
2. Server polls Neo4j every 2 seconds and sends progress JSON
3. When `status` becomes `"completed"`, the server sends the final message and closes
4. If the crawl is not found, sends `{"error": "Crawl not found"}` and closes
5. If the client disconnects, the server stops polling

---

## Health Endpoints

### Liveness Check

```
GET /livez
```

Basic liveness check. Always returns 200 if the server is running.

```bash
curl http://localhost:8080/livez
```

**Response:** `200 OK`

```json
{"status": "ok"}
```

### Readiness Check

```
GET /readyz
```

Checks that the manager can connect to Neo4j.

```bash
curl http://localhost:8080/readyz
```

**Response:** `200 OK`

```json
{"status": "ready"}
```

**Response:** `503 Service Unavailable`

```json
{"status": "not ready", "reason": "neo4j unavailable"}
```

---

## Error Response Format

All error responses use this format:

```json
{"error": "Human-readable error message"}
```

| Status Code | Meaning |
|-------------|---------|
| `400 Bad Request` | Invalid request body or parameters |
| `404 Not Found` | Crawl ID not found, or target URL returned 404 |
| `500 Internal Server Error` | Neo4j connection or query failure |
| `502 Bad Gateway` | Target URL unreachable, DNS failure, or HTTP error |
| `503 Service Unavailable` | Manager cannot connect to Neo4j (readiness check) |
| `504 Gateway Timeout` | Target URL request timed out |
