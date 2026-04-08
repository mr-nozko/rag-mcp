'use client';

import { useEffect, useRef } from 'react';
import { animate } from 'animejs';
import type { PageIndexStats } from '@/lib/types';

interface Props {
  data: PageIndexStats;
}

export default function PageIndexPanel({ data }: Props) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (panelRef.current) {
      animate(panelRef.current, {
        opacity: [0, 1],
        translateY: [20, 0],
        duration: 800,
        ease: 'easeOutElastic(1, .8)',
      });
    }
  }, []);

  return (
    <div
      ref={panelRef}
      className="bg-slate-800/80 border border-fuchsia-500/30 rounded-xl shadow-lg p-6 text-white transition-all hover:shadow-2xl hover:shadow-fuchsia-500/10 hover:-translate-y-1 relative overflow-hidden group"
    >
      {/* Title */}
      <div className="flex justify-between items-center mb-6">
        <h3 className="text-lg font-bold text-slate-200 tracking-wider">
          <span className="text-fuchsia-400 mr-2">⚗️</span> 
          PageIndex (Reasoning RAG)
        </h3>
        
        {/* Status badge */}
        <span className={`px-3 py-1 rounded-full text-xs font-semibold border flex items-center gap-2 ${
          data.isHealthy 
            ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20' 
            : 'bg-red-500/10 text-red-400 border-red-500/20'
        }`}>
          <span className={`w-2 h-2 rounded-full ${data.isHealthy ? 'bg-emerald-400' : 'bg-red-400'} animate-pulse`}></span>
          {data.isHealthy ? 'Sidecar Offline/Ready' : 'Sidecar Unhealthy'}
        </span>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        {/* Metric 1 */}
        <div className="flex flex-col border-r border-slate-700/50 pr-4">
          <span className="text-xs text-slate-400 mb-1">Indexed Docs</span>
          <span className="text-2xl font-bold text-white">
            {data.indexedDocs} <span className="text-sm text-slate-500">/ {data.eligibleDocs}</span>
          </span>
        </div>

        {/* Metric 2 */}
        <div className="flex flex-col border-r border-slate-700/50 pr-4">
          <span className="text-xs text-slate-400 mb-1">Avg Tree Nodes</span>
          <span className="text-2xl font-bold text-white">{data.avgTreeNodes}</span>
        </div>

        {/* Metric 3 */}
        <div className="flex flex-col border-r border-slate-700/50 pr-4">
          <span className="text-xs text-slate-400 mb-1">Queries Today</span>
          <span className="text-2xl font-bold text-white">{data.queriesToday}</span>
        </div>

        {/* Metric 4 */}
        <div className="flex flex-col border-r border-slate-700/50 pr-4">
          <span className="text-xs text-slate-400 mb-1">Avg Iterations</span>
          <span className="text-2xl font-bold text-white">{data.avgIterations.toFixed(1)}</span>
        </div>

        {/* Metric 5 */}
        <div className="flex flex-col">
          <span className="text-xs text-slate-400 mb-1">Avg Latency</span>
          <span className="text-2xl font-bold text-amber-400">{data.avgLatencyMs.toLocaleString()}ms</span>
        </div>
      </div>

      {/* Subtle hover accent line */}
      <div className="absolute bottom-0 left-0 right-0 h-1 bg-gradient-to-r from-transparent via-fuchsia-500 to-transparent opacity-0 group-hover:opacity-100 transition-opacity" />
    </div>
  );
}
