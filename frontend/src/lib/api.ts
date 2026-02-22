import type {
  CrawlResponse,
  CrawlProgress,
  CrawlListResponse,
  CrawlStats,
  GraphData,
} from "../types/api";

const BASE = "/api/v1";

async function fetchJSON<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `HTTP ${res.status}`);
  }
  return res.json();
}

export async function createCrawl(
  url: string,
  depth: number,
  targeted?: boolean
): Promise<CrawlResponse> {
  return fetchJSON(`${BASE}/crawls`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ url, depth, ...(targeted ? { targeted } : {}) }),
  });
}

export async function listCrawls(
  params: { status?: string; limit?: number; offset?: number } = {}
): Promise<CrawlListResponse> {
  const query = new URLSearchParams();
  if (params.status) query.set("status", params.status);
  if (params.limit) query.set("limit", String(params.limit));
  if (params.offset) query.set("offset", String(params.offset));
  const qs = query.toString();
  return fetchJSON(`${BASE}/crawls${qs ? `?${qs}` : ""}`);
}

export async function getCrawl(id: string): Promise<CrawlProgress> {
  return fetchJSON(`${BASE}/crawls/${id}`);
}

export async function deleteCrawl(
  id: string
): Promise<{ status: string; crawl_id: string }> {
  return fetchJSON(`${BASE}/crawls/${id}`, { method: "DELETE" });
}

export async function getCrawlGraph(id: string): Promise<GraphData> {
  return fetchJSON(`${BASE}/crawls/${id}/graph`);
}

export async function getCrawlStats(id: string): Promise<CrawlStats> {
  return fetchJSON(`${BASE}/crawls/${id}/stats`);
}
