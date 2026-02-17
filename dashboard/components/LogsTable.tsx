'use client';

import type { QueryLog } from '@/lib/types';

interface Props {
  logs: QueryLog[];
}

export default function LogsTable({ logs }: Props) {
  function formatTimestamp(timestamp: string): string {
    const date = new Date(timestamp);
    return date.toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  }

  function getLatencyColor(latencyMs: number | null): string {
    if (!latencyMs) return 'bg-slate-700 text-slate-400';
    if (latencyMs < 500) return 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30';
    if (latencyMs < 1000) return 'bg-amber-500/10 text-amber-400 border-amber-500/30';
    return 'bg-red-500/10 text-red-400 border-red-500/30';
  }

  function getMethodColor(method: string | null): string {
    const methodMap: Record<string, string> = {
      'vector': 'bg-sky-500/10 text-sky-400 border-sky-500/20',
      'hybrid': 'bg-purple-500/10 text-purple-400 border-purple-500/20',
      'keyword': 'bg-amber-500/10 text-amber-400 border-amber-500/20',
    };
    return methodMap[method?.toLowerCase() || ''] || 'bg-slate-700/50 text-slate-400 border-slate-600';
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-slate-700/30">
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Time</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Query</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Method</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Results</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Latency</th>
          </tr>
        </thead>
        <tbody>
          {logs.map((log) => (
            <tr 
              key={log.queryId} 
              className="border-b border-slate-700/30 hover:bg-slate-800/50 transition-all group relative"
            >
              {/* Left accent border on hover */}
              <td className="py-4 text-slate-500 text-xs whitespace-nowrap relative">
                <div className="absolute left-0 top-0 bottom-0 w-1 bg-emerald-500 opacity-0 group-hover:opacity-100 transition-opacity" />
                {formatTimestamp(log.timestamp)}
              </td>
              <td className="py-4 text-slate-300 max-w-md">
                <span className="font-mono text-xs truncate block">
                  {log.queryText}
                </span>
              </td>
              <td className="py-4">
                <span className={`inline-block px-3 py-1 rounded-md text-xs font-semibold border ${getMethodColor(log.retrievalMethod)}`}>
                  {log.retrievalMethod || 'N/A'}
                </span>
              </td>
              <td className="py-4 text-slate-300 font-medium">{log.resultCount || 0}</td>
              <td className="py-4">
                <span className={`inline-block px-3 py-1 rounded-md font-bold text-xs border ${getLatencyColor(log.latencyMs)}`}>
                  {log.latencyMs ? `${log.latencyMs}ms` : 'N/A'}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
