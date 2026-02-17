'use client';

import { useEffect, useState } from 'react';
import type { MissingEmbedding } from '@/lib/queries';

export default function MissingEmbeddingsList() {
  const [chunks, setChunks] = useState<MissingEmbedding[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchTerm, setSearchTerm] = useState('');
  
  const limit = 20;

  useEffect(() => {
    async function fetchMissing() {
      try {
        setLoading(true);
        const offset = page * limit;
        const response = await fetch(`/api/embeddings?action=missing&limit=${limit}&offset=${offset}`);
        
        if (!response.ok) {
          throw new Error('Failed to fetch missing embeddings');
        }
        
        const data = await response.json();
        setChunks(data.chunks || []);
        setTotal(data.total || 0);
        setError(null);
      } catch (err) {
        console.error('Error loading missing embeddings:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
      } finally {
        setLoading(false);
      }
    }
    
    fetchMissing();
  }, [page]);

  const filteredChunks = chunks.filter(chunk => 
    !searchTerm || 
    chunk.docPath.toLowerCase().includes(searchTerm.toLowerCase()) ||
    chunk.chunkId.toLowerCase().includes(searchTerm.toLowerCase())
  );

  const totalPages = Math.ceil(total / limit);
  const hasNext = page < totalPages - 1;
  const hasPrev = page > 0;

  if (loading && chunks.length === 0) {
    return (
      <div className="border border-slate-700 rounded-xl p-6 bg-transparent">
        <div className="animate-pulse space-y-4">
          <div className="h-4 bg-slate-700 rounded w-1/4"></div>
          {[1, 2, 3].map((i) => (
            <div key={i} className="h-20 bg-slate-700 rounded"></div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="border border-slate-700 rounded-xl p-6 space-y-4 bg-transparent">
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-xl font-bold text-white mb-2">Missing Embeddings</h3>
          <p className="text-slate-400 text-sm">
            {total > 0 ? (
              <>
                <span className="font-bold text-amber-400">{total.toLocaleString()}</span> chunks without embeddings
              </>
            ) : (
              'All chunks have embeddings!'
            )}
          </p>
        </div>

        {/* Search */}
        <div className="flex-shrink-0 w-64">
          <input
            type="text"
            placeholder="Search by path or ID..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full px-4 py-2 bg-slate-950 border border-slate-700 rounded-lg text-slate-200 text-sm placeholder:text-slate-500 focus:border-emerald-500 focus:outline-none"
          />
        </div>
      </div>

      {error && (
        <div className="bg-red-500/10 border border-red-500/30 rounded-lg p-4">
          <p className="text-red-400 text-sm">{error}</p>
        </div>
      )}

      {/* List */}
      {filteredChunks.length === 0 ? (
        <div className="text-center py-12">
          <svg className="w-16 h-16 text-slate-600 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <p className="text-slate-400 font-semibold">
            {searchTerm ? 'No matching chunks found' : 'All chunks have embeddings!'}
          </p>
          {searchTerm && (
            <p className="text-slate-500 text-sm mt-2">Try a different search term</p>
          )}
        </div>
      ) : (
        <div className="space-y-3">
          {filteredChunks.map((chunk) => (
            <div key={chunk.chunkId} className="border border-slate-700 rounded-lg p-4 hover:border-slate-600 transition-colors">
              <div className="flex items-start justify-between gap-4 mb-3">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-xs font-mono text-slate-500 truncate">{chunk.chunkId}</span>
                  </div>
                  <span className="text-sm text-slate-300 font-mono block truncate">{chunk.docPath}</span>
                  {chunk.sectionHeader && (
                    <span className="text-xs text-emerald-400 font-semibold mt-1 block">
                      {chunk.sectionHeader}
                    </span>
                  )}
                </div>
                <div className="flex-shrink-0 text-right">
                  <span className="text-xs text-slate-400">{chunk.chunkTokens} tokens</span>
                </div>
              </div>
              
              <p className="text-sm text-slate-400 line-clamp-2">{chunk.chunkPreview}...</p>
            </div>
          ))}
        </div>
      )}

      {/* Pagination */}
      {total > limit && (
        <div className="flex items-center justify-between pt-4 border-t border-slate-700">
          <div className="text-sm text-slate-400">
            Page {page + 1} of {totalPages} 
            <span className="mx-2">•</span>
            Showing {page * limit + 1}-{Math.min((page + 1) * limit, total)} of {total.toLocaleString()}
          </div>
          
          <div className="flex gap-2">
            <button
              onClick={() => setPage(p => Math.max(0, p - 1))}
              disabled={!hasPrev || loading}
              className="px-4 py-2 bg-slate-700 hover:bg-slate-600 text-slate-200 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed text-sm font-medium"
            >
              Previous
            </button>
            <button
              onClick={() => setPage(p => p + 1)}
              disabled={!hasNext || loading}
              className="px-4 py-2 bg-slate-700 hover:bg-slate-600 text-slate-200 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed text-sm font-medium"
            >
              Next
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
