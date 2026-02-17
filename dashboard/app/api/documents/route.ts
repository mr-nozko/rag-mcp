import { NextResponse } from 'next/server';
import { requireAuth } from '@/lib/auth';
import { getAllDocuments, getDocumentsByNamespace } from '@/lib/queries';

export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  try {
    await requireAuth();
    
    const { searchParams } = new URL(request.url);
    const namespace = searchParams.get('namespace');
    const limit = parseInt(searchParams.get('limit') || '100');
    
    const documents = namespace 
      ? getDocumentsByNamespace(namespace)
      : getAllDocuments(limit);
    
    return NextResponse.json({ documents });
  } catch (error) {
    console.error('Error fetching documents:', error);
    return NextResponse.json(
      { error: 'Failed to fetch documents' },
      { status: 500 }
    );
  }
}
