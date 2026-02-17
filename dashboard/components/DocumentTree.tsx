'use client';

import { useEffect, useState } from 'react';
import type { Document } from '@/lib/types';

interface Props {
  onDocumentClick?: (docId: string) => void;
  selectedDocId?: string | null;
}

interface NamespaceGroup {
  namespace: string;
  documents: Document[];
  color: string;
}

function getNamespaceColor(namespace: string): string {
  const colorMap: Record<string, string> = {
    agents: 'emerald',
    business: 'sky',
    'coding-systems': 'amber',
    community: 'violet',
    self: 'pink',
    system: 'indigo',
  };
  return colorMap[namespace] || 'slate';
}

export default function DocumentTree({ onDocumentClick, selectedDocId }: Props) {
  const [documents, setDocuments] = useState<Document[]>([]);
  const [namespaceGroups, setNamespaceGroups] = useState<NamespaceGroup[]>([]);
  const [expandedNamespaces, setExpandedNamespaces] = useState<Set<string>>(new Set());
  const [searchTerm, setSearchTerm] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    async function fetchDocuments() {
      try {
        setLoading(true);
        const response = await fetch('/api/documents?limit=200');
        
        if (!response.ok) {
          throw new Error('Failed to fetch documents');
        }
        
        const data = await response.json();
        const docs: Document[] = data.documents || [];
        setDocuments(docs);
        
        // Group by namespace
        const groups = new Map<string, Document[]>();
        docs.forEach(doc => {
          if (!groups.has(doc.namespace)) {
            groups.set(doc.namespace, []);
          }
          groups.get(doc.namespace)!.push(doc);
        });
        
        const namespaceGroupsArray: NamespaceGroup[] = Array.from(groups.entries()).map(([namespace, docs]) => ({
          namespace,
          documents: docs.sort((a, b) => a.docPath.localeCompare(b.docPath)),
          color: getNamespaceColor(namespace),
        }));
        
        setNamespaceGroups(namespaceGroupsArray);
        
        // Expand all namespaces by default
        setExpandedNamespaces(new Set(namespaceGroupsArray.map(g => g.namespace)));
        
        setError(null);
      } catch (err) {
        console.error('Error loading documents:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
      } finally {
        setLoading(false);
      }
    }
    
    fetchDocuments();
  }, []);

  function toggleNamespace(namespace: string) {
    const newExpanded = new Set(expandedNamespaces);
    if (newExpanded.has(namespace)) {
      newExpanded.delete(namespace);
    } else {
      newExpanded.add(namespace);
    }
    setExpandedNamespaces(newExpanded);
  }

  function toggleAll() {
    if (expandedNamespaces.size === namespaceGroups.length) {
      setExpandedNamespaces(new Set());
    } else {
      setExpandedNamespaces(new Set(namespaceGroups.map(g => g.namespace)));
    }
  }

  const filteredGroups = namespaceGroups.map(group => ({
    ...group,
    documents: group.documents.filter(doc => 
      !searchTerm || 
      doc.docPath.toLowerCase().includes(searchTerm.toLowerCase()) ||
      doc.agentName?.toLowerCase().includes(searchTerm.toLowerCase())
    ),
  })).filter(group => group.documents.length > 0);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full rounded-xl border border-slate-800">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-emerald-500 mx-auto mb-4"></div>
          <p className="text-slate-400">Loading documents...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full rounded-xl border border-slate-800">
        <div className="text-center">
          <svg className="w-16 h-16 text-red-400 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <p className="text-red-400 font-semibold">Error loading documents</p>
          <p className="text-slate-500 text-sm mt-2">{error}</p>
        </div>
      </div>
    );
  }

  const totalDocs = documents.length;
  const visibleDocs = filteredGroups.reduce((sum, g) => sum + g.documents.length, 0);

  return (
    <div className="h-full rounded-xl border border-slate-800 overflow-hidden flex flex-col bg-transparent">
      {/* Header with Search */}
      <div className="p-4 border-b border-slate-700 space-y-3 bg-transparent">
        <div className="flex items-center justify-between">
          <h3 className="text-lg font-bold text-white">Document Explorer</h3>
          <button
            onClick={toggleAll}
            className="text-xs text-slate-400 hover:text-emerald-400 transition-colors px-3 py-1 border border-slate-700 rounded hover:border-emerald-500/30"
          >
            {expandedNamespaces.size === namespaceGroups.length ? 'Collapse All' : 'Expand All'}
          </button>
        </div>
        
        <div className="relative">
          <input
            type="text"
            placeholder="Search documents..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full px-4 py-2 pl-10 bg-slate-950 border border-slate-700 rounded-lg text-slate-200 text-sm placeholder:text-slate-500 focus:border-emerald-500 focus:outline-none"
          />
          <svg className="w-4 h-4 text-slate-500 absolute left-3 top-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
        </div>
        
        <div className="text-xs text-slate-500">
          Showing <span className="font-bold text-slate-300">{visibleDocs}</span> of <span className="font-bold text-slate-300">{totalDocs}</span> documents
        </div>
      </div>

      {/* Tree View */}
      <div className="flex-1 overflow-y-auto p-4 space-y-2 bg-transparent">
        {filteredGroups.length === 0 ? (
          <div className="text-center py-12">
            <svg className="w-12 h-12 text-slate-600 mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
            <p className="text-slate-400">No documents match your search</p>
          </div>
        ) : (
          filteredGroups.map((group) => {
            const isExpanded = expandedNamespaces.has(group.namespace);
            
            return (
              <div key={group.namespace} className="space-y-1">
                {/* Namespace Header */}
                <button
                  onClick={() => toggleNamespace(group.namespace)}
                  className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg transition-all border border-${group.color}-500/30 hover:border-${group.color}-500/50`}
                >
                  <svg 
                    className={`w-4 h-4 text-${group.color}-400 transition-transform ${isExpanded ? 'rotate-90' : ''}`} 
                    fill="none" 
                    stroke="currentColor" 
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                  
                  <svg className={`w-5 h-5 text-${group.color}-400`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                  </svg>
                  
                  <span className={`flex-1 text-left font-semibold text-${group.color}-400 text-sm`}>
                    {group.namespace}
                  </span>
                  
                  <span className={`text-xs font-bold text-${group.color}-400 px-2 py-1 rounded border border-${group.color}-500/30`}>
                    {group.documents.length}
                  </span>
                </button>

                {/* Documents List */}
                {isExpanded && (
                  <div className="ml-6 space-y-1">
                    {group.documents.map((doc) => {
                      const fileName = doc.docPath.split('/').pop() || doc.docPath;
                      const isSelected = selectedDocId === doc.docId;
                      
                      return (
                        <button
                          key={doc.docId}
                          onClick={() => onDocumentClick?.(doc.docId)}
                          className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg transition-all text-left ${
                            isSelected
                              ? `border border-${group.color}-500/50`
                              : 'border border-transparent hover:border-slate-700'
                          }`}
                        >
                          <svg className={`w-4 h-4 shrink-0 ${isSelected ? `text-${group.color}-400` : 'text-slate-500'}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                          </svg>
                          
                          <div className="flex-1 min-w-0">
                            <div className={`text-sm font-medium truncate ${isSelected ? 'text-white' : 'text-slate-300'}`}>
                              {fileName}
                            </div>
                            {doc.agentName && (
                              <div className="text-xs text-slate-500 mt-0.5 truncate">
                                Agent: {doc.agentName}
                              </div>
                            )}
                          </div>
                          
                          <div className="text-xs text-slate-500 shrink-0">
                            {doc.contentTokens.toLocaleString()} tokens
                          </div>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
