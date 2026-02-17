'use client';

import { useEffect, useState, useRef } from 'react';
import dynamic from 'next/dynamic';
import type { Document } from '@/lib/types';

// Dynamically import ForceGraph2D to avoid SSR issues
const ForceGraph2D = dynamic(() => import('react-force-graph-2d'), {
  ssr: false,
  loading: () => (
    <div className="flex items-center justify-center h-full">
      <div className="text-slate-400">Loading graph...</div>
    </div>
  ),
});

interface GraphNode {
  id: string;
  label: string;
  namespace: string;
  color: string;
}

interface GraphData {
  nodes: GraphNode[];
  links: Array<{ source: string; target: string; type: string }>;
}

interface Props {
  onNodeClick?: (docId: string) => void;
}

// Get color based on namespace
function getNamespaceColor(namespace: string): string {
  const colorMap: Record<string, string> = {
    agents: '#10b981', // emerald
    business: '#0ea5e9', // sky
    'coding-systems': '#f59e0b', // amber
    community: '#8b5cf6', // violet
    self: '#ec4899', // pink
    system: '#6366f1', // indigo
  };
  return colorMap[namespace] || '#64748b'; // slate default
}

export default function KnowledgeGraph({ onNodeClick }: Props) {
  const [graphData, setGraphData] = useState<GraphData>({ nodes: [], links: [] });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const graphRef = useRef<any>();

  useEffect(() => {
    async function fetchGraphData() {
      try {
        setLoading(true);
        const response = await fetch('/api/graph');
        
        if (!response.ok) {
          throw new Error('Failed to fetch graph data');
        }
        
        const data = await response.json();
        const documents: Document[] = data.nodes || [];
        const links: Array<{ source: string; target: string; relationType: string }> = data.links || [];
        
        // Build graph nodes from documents
        const nodes: GraphNode[] = documents.map((doc) => {
          const fileName = doc.docPath.split('/').pop() || doc.docPath;
          return {
            id: doc.docId,
            label: fileName,
            namespace: doc.namespace,
            color: getNamespaceColor(doc.namespace),
          };
        });
        
        // Format links
        const formattedLinks = links.map((link) => ({
          source: link.source,
          target: link.target,
          type: link.relationType,
        }));
        
        setGraphData({ nodes, links: formattedLinks });
        setError(null);
      } catch (err) {
        console.error('Error loading graph:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
      } finally {
        setLoading(false);
      }
    }
    
    fetchGraphData();
  }, []);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full bg-slate-900/30 rounded-xl border border-slate-800">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-emerald-500 mx-auto mb-4"></div>
          <p className="text-slate-400">Loading knowledge graph...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full bg-slate-900/30 rounded-xl border border-slate-800">
        <div className="text-center">
          <svg className="w-16 h-16 text-red-400 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <p className="text-red-400 font-semibold">Error loading graph</p>
          <p className="text-slate-500 text-sm mt-2">{error}</p>
        </div>
      </div>
    );
  }

  if (graphData.nodes.length === 0) {
    return (
      <div className="flex items-center justify-center h-full bg-slate-900/30 rounded-xl border border-slate-800">
        <div className="text-center">
          <svg className="w-16 h-16 text-slate-600 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
          </svg>
          <p className="text-slate-400 font-semibold">No documents available</p>
          <p className="text-slate-500 text-sm mt-2">Documents will appear here once they are indexed</p>
        </div>
      </div>
    );
  }

  return (
    <div className="relative h-full bg-slate-900/30 rounded-xl border border-slate-800 overflow-hidden">
      {/* Legend */}
      <div className="absolute top-4 right-4 bg-slate-900/90 backdrop-blur-sm border border-slate-700 rounded-lg p-4 z-10">
        <h3 className="text-xs font-bold text-slate-300 uppercase tracking-wider mb-3">Namespaces</h3>
        <div className="space-y-2">
          {['agents', 'business', 'coding-systems', 'community', 'self', 'system'].map((namespace) => (
            <div key={namespace} className="flex items-center gap-2">
              <div 
                className="w-3 h-3 rounded-full" 
                style={{ backgroundColor: getNamespaceColor(namespace) }}
              />
              <span className="text-xs text-slate-400">{namespace}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Graph */}
      <ForceGraph2D
        ref={graphRef}
        graphData={graphData}
        nodeLabel={(node: any) => node.label}
        nodeColor="color"
        nodeRelSize={8}
        linkDistance={80}
        linkStrength={0.3}
        linkDirectionalArrowLength={4}
        linkDirectionalArrowRelPos={1}
        linkCurvature={0.15}
        linkLabel="type"
        d3AlphaDecay={0.015}
        d3VelocityDecay={0.4}
        chargeStrength={-200}
        cooldownTicks={200}
        warmupTicks={0}
        onNodeClick={(node: any) => {
          if (onNodeClick) {
            onNodeClick(node.id);
          }
        }}
        backgroundColor="#0f172a00"
        linkColor={() => '#475569'}
        nodeCanvasObjectMode={() => 'after'}
        nodeCanvasObject={(node: any, ctx: CanvasRenderingContext2D) => {
          // Draw node label (filename)
          const label = node.label || node.id;
          const fontSize = 11;
          ctx.font = `${fontSize}px Inter, sans-serif`;
          ctx.textAlign = 'center';
          ctx.textBaseline = 'middle';
          ctx.fillStyle = '#e2e8f0';
          ctx.fillText(label.length > 25 ? label.substring(0, 22) + '...' : label, node.x, node.y + 18);
        }}
      />

      {/* Stats */}
      <div className="absolute bottom-4 left-4 bg-slate-900/90 backdrop-blur-sm border border-slate-700 rounded-lg px-4 py-2">
        <span className="text-xs text-slate-400">
          <span className="font-bold text-emerald-400">{graphData.nodes.length}</span> documents
          <span className="mx-2">•</span>
          <span className="font-bold text-sky-400">{graphData.links.length}</span> relations
        </span>
      </div>
    </div>
  );
}
