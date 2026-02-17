import { NextResponse } from 'next/server';
import { getRecentQueries } from '@/lib/queries';
import { isAuthenticated } from '@/lib/auth';

export async function GET(request: Request) {
  if (!(await isAuthenticated())) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }
  
  const { searchParams } = new URL(request.url);
  const limit = parseInt(searchParams.get('limit') || '10');
  
  try {
    const logs = getRecentQueries(limit);
    return NextResponse.json({ logs });
  } catch (error) {
    return NextResponse.json({ error: String(error) }, { status: 500 });
  }
}
