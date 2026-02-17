import { NextResponse } from 'next/server';
import { requireAuth } from '@/lib/auth';
import { 
  getEmbeddingStats, 
  getMissingEmbeddings, 
  getMissingEmbeddingsCount,
  getChunksWithEmbeddings 
} from '@/lib/queries';

export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  try {
    await requireAuth();
    
    const { searchParams } = new URL(request.url);
    const action = searchParams.get('action') || 'stats';
    
    if (action === 'stats') {
      const stats = getEmbeddingStats();
      return NextResponse.json(stats);
    }
    
    if (action === 'missing') {
      const limit = parseInt(searchParams.get('limit') || '20');
      const offset = parseInt(searchParams.get('offset') || '0');
      const missing = getMissingEmbeddings(limit, offset);
      const total = getMissingEmbeddingsCount();
      
      return NextResponse.json({
        chunks: missing,
        total,
        limit,
        offset,
      });
    }
    
    if (action === 'list') {
      const limit = parseInt(searchParams.get('limit') || '100');
      const chunks = getChunksWithEmbeddings(limit);
      return NextResponse.json({ chunks });
    }
    
    return NextResponse.json(
      { error: 'Invalid action' },
      { status: 400 }
    );
  } catch (error) {
    console.error('Error fetching embeddings:', error);
    return NextResponse.json(
      { error: 'Failed to fetch embeddings' },
      { status: 500 }
    );
  }
}
