import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as Dialog from "@radix-ui/react-dialog";
import { getCrawl, getCrawlStats, getCrawlGraph, deleteCrawl } from "../lib/api";
import { useWebSocket } from "../hooks/useWebSocket";
import { Card, CardContent, CardHeader, CardTitle } from "../components/ui/card";
import { Button } from "../components/ui/button";
import { StatusBadge } from "../components/StatusBadge";
import { ProgressBar } from "../components/ProgressBar";
import { GraphView } from "../components/GraphView";
import { StatsView } from "../components/StatsView";

type Tab = "progress" | "graph" | "stats";

export default function CrawlDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<Tab>("progress");
  const [cancelling, setCancelling] = useState(false);
  const [showCancelDialog, setShowCancelDialog] = useState(false);

  const { data: crawl, isLoading } = useQuery({
    queryKey: ["crawl", id],
    queryFn: () => getCrawl(id!),
    enabled: !!id,
    refetchInterval: 5000,
  });

  const { data: stats } = useQuery({
    queryKey: ["crawl-stats", id],
    queryFn: () => getCrawlStats(id!),
    enabled: !!id && activeTab === "stats",
    refetchInterval: 10000,
  });

  const { data: graphData } = useQuery({
    queryKey: ["crawl-graph", id],
    queryFn: () => getCrawlGraph(id!),
    enabled: !!id && activeTab === "graph",
    refetchInterval: crawl?.status === "running" ? 15000 : false,
  });

  // WebSocket for live updates
  const { progress: wsProgress, connected } = useWebSocket(
    crawl?.status === "running" ? id : undefined
  );

  const liveProgress = wsProgress || crawl;

  const handleCancel = async () => {
    if (!id) return;
    setCancelling(true);
    setShowCancelDialog(false);
    try {
      await deleteCrawl(id);
      queryClient.invalidateQueries({ queryKey: ["crawl", id] });
    } catch {
      // ignore
    } finally {
      setCancelling(false);
    }
  };

  if (isLoading) {
    return <p className="text-gray-500 text-center py-16">Loading...</p>;
  }

  if (!crawl) {
    return (
      <div className="text-center py-16">
        <p className="text-gray-500 mb-4">Crawl not found</p>
        <Button variant="outline" onClick={() => navigate("/crawls")}>
          Back to Crawls
        </Button>
      </div>
    );
  }

  const tabs: { key: Tab; label: string }[] = [
    { key: "progress", label: "Progress" },
    { key: "graph", label: "Graph" },
    { key: "stats", label: "Statistics" },
  ];

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-3 mb-1">
            <h1 className="text-2xl font-bold text-gray-900">Crawl Detail</h1>
            <StatusBadge status={liveProgress?.status ?? crawl.status} />
            {connected && (
              <span className="inline-flex items-center gap-1 text-xs text-green-600">
                <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse" />
                Live
              </span>
            )}
          </div>
          <p className="text-gray-500 truncate max-w-2xl">
            {crawl.root_url.toLowerCase()}
          </p>
          <p className="text-gray-400 text-sm mt-1">
            Depth: {crawl.requested_depth}
            {crawl.targeted && (
              <span className="ml-2 inline-flex items-center rounded bg-purple-100 px-1.5 py-0.5 text-xs font-medium text-purple-700">
                Targeted
              </span>
            )}
            {" "}| ID: {id}
          </p>
        </div>
        <div className="flex gap-2">
          {crawl.status === "running" && (
            <Dialog.Root open={showCancelDialog} onOpenChange={setShowCancelDialog}>
              <Dialog.Trigger asChild>
                <Button variant="destructive" disabled={cancelling}>
                  {cancelling ? "Cancelling..." : "Cancel"}
                </Button>
              </Dialog.Trigger>
              <Dialog.Portal>
                <Dialog.Overlay className="fixed inset-0 bg-black/50" />
                <Dialog.Content className="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-white rounded-lg p-6 w-full max-w-sm shadow-lg space-y-4">
                  <Dialog.Title className="text-lg font-semibold text-gray-900">
                    Cancel this crawl?
                  </Dialog.Title>
                  <Dialog.Description className="text-sm text-gray-500">
                    This will stop the crawl and mark it as cancelled. This action
                    cannot be undone.
                  </Dialog.Description>
                  <div className="flex justify-end gap-3">
                    <Dialog.Close asChild>
                      <Button variant="outline">Keep Running</Button>
                    </Dialog.Close>
                    <Button variant="destructive" onClick={handleCancel}>
                      Cancel Crawl
                    </Button>
                  </div>
                </Dialog.Content>
              </Dialog.Portal>
            </Dialog.Root>
          )}
          <Button variant="outline" onClick={() => navigate("/crawls")}>
            Back
          </Button>
        </div>
      </div>

      {/* Progress summary */}
      {liveProgress && (
        <Card>
          <CardContent className="pt-6">
            <ProgressBar
              completed={liveProgress.completed}
              total={liveProgress.total}
              failed={liveProgress.failed}
            />
            <div className="grid grid-cols-5 gap-4 mt-4">
              <div className="text-center">
                <p className="text-2xl font-bold">{liveProgress.total}</p>
                <p className="text-xs text-gray-500">Total</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold text-green-600">
                  {liveProgress.completed}
                </p>
                <p className="text-xs text-gray-500">Completed</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold text-blue-600">
                  {liveProgress.in_progress}
                </p>
                <p className="text-xs text-gray-500">In Progress</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold text-yellow-600">
                  {liveProgress.pending}
                </p>
                <p className="text-xs text-gray-500">Pending</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold text-red-600">
                  {liveProgress.failed}
                </p>
                <p className="text-xs text-gray-500">Failed</p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Tabs */}
      <div className="border-b border-gray-200">
        <div className="flex space-x-8">
          {tabs.map((tab) => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={`py-3 border-b-2 text-sm font-medium transition-colors ${
                activeTab === tab.key
                  ? "border-blue-600 text-blue-600"
                  : "border-transparent text-gray-500 hover:text-gray-700"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content */}
      {activeTab === "progress" && liveProgress && (
        <Card>
          <CardHeader>
            <CardTitle>Crawl Progress</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-6">
              <div>
                <h4 className="text-sm font-medium text-gray-500 mb-2">
                  Details
                </h4>
                <dl className="space-y-2">
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Root URL</dt>
                    <dd className="font-medium truncate max-w-xs">
                      {crawl.root_url.toLowerCase()}
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Requested Depth</dt>
                    <dd className="font-medium">{crawl.requested_depth}</dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Scope</dt>
                    <dd className="font-medium">
                      {crawl.targeted ? "Targeted" : "Unrestricted"}
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Status</dt>
                    <dd>
                      <StatusBadge status={liveProgress.status} />
                    </dd>
                  </div>
                </dl>
              </div>
              <div>
                <h4 className="text-sm font-medium text-gray-500 mb-2">
                  Job Counts
                </h4>
                <dl className="space-y-2">
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Completed</dt>
                    <dd className="font-medium text-green-600">
                      {liveProgress.completed}
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">In Progress</dt>
                    <dd className="font-medium text-blue-600">
                      {liveProgress.in_progress}
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Pending</dt>
                    <dd className="font-medium text-yellow-600">
                      {liveProgress.pending}
                    </dd>
                  </div>
                  <div className="flex justify-between">
                    <dt className="text-gray-600">Failed</dt>
                    <dd className="font-medium text-red-600">
                      {liveProgress.failed}
                    </dd>
                  </div>
                </dl>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {activeTab === "graph" && (
        <Card>
          <CardHeader>
            <CardTitle>
              Graph Visualization
              {graphData && (
                <span className="text-sm font-normal text-gray-500 ml-2">
                  {graphData.nodes.length + 1} nodes, {graphData.edges.length} edges
                </span>
              )}
            </CardTitle>
          </CardHeader>
          <CardContent>
            {graphData ? (
              <GraphView data={graphData} />
            ) : (
              <p className="text-gray-500 text-center py-16">
                Loading graph data...
              </p>
            )}
          </CardContent>
        </Card>
      )}

      {activeTab === "stats" && (
        <StatsView stats={stats ?? null} />
      )}
    </div>
  );
}
