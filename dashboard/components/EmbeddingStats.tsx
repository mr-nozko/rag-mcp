'use client';

import { useEffect, useState } from 'react';
import type { EmbeddingStats as EmbeddingStatsType } from '@/lib/queries';

export default function EmbeddingStats() {
  const [stats, setStats] = useState<EmbeddingStatsType | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    async function fetchStats() {
      try {
        setLoading(true);
        const response = await fetch('/api/embeddings?action=stats');
        
        if (!response.ok) {
          throw new Error('Failed to fetch embedding stats');
        }
        
        const data = await response.json();
        setStats(data);
        setError(null);
      } catch (err) {
        console.error('Error loading embedding stats:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
      } finally {
        setLoading(false);
      }
    }
    
    fetchStats();
  }, []);

  function getStatusColor(coverage: number): string {
    if (coverage >= 90) return 'emerald';
    if (coverage >= 70) return 'sky';
    if (coverage >= 50) return 'amber';
    return 'red';
  }

  function getStatusLabel(coverage: number): string {
    if (coverage >= 90) return 'Excellent';
    if (coverage >= 70) return 'Good';
    if (coverage >= 50) return 'Fair';
    return 'Poor';
  }

  if (loading) {
    return (
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {[1, 2, 3].map((i) => (
          <div key={i} className="border border-slate-700 rounded-xl p-6 animate-pulse bg-transparent">
            <div className="h-4 bg-slate-700 rounded w-1/2 mb-4"></div>
            <div className="h-8 bg-slate-700 rounded w-3/4"></div>
          </div>
        ))}
      </div>
    );
  }

  if (error || !stats) {
    return (
      <div className="bg-red-500/10 border border-red-500/30 rounded-xl p-6">
        <p className="text-red-400">Error loading embedding stats: {error}</p>
      </div>
    );
  }

  const statusColor = getStatusColor(stats.coveragePercent);
  const statusLabel = getStatusLabel(stats.coveragePercent);

  return (
    <div className="space-y-6">
      {/* Overview Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <div className="border border-slate-700 rounded-xl p-6 bg-transparent">
          <h3 className="text-xs font-bold text-slate-400 uppercase tracking-wider mb-2">
            Total Chunks
          </h3>
          <div className="text-4xl font-extrabold text-white">
            {stats.totalChunks.toLocaleString()}
          </div>
        </div>

        <div className="border border-slate-700 rounded-xl p-6 bg-transparent">
          <h3 className="text-xs font-bold text-slate-400 uppercase tracking-wider mb-2">
            Embedded Chunks
          </h3>
          <div className="text-4xl font-extrabold text-white">
            {stats.embeddedChunks.toLocaleString()}
          </div>
        </div>

        <div className={`border border-${statusColor}-500/30 rounded-xl p-6 relative overflow-hidden bg-transparent`}>
          <div className="flex items-start justify-between mb-2">
            <h3 className="text-xs font-bold text-slate-400 uppercase tracking-wider">
              Coverage
            </h3>
            <span className={`px-3 py-1 rounded-full text-xs font-semibold bg-${statusColor}-500/10 text-${statusColor}-400 border border-${statusColor}-500/20`}>
              {statusLabel}
            </span>
          </div>
          <div className="text-4xl font-extrabold text-white">
            {stats.coveragePercent}%
          </div>
          <div className={`absolute bottom-0 left-0 right-0 h-1 bg-gradient-to-r from-transparent via-${statusColor}-500 to-transparent`} />
        </div>
      </div>

      {/* Namespace Breakdown */}
      <div className="border border-slate-700 rounded-xl p-6 bg-transparent">
        <h3 className="text-xl font-bold text-white mb-4">Coverage by Namespace</h3>
        
        {stats.byNamespace.length === 0 ? (
          <p className="text-slate-400 text-center py-8">No namespace data available</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b border-slate-700/30">
                  <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Namespace</th>
                  <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Total Chunks</th>
                  <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Embedded</th>
                  <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Coverage</th>
                  <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Status</th>
                </tr>
              </thead>
              <tbody>
                {stats.byNamespace.map((ns) => {
                  const nsStatusColor = getStatusColor(ns.coveragePercent);
                  const nsStatusLabel = getStatusLabel(ns.coveragePercent);
                  
                  return (
                    <tr key={ns.namespace} className="border-b border-slate-700/30 hover:border-slate-600 transition-colors">
                      <td className="py-4">
                        <span className="inline-block px-3 py-1 rounded-md bg-emerald-500/10 text-emerald-400 text-sm font-semibold border border-emerald-500/20">
                          {ns.namespace}
                        </span>
                      </td>
                      <td className="py-4 text-slate-300 font-medium">{ns.totalChunks}</td>
                      <td className="py-4 text-slate-300 font-medium">{ns.embeddedChunks}</td>
                      <td className="py-4">
                        <div className="flex items-center gap-3">
                          <div className="flex-1 bg-slate-700 rounded-full h-2.5 max-w-[120px] overflow-hidden">
                            <div
                              className={`bg-gradient-to-r from-${nsStatusColor}-500 to-${nsStatusColor}-400 h-2.5 rounded-full transition-all duration-500`}
                              style={{ width: `${ns.coveragePercent}%` }}
                            />
                          </div>
                          <span className="text-slate-300 text-sm font-bold min-w-[45px]">{ns.coveragePercent}%</span>
                        </div>
                      </td>
                      <td className="py-4">
                        <span className={`px-3 py-1 rounded-md text-xs font-semibold bg-${nsStatusColor}-500/10 text-${nsStatusColor}-400 border border-${nsStatusColor}-500/20`}>
                          {nsStatusLabel}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
