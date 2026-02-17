import { NextResponse } from 'next/server';
import { requireAuth } from '@/lib/auth';
import { getDocumentGraph } from '@/lib/queries';

export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  try {
    await requireAuth();
    
    const graphData = getDocumentGraph();
    
    return NextResponse.json(graphData);
  } catch (error) {
    console.error('Error fetching graph data:', error);
    return NextResponse.json(
      { error: 'Failed to fetch graph data' },
      { status: 500 }
    );
  }
}
