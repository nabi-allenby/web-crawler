export interface CrawlResponse {
  crawl_id: string;
  status: string;
}

export interface CrawlProgress {
  crawl_id: string;
  status: string;
  total: number;
  completed: number;
  pending: number;
  in_progress: number;
  failed: number;
  root_url: string;
  requested_depth: number;
  targeted: boolean;
}

export interface CrawlListItem {
  crawl_id: string;
  root_url: string;
  requested_depth: number;
  status: string;
  total: number;
  completed: number;
  failed: number;
  targeted: boolean;
}

export interface CrawlListResponse {
  crawls: CrawlListItem[];
  total: number;
  offset: number;
  limit: number;
}

export interface CrawlStats {
  crawl_id: string;
  total_urls: number;
  unique_domains: number;
  max_depth_reached: number;
  status_counts: {
    pending: number;
    in_progress: number;
    completed: number;
    failed: number;
  };
}

export interface GraphNode {
  id: string;
  label: string;
  domain: string;
  depth: number;
  status: string;
  node_type: string;
}

export interface GraphEdge {
  source: string;
  target: string;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}
