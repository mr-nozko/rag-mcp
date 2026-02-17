import { NextResponse } from 'next/server';
import { requireAuth } from '@/lib/auth';
import { getChunkWithEmbedding } from '@/lib/queries';
import { decodeEmbedding, cosineSimilarity } from '@/lib/vector-utils';

export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  try {
    await requireAuth();
    
    const { searchParams } = new URL(request.url);
    const chunkId1 = searchParams.get('chunk1');
    const chunkId2 = searchParams.get('chunk2');
    
    if (!chunkId1 || !chunkId2) {
      return NextResponse.json(
        { error: 'Both chunk1 and chunk2 parameters are required' },
        { status: 400 }
      );
    }
    
    const chunk1 = getChunkWithEmbedding(chunkId1);
    const chunk2 = getChunkWithEmbedding(chunkId2);
    
    if (!chunk1 || !chunk2) {
      return NextResponse.json(
        { error: 'One or both chunks not found or missing embeddings' },
        { status: 404 }
      );
    }
    
    const embedding1 = decodeEmbedding(chunk1.embedding);
    const embedding2 = decodeEmbedding(chunk2.embedding);
    
    if (!embedding1 || !embedding2) {
      return NextResponse.json(
        { error: 'Failed to decode embeddings' },
        { status: 500 }
      );
    }
    
    const similarity = cosineSimilarity(embedding1, embedding2);
    
    return NextResponse.json({
      chunk1: {
        chunkId: chunk1.chunkId,
        chunkText: chunk1.chunkText,
        chunkTokens: chunk1.chunkTokens,
        sectionHeader: chunk1.sectionHeader,
        embeddingDimensions: embedding1.length,
        embeddingPreview: embedding1.slice(0, 20),
      },
      chunk2: {
        chunkId: chunk2.chunkId,
        chunkText: chunk2.chunkText,
        chunkTokens: chunk2.chunkTokens,
        sectionHeader: chunk2.sectionHeader,
        embeddingDimensions: embedding2.length,
        embeddingPreview: embedding2.slice(0, 20),
      },
      similarity,
    });
  } catch (error) {
    console.error('Error comparing embeddings:', error);
    return NextResponse.json(
      { error: 'Failed to compare embeddings' },
      { status: 500 }
    );
  }
}
