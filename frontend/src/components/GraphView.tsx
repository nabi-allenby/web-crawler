import { useState, useRef, useCallback, useMemo, useEffect } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";
import { forceRadial } from "d3-force";
import type { GraphData } from "../types/api";

interface GraphViewProps {
  data: GraphData;
}

interface CrawlNode {
  label: string;
  domain: string;
  depth: number;
  status: string;
  nodeType: string;
  val: number;
}

const STATUS_COLORS: Record<string, string> = {
  root: "#3b82f6",
  COMPLETED: "#22c55e",
  PENDING: "#eab308",
  "IN-PROGRESS": "#6366f1",
  FAILED: "#ef4444",
  CANCELLED: "#9ca3af",
};

export function GraphView({ data }: GraphViewProps) {
  const fgRef = useRef<ForceGraphMethods<NodeObject<CrawlNode>> | undefined>(
    undefined
  );
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const needsRecenter = useRef(true);
  const [containerWidth, setContainerWidth] = useState(0);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      setContainerWidth(entries[0].contentRect.width);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const graphData = useMemo(() => {
    const nodes = data.nodes.map((n) => ({
      id: n.id,
      label: n.label,
      domain: n.domain,
      depth: n.depth,
      status: n.status,
      nodeType: n.node_type,
      val: { ROOT: 4, 1: 2.5, 2: 1.5 }[n.node_type === "ROOT" ? "ROOT" : n.depth] ?? 1,
      // Pin root node at origin for stable centering
      ...(n.node_type === "ROOT" ? { fx: 0, fy: 0 } : {}),
    }));

    const links = data.edges.map((e) => ({
      source: e.source,
      target: e.target,
    }));

    return { nodes, links };
  }, [data]);

  const { neighborIds, connectedLinks } = useMemo(() => {
    if (!selectedNode) return { neighborIds: new Set<string>(), connectedLinks: new Set<string>() };
    const nIds = new Set<string>();
    const cLinks = new Set<string>();
    graphData.links.forEach((link) => {
      const src = typeof link.source === "object" ? (link.source as NodeObject<CrawlNode>).id : link.source;
      const tgt = typeof link.target === "object" ? (link.target as NodeObject<CrawlNode>).id : link.target;
      if (src === selectedNode || tgt === selectedNode) {
        nIds.add(src as string);
        nIds.add(tgt as string);
        cLinks.add(`${src}->${tgt}`);
      }
    });
    return { neighborIds: nIds, connectedLinks: cLinks };
  }, [selectedNode, graphData]);

  const activeStatuses = useMemo(() => {
    const statuses = new Set<string>();
    data.nodes.forEach((n) => {
      if (n.node_type === "ROOT") statuses.add("root");
      else if (n.status) statuses.add(n.status);
    });
    return Object.entries(STATUS_COLORS).filter(([s]) => statuses.has(s));
  }, [data]);

  useEffect(() => {
    const fg = fgRef.current;
    if (!fg) return;

    const ringSpacing = 120;

    // Radial force: push nodes into concentric rings by depth
    fg.d3Force(
      "radial",
      forceRadial(
        (node: NodeObject<CrawlNode>) => ((node as CrawlNode).depth ?? 0) * ringSpacing,
        0,
        0
      ).strength(0.8)
    );

    // Link distance based on depth
    fg.d3Force("link")?.distance(
      (link: LinkObject<CrawlNode>) => {
        const src = link.source as NodeObject<CrawlNode>;
        const tgt = link.target as NodeObject<CrawlNode>;
        return 30 + Math.abs((tgt.depth ?? 0) - (src.depth ?? 0)) * 60;
      }
    );

    // Stronger charge to spread nodes within rings
    fg.d3Force("charge")?.strength(-80);

    needsRecenter.current = true;
    fg.d3ReheatSimulation();
  }, [graphData]);

  const handleEngineStop = useCallback(() => {
    const fg = fgRef.current;
    if (!fg || !needsRecenter.current) return;
    needsRecenter.current = false;

    // Root is pinned at (0,0). Center on it and zoom to fit all nodes.
    fg.centerAt(0, 0);
    fg.zoomToFit(400, 40);
  }, []);

  const nodeColor = useCallback(
    (node: NodeObject<CrawlNode>) => {
      const base = STATUS_COLORS[node.status || ""] || "#9ca3af";
      if (!selectedNode) return base;
      if (node.id === selectedNode || neighborIds.has(node.id as string)) return base;
      // Dim unrelated nodes: parse hex to rgba with low opacity
      const r = parseInt(base.slice(1, 3), 16);
      const g = parseInt(base.slice(3, 5), 16);
      const b = parseInt(base.slice(5, 7), 16);
      return `rgba(${r},${g},${b},0.2)`;
    },
    [selectedNode, neighborIds]
  );

  const nodeLabel = useCallback(
    (node: NodeObject<CrawlNode>) => {
      return `${node.label}\nDepth: ${node.depth}\nStatus: ${node.status}`;
    },
    []
  );

  if (data.nodes.length === 0) {
    return (
      <p className="text-gray-500 text-center py-16">
        No graph data available
      </p>
    );
  }

  return (
    <div
      ref={containerRef}
      className="relative border rounded-lg overflow-hidden bg-gray-900"
      style={{ height: 600 }}
    >
      <ForceGraph2D<NodeObject<CrawlNode>>
        ref={fgRef}
        graphData={graphData}
        nodeColor={nodeColor}
        nodeLabel={nodeLabel}
        nodeRelSize={6}
        onNodeClick={(node: NodeObject<CrawlNode>) => {
          setSelectedNode(node.id === selectedNode ? null : (node.id as string));
        }}
        onBackgroundClick={() => setSelectedNode(null)}
        nodeCanvasObjectMode={() => selectedNode ? ("after" as const) : undefined}
        nodeCanvasObject={(node: NodeObject<CrawlNode>, ctx, globalScale) => {
          if (node.id !== selectedNode) return;
          const r = Math.sqrt(node.val ?? 1) * 6 + 2;
          ctx.beginPath();
          ctx.arc(node.x!, node.y!, r, 0, 2 * Math.PI);
          ctx.strokeStyle = "#ffffff";
          ctx.lineWidth = 2 / globalScale;
          ctx.stroke();
        }}
        linkColor={(link: LinkObject<CrawlNode>) => {
          if (selectedNode) {
            const src = typeof link.source === "object" ? (link.source as NodeObject<CrawlNode>).id : link.source;
            const tgt = typeof link.target === "object" ? (link.target as NodeObject<CrawlNode>).id : link.target;
            const key = `${src}->${tgt}`;
            return connectedLinks.has(key) ? "rgba(255,255,255,0.6)" : "rgba(255,255,255,0.03)";
          }
          const depth = Math.max(
            (link.source as NodeObject<CrawlNode>)?.depth ?? 0,
            (link.target as NodeObject<CrawlNode>)?.depth ?? 0
          );
          const opacity = Math.max(0.05, 0.25 - depth * 0.05);
          return `rgba(255,255,255,${opacity})`;
        }}
        linkWidth={(link: LinkObject<CrawlNode>) => {
          if (!selectedNode) return 0.5;
          const src = typeof link.source === "object" ? (link.source as NodeObject<CrawlNode>).id : link.source;
          const tgt = typeof link.target === "object" ? (link.target as NodeObject<CrawlNode>).id : link.target;
          return (src === selectedNode || tgt === selectedNode) ? 2 : 0.5;
        }}
        linkDirectionalArrowLength={3}
        linkDirectionalArrowRelPos={1}
        backgroundColor="#111827"
        onEngineStop={handleEngineStop}
        cooldownTicks={100}
        width={containerWidth || undefined}
        height={600}
      />
      <div className="absolute bottom-4 left-4 flex gap-3 bg-gray-800/80 rounded-lg p-2">
        {activeStatuses.map(([status, color]) => (
          <div key={status} className="flex items-center gap-1">
            <span
              className="w-3 h-3 rounded-full"
              style={{ backgroundColor: color }}
            />
            <span className="text-xs text-gray-300 capitalize">{status}</span>
          </div>
        ))}
      </div>
      {selectedNode && (() => {
        const node = graphData.nodes.find((n) => n.id === selectedNode);
        if (!node) return null;
        return (
          <div className="absolute top-4 right-4 bg-gray-800/90 backdrop-blur rounded-lg p-3 max-w-xs text-sm text-gray-200 space-y-1">
            <div className="flex justify-between items-start gap-2">
              <span className="font-semibold text-white text-xs truncate">{node.label}</span>
              <button
                onClick={() => setSelectedNode(null)}
                className="text-gray-400 hover:text-white shrink-0 leading-none"
              >
                &times;
              </button>
            </div>
            <div className="text-xs">Domain: <span className="text-gray-400">{node.domain}</span></div>
            <div className="text-xs">Depth: <span className="text-gray-400">{node.depth}</span></div>
            <div className="text-xs">Status: <span style={{ color: STATUS_COLORS[node.status] || "#9ca3af" }}>{node.status}</span></div>
            <div className="text-xs">Type: <span className="text-gray-400">{node.nodeType}</span></div>
            <div className="text-xs">Connections: <span className="text-gray-400">{neighborIds.size > 0 ? neighborIds.size - 1 : 0}</span></div>
          </div>
        );
      })()}
    </div>
  );
}
