'use client';

import { useState } from 'react';
import DocumentTree from '@/components/DocumentTree';
import DocumentViewer from '@/components/DocumentViewer';

export default function VisualizerPage() {
  const [selectedDocId, setSelectedDocId] = useState<string | null>(null);

  function handleDocumentClick(docId: string) {
    setSelectedDocId(docId);
  }

  return (
    <div className="min-h-screen bg-slate-950 p-8">
      <div className="max-w-[1800px] mx-auto">
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-5xl font-bold text-white tracking-tight mb-2">RAG Visualizer</h1>
          <p className="text-slate-400 text-sm">Browse documents by namespace and explore relationships</p>
        </div>

        {/* Main Layout */}
        <div className="grid grid-cols-1 xl:grid-cols-5 gap-6">
          {/* Document Tree - 2 columns */}
          <div className="xl:col-span-2 h-full bg-transparent">
            <DocumentTree 
              onDocumentClick={handleDocumentClick}
              selectedDocId={selectedDocId}
            />
          </div>

          {/* Document Viewer - 3 columns */}
          <div className="xl:col-span-3 h-full bg-transparent">
            <DocumentViewer documentId={selectedDocId} />
          </div>
        </div>
      </div>
    </div>
  );
}
