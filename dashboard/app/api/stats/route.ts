import { NextResponse } from 'next/server';
import { getIndexStats, getNamespaceDistribution, getPageIndexStats } from '@/lib/queries';
import { isAuthenticated } from '@/lib/auth';

export async function GET() {
  if (!(await isAuthenticated())) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }
  
  try {
    const indexStats = getIndexStats();
    const namespaces = getNamespaceDistribution();
    const pageIndexStats = getPageIndexStats();
    
    return NextResponse.json({ indexStats, namespaces, pageIndex: pageIndexStats });
  } catch (error) {
    return NextResponse.json({ error: String(error) }, { status: 500 });
  }
}
