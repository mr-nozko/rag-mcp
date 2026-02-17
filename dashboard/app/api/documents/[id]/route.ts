import { NextResponse } from 'next/server';
import { requireAuth } from '@/lib/auth';
import { getDocumentDetails, getRelatedDocuments } from '@/lib/queries';

export const dynamic = 'force-dynamic';

/**
 * GET /api/documents/[id]
 *
 * Returns full details for a single document, including:
 *   - document metadata and content
 *   - all associated chunks (with embedding status)
 *   - knowledge-graph relations where this doc is source or target
 *   - related documents (via entity graph)
 *
 * Used by DocumentViewer (visualizer page) and EmbeddingVisualizer (embeddings page).
 */
export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    await requireAuth();

    // Next.js 15 App Router: params is a Promise — must be awaited
    const { id } = await params;

    // The component uses encodeURIComponent; Next.js decodes the segment
    // automatically, but decodeURIComponent handles any double-encoding edge cases.
    const docId = decodeURIComponent(id);

    const document = getDocumentDetails(docId);

    if (!document) {
      return NextResponse.json(
        { error: 'Document not found' },
        { status: 404 }
      );
    }

    // Fetch related documents via shared entity graph edges
    const relatedDocuments = getRelatedDocuments(docId);

    return NextResponse.json({ document, relatedDocuments });
  } catch (error) {
    console.error('Error fetching document details:', error);
    return NextResponse.json(
      { error: 'Failed to fetch document details' },
      { status: 500 }
    );
  }
}
