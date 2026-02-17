import { NextResponse } from 'next/server';
import { getIndexStats, getNamespaceDistribution } from '@/lib/queries';
import { isAuthenticated } from '@/lib/auth';

export async function GET() {
  if (!(await isAuthenticated())) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }
  
  try {
    const indexStats = getIndexStats();
    const namespaces = getNamespaceDistribution();
    return NextResponse.json({ indexStats, namespaces });
  } catch (error) {
    return NextResponse.json({ error: String(error) }, { status: 500 });
  }
}
