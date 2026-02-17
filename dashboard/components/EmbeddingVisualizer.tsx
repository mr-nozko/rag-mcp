'use client';

import { useEffect, useState } from 'react';
import type { Document, Chunk } from '@/lib/types';

export default function EmbeddingVisualizer() {
  const [documents, setDocuments] = useState<Document[]>([]);
  const [selectedDoc, setSelectedDoc] = useState<string | null>(null);
  const [chunks, setChunks] = useState<Chunk[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingChunks, setLoadingChunks] = useState(false);

  useEffect(() => {
    async function fetchDocuments() {
      try {
        setLoading(true);
        const response = await fetch('/api/documents?limit=50');
        
        if (!response.ok) {
          throw new Error('Failed to fetch documents');
        }
        
        const data = await response.json();
        setDocuments(data.documents || []);
      } catch (err) {
        console.error('Error loading documents:', err);
      } finally {
        setLoading(false);
      }
    }
    
    fetchDocuments();
  }, []);

  useEffect(() => {
    if (!selectedDoc) {
      setChunks([]);
      return;
    }

    async function fetchChunks() {
      try {
        setLoadingChunks(true);
        const response = await fetch(`/api/documents/${encodeURIComponent(selectedDoc)}`);
        
        if (!response.ok) {
          throw new Error('Failed to fetch document');
        }
        
        const data = await response.json();
        setChunks(data.document?.chunks || []);
      } catch (err) {
        console.error('Error loading chunks:', err);
      } finally {
        setLoadingChunks(false);
      }
    }

    fetchChunks();
  }, [selectedDoc]);

  function getEmbeddingCoverage(doc: Document): number {
    // This would need to be calculated from chunks, but we don't have that info in Document type
    // For now, return a placeholder
    return 0;
  }

  if (loading) {
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
    <div className="border border-slate-700 rounded-xl p-6 bg-transparent">
      <h3 className="text-xl font-bold text-white mb-4">Embedding Visualization by Document</h3>
      <p className="text-slate-400 text-sm mb-6">Select a document to view its chunks and embedding status</p>

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* Document List */}
        <div className="space-y-3 max-h-[600px] overflow-y-auto">
          <h4 className="text-sm font-bold text-slate-300 uppercase tracking-wider sticky top-0 bg-slate-950 pb-2">
            Documents ({documents.length})
          </h4>
          {documents.length === 0 ? (
            <div className="text-center py-12">
              <p className="text-slate-400">No documents available</p>
            </div>
          ) : (
            documents.map((doc) => {
              const isSelected = selectedDoc === doc.docId;
              return (
                <button
                  key={doc.docId}
                  onClick={() => setSelectedDoc(doc.docId)}
                  className={`w-full text-left p-4 rounded-lg border transition-all ${
                    isSelected
                      ? 'bg-emerald-500/10 border-emerald-500/30'
                      : 'border-slate-700 hover:border-slate-600'
                  }`}
                >
                  <div className="flex items-start justify-between gap-3 mb-2">
                    <span className="text-sm text-slate-200 font-mono truncate flex-1">
                      {doc.docPath.split('/').pop()}
                    </span>
                    <span className={`px-2 py-1 rounded text-xs font-semibold ${
                      isSelected
                        ? 'bg-emerald-500/20 text-emerald-400'
                        : 'bg-slate-700 text-slate-400'
                    }`}>
                      {doc.namespace}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 text-xs text-slate-500">
                    <span>{doc.contentTokens} tokens</span>
                    <span>•</span>
                    <span>{new Date(doc.lastModified).toLocaleDateString()}</span>
                  </div>
                </button>
              );
            })
          )}
        </div>

        {/* Chunks Display */}
        <div className="space-y-3 max-h-[600px] overflow-y-auto">
          <h4 className="text-sm font-bold text-slate-300 uppercase tracking-wider sticky top-0 bg-slate-950 pb-2">
            Chunks {chunks.length > 0 && `(${chunks.length})`}
          </h4>
          
          {!selectedDoc ? (
            <div className="text-center py-12">
              <svg className="w-12 h-12 text-slate-600 mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 15l-2 5L9 9l11 4-5 2zm0 0l5 5M7.188 2.239l.777 2.897M5.136 7.965l-2.898-.777M13.95 4.05l-2.122 2.122m-5.657 5.656l-2.12 2.122" />
              </svg>
              <p className="text-slate-400">Select a document to view chunks</p>
            </div>
          ) : loadingChunks ? (
            <div className="text-center py-12">
              <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-emerald-500 mx-auto mb-3"></div>
              <p className="text-slate-400">Loading chunks...</p>
            </div>
          ) : chunks.length === 0 ? (
            <div className="text-center py-12">
              <p className="text-slate-400">No chunks found</p>
            </div>
          ) : (
            <>
              {/* Summary */}
              <div className="border border-slate-700 rounded-lg p-4 mb-4">
                <div className="flex items-center justify-between">
                  <span className="text-sm text-slate-400">Embedding Coverage</span>
                  <span className="text-lg font-bold text-white">
                    {chunks.filter(c => c.embedding).length} / {chunks.length}
                  </span>
                </div>
                <div className="mt-2 bg-slate-800 rounded-full h-2 overflow-hidden">
                  <div 
                    className="bg-gradient-to-r from-emerald-500 to-emerald-400 h-2 transition-all duration-500"
                    style={{ width: `${(chunks.filter(c => c.embedding).length / chunks.length) * 100}%` }}
                  />
                </div>
              </div>

              {/* Chunk List */}
              {chunks.map((chunk) => (
                <div
                  key={chunk.chunkId}
                  className={`p-4 rounded-lg border ${
                    chunk.embedding
                      ? 'bg-emerald-500/5 border-emerald-500/30'
                      : 'bg-red-500/5 border-red-500/30'
                  }`}
                >
                  <div className="flex items-start justify-between gap-3 mb-2">
                    <div className="flex-1 min-w-0">
                      {chunk.sectionHeader && (
                        <span className="text-xs font-semibold text-emerald-400 block mb-1">
                          {chunk.sectionHeader}
                        </span>
                      )}
                      <span className="text-xs text-slate-500 font-mono block">
                        Chunk {chunk.chunkIndex + 1}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 flex-shrink-0">
                      <span className="text-xs text-slate-400">{chunk.chunkTokens} tokens</span>
                      {chunk.embedding ? (
                        <span className="px-2 py-1 bg-emerald-500/20 text-emerald-400 text-xs font-bold rounded border border-emerald-500/30">
                          ✓ Embedded
                        </span>
                      ) : (
                        <span className="px-2 py-1 bg-red-500/20 text-red-400 text-xs font-bold rounded border border-red-500/30">
                          ✗ Missing
                        </span>
                      )}
                    </div>
                  </div>
                  <p className="text-sm text-slate-300 line-clamp-2">{chunk.chunkText}</p>
                </div>
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
