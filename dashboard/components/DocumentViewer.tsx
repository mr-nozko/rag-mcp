'use client';

import { useEffect, useState } from 'react';
import type { DocumentDetails, Document } from '@/lib/types';

interface Props {
  documentId: string | null;
}

export default function DocumentViewer({ documentId }: Props) {
  const [document, setDocument] = useState<DocumentDetails | null>(null);
  const [relatedDocs, setRelatedDocs] = useState<Document[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!documentId) {
      setDocument(null);
      setRelatedDocs([]);
      return;
    }

    async function fetchDocument() {
      try {
        setLoading(true);
        setError(null);
        
        const response = await fetch(`/api/documents/${encodeURIComponent(documentId)}`);
        
        if (!response.ok) {
          throw new Error('Failed to fetch document');
        }
        
        const data = await response.json();
        setDocument(data.document);
        setRelatedDocs(data.relatedDocuments || []);
      } catch (err) {
        console.error('Error loading document:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
      } finally {
        setLoading(false);
      }
    }

    fetchDocument();
  }, [documentId]);

  if (!documentId) {
    return (
      <div className="h-full flex items-center justify-center rounded-xl border border-slate-800 bg-transparent">
        <div className="text-center">
          <svg className="w-16 h-16 text-slate-600 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
          </svg>
          <p className="text-slate-400 font-semibold">Select a node</p>
          <p className="text-slate-500 text-sm mt-2">Click on a graph node to view document details</p>
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center rounded-xl border border-slate-800 bg-transparent">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-emerald-500 mx-auto mb-4"></div>
          <p className="text-slate-400">Loading document...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center rounded-xl border border-slate-800 bg-transparent">
        <div className="text-center">
          <svg className="w-16 h-16 text-red-400 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <p className="text-red-400 font-semibold">Error loading document</p>
          <p className="text-slate-500 text-sm mt-2">{error}</p>
        </div>
      </div>
    );
  }

  if (!document) {
    return (
      <div className="h-full flex items-center justify-center rounded-xl border border-slate-800 bg-transparent">
        <div className="text-center">
          <p className="text-slate-400">Document not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full rounded-xl border border-slate-800 overflow-hidden flex flex-col bg-transparent">
      {/* Scrollable Content */}
      <div className="flex-1 overflow-y-auto p-6 space-y-6 bg-transparent">
        {/* Metadata */}
        <div>
          <h2 className="text-2xl font-bold text-white mb-4 tracking-tight">Document Details</h2>
          
          <div className="grid grid-cols-2 gap-4 rounded-lg p-4 border border-slate-700">
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Path</span>
              <span className="text-sm text-slate-200 font-mono break-all">{document.docPath}</span>
            </div>
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Namespace</span>
              <span className="inline-block px-3 py-1 rounded-md bg-emerald-500/10 text-emerald-400 text-sm font-semibold border border-emerald-500/20">
                {document.namespace}
              </span>
            </div>
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Type</span>
              <span className="text-sm text-slate-200">{document.docType}</span>
            </div>
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Agent</span>
              <span className="text-sm text-slate-200">{document.agentName || 'N/A'}</span>
            </div>
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Tokens</span>
              <span className="text-sm text-slate-200">{document.contentTokens.toLocaleString()}</span>
            </div>
            <div>
              <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider block mb-1">Last Modified</span>
              <span className="text-sm text-slate-200">{new Date(document.lastModified).toLocaleString()}</span>
            </div>
          </div>
        </div>

        {/* Content Preview */}
        <div>
          <h3 className="text-lg font-bold text-white mb-3">Content</h3>
          <div className="rounded-lg p-4 border border-slate-700">
            <pre className="text-sm text-slate-300 whitespace-pre-wrap font-mono overflow-x-auto max-h-96">
              {document.contentText.length > 2000 
                ? `${document.contentText.substring(0, 2000)}...\n\n[Content truncated - ${document.contentText.length} total characters]`
                : document.contentText
              }
            </pre>
          </div>
        </div>

        {/* Chunks */}
        <div>
          <h3 className="text-lg font-bold text-white mb-3">
            Chunks <span className="text-slate-400 text-sm font-normal">({document.chunks.length})</span>
          </h3>
          <div className="space-y-3">
            {document.chunks.slice(0, 10).map((chunk) => (
              <div key={chunk.chunkId} className="rounded-lg p-4 border border-slate-700">
                <div className="flex items-start justify-between mb-2">
                  <div className="flex-1">
                    {chunk.sectionHeader && (
                      <span className="text-xs font-semibold text-emerald-400 block mb-1">
                        {chunk.sectionHeader}
                      </span>
                    )}
                    <span className="text-xs text-slate-500 font-mono">{chunk.chunkId}</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-slate-400">{chunk.chunkTokens} tokens</span>
                    {chunk.embedding && (
                      <span className="px-2 py-1 bg-emerald-500/10 text-emerald-400 text-xs font-semibold rounded border border-emerald-500/20">
                        Embedded
                      </span>
                    )}
                  </div>
                </div>
                <p className="text-sm text-slate-300 line-clamp-3">{chunk.chunkText}</p>
              </div>
            ))}
            {document.chunks.length > 10 && (
              <p className="text-center text-sm text-slate-500">
                ... and {document.chunks.length - 10} more chunks
              </p>
            )}
          </div>
        </div>

        {/* Relations */}
        {document.relations.length > 0 && (
          <div>
            <h3 className="text-lg font-bold text-white mb-3">
              Knowledge Graph Relations <span className="text-slate-400 text-sm font-normal">({document.relations.length})</span>
            </h3>
            <div className="space-y-2">
              {document.relations.map((rel) => (
                <div key={rel.relationId} className="rounded-lg p-3 border border-slate-700 flex items-center gap-3">
                  <span className="text-sm text-slate-300 font-mono flex-shrink-0">{rel.sourceEntity}</span>
                  <svg className="w-4 h-4 text-slate-500 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 7l5 5m0 0l-5 5m5-5H6" />
                  </svg>
                  <span className="px-2 py-1 bg-sky-500/10 text-sky-400 text-xs font-semibold rounded border border-sky-500/20 flex-shrink-0">
                    {rel.relationType}
                  </span>
                  <svg className="w-4 h-4 text-slate-500 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 7l5 5m0 0l-5 5m5-5H6" />
                  </svg>
                  <span className="text-sm text-slate-300 font-mono flex-1 truncate">{rel.targetEntity}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Related Documents */}
        {relatedDocs.length > 0 && (
          <div>
            <h3 className="text-lg font-bold text-white mb-3">
              Related Documents <span className="text-slate-400 text-sm font-normal">({relatedDocs.length})</span>
            </h3>
            <div className="space-y-2">
              {relatedDocs.map((doc) => (
                <div key={doc.docId} className="rounded-lg p-3 border border-slate-700 hover:border-slate-600 transition-colors">
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <span className="text-sm text-slate-200 font-mono block truncate">{doc.docPath}</span>
                      <span className="text-xs text-slate-500 mt-1 block">{doc.namespace}</span>
                    </div>
                    <span className="text-xs text-slate-400 flex-shrink-0">{doc.contentTokens} tokens</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
