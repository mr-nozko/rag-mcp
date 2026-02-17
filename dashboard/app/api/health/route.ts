import { NextResponse } from 'next/server';
import { getHealthMetrics } from '@/lib/queries';
import { isAuthenticated } from '@/lib/auth';

export async function GET() {
  if (!(await isAuthenticated())) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }
  
  try {
    const health = getHealthMetrics();
    return NextResponse.json(health);
  } catch (error) {
    return NextResponse.json({ error: String(error) }, { status: 500 });
  }
}
