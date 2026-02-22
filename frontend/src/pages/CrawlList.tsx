import { useQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import { listCrawls } from "../lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "../components/ui/card";
import { Button } from "../components/ui/button";
import { StatusBadge } from "../components/StatusBadge";
import { ProgressBar } from "../components/ProgressBar";

const PAGE_SIZE = 20;

export default function CrawlList() {
  const [searchParams, setSearchParams] = useSearchParams();
  const statusFilter = searchParams.get("status") || "";
  const page = Number(searchParams.get("page") || "1");
  const offset = (page - 1) * PAGE_SIZE;

  const { data, isLoading } = useQuery({
    queryKey: ["crawls", statusFilter, page],
    queryFn: () =>
      listCrawls({
        status: statusFilter || undefined,
        limit: PAGE_SIZE,
        offset,
      }),
    refetchInterval: 5000,
  });

  const setPage = (p: number) => {
    const params = new URLSearchParams(searchParams);
    params.set("page", String(p));
    setSearchParams(params);
  };

  const handleFilter = (status: string) => {
    const params = new URLSearchParams();
    if (status) params.set("status", status);
    params.set("page", "1");
    setSearchParams(params);
  };

  const totalPages = data ? Math.ceil(data.total / PAGE_SIZE) : 0;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-gray-900">Crawls</h1>
          <p className="text-gray-500 mt-1">
            {data?.total ?? 0} total crawls
          </p>
        </div>
        <Link to="/new">
          <Button>New Crawl</Button>
        </Link>
      </div>

      {/* Filters */}
      <div className="flex gap-2">
        {[
          { value: "", label: "All" },
          { value: "running", label: "Running" },
          { value: "completed", label: "Completed" },
        ].map(({ value, label }) => (
          <Button
            key={value}
            variant={statusFilter === value ? "default" : "outline"}
            size="sm"
            onClick={() => handleFilter(value)}
          >
            {label}
          </Button>
        ))}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Crawl History</CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <p className="text-gray-500 text-center py-8">Loading...</p>
          ) : data?.crawls.length === 0 ? (
            <p className="text-gray-500 text-center py-8">No crawls found</p>
          ) : (
            <div className="space-y-3">
              {data?.crawls.map((crawl) => (
                <Link
                  key={crawl.crawl_id}
                  to={`/crawls/${crawl.crawl_id}`}
                  className="block p-4 rounded-lg border border-gray-100 hover:border-gray-300 transition-colors"
                >
                  <div className="flex items-center justify-between mb-2">
                    <div className="flex items-center gap-3 min-w-0">
                      <span className="font-medium text-gray-900 truncate">
                        {crawl.root_url.toLowerCase()}
                      </span>
                      <span className="text-gray-400 text-sm shrink-0">
                        depth {crawl.requested_depth}
                      </span>
                      {crawl.targeted && (
                        <span className="inline-flex items-center rounded bg-purple-100 px-1.5 py-0.5 text-xs font-medium text-purple-700 shrink-0">
                          Targeted
                        </span>
                      )}
                    </div>
                    <StatusBadge status={crawl.status} />
                  </div>
                  <ProgressBar
                    completed={crawl.completed}
                    total={crawl.total}
                    failed={crawl.failed}
                  />
                </Link>
              ))}
            </div>
          )}

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-center gap-2 mt-6">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage(page - 1)}
              >
                Previous
              </Button>
              <span className="text-sm text-gray-500">
                Page {page} of {totalPages}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => setPage(page + 1)}
              >
                Next
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
