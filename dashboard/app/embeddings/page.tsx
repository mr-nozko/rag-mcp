'use client';

import EmbeddingStats from '@/components/EmbeddingStats';
import MissingEmbeddingsList from '@/components/MissingEmbeddingsList';
import EmbeddingVisualizer from '@/components/EmbeddingVisualizer';

export default function EmbeddingsPage() {
  return (
    <div className="p-8">
      <div className="max-w-[1800px] mx-auto space-y-8">
        {/* Header */}
        <div>
          <h1 className="text-5xl font-bold text-white tracking-tight mb-2">Embed Checker</h1>
          <p className="text-slate-400 text-sm">Monitor embedding coverage and visualize chunks by document</p>
        </div>

        {/* Stats Overview */}
        <EmbeddingStats />

        {/* Missing Embeddings */}
        <MissingEmbeddingsList />

        {/* Embedding Visualizer by Document */}
        <EmbeddingVisualizer />
      </div>
    </div>
  );
}
