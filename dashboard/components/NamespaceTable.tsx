'use client';

import type { NamespaceStats } from '@/lib/types';

interface Props {
  namespaces: NamespaceStats[];
}

export default function NamespaceTable({ namespaces }: Props) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left">
        <thead>
          <tr className="border-b border-slate-700/30">
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">#</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Namespace</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Documents</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Chunks</th>
            <th className="pb-4 text-slate-400 font-bold text-xs uppercase tracking-wider">Embedding Coverage</th>
          </tr>
        </thead>
        <tbody>
          {namespaces.map((ns, index) => (
            <tr 
              key={ns.namespace} 
              className="border-b border-slate-700/30 hover:bg-slate-800/50 transition-colors group"
            >
              <td className="py-4 text-slate-500 text-sm font-medium">{index + 1}</td>
              <td className="py-4">
                <span className="inline-block px-3 py-1 rounded-md bg-emerald-500/10 text-emerald-400 text-sm font-semibold border border-emerald-500/20">
                  {ns.namespace}
                </span>
              </td>
              <td className="py-4 text-slate-300 font-medium text-sm">{ns.docCount}</td>
              <td className="py-4 text-slate-300 font-medium text-sm">{ns.chunkCount}</td>
              <td className="py-4">
                <div className="flex items-center gap-3">
                  <div className="flex-1 bg-slate-800 rounded-full h-2.5 max-w-[120px] overflow-hidden">
                    <div
                      className="bg-gradient-to-r from-emerald-500 to-emerald-400 h-2.5 rounded-full transition-all duration-500"
                      style={{ width: `${ns.embeddingCoverage}%` }}
                    />
                  </div>
                  <span className="text-slate-300 text-sm font-bold min-w-[45px]">{ns.embeddingCoverage}%</span>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
