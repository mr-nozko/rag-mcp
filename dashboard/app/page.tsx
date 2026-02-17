import { requireAuth } from '@/lib/auth';
import { getIndexStats, getNamespaceDistribution, getRecentQueries } from '@/lib/queries';
import StatsCard from '@/components/StatsCard';
import NamespaceTable from '@/components/NamespaceTable';
import LogsTable from '@/components/LogsTable';
import RefreshButton from '@/components/RefreshButton';
import TimeFilter from '@/components/TimeFilter';
import { isValidTimePeriod, type TimePeriod } from '@/lib/time-utils';

export const dynamic = 'force-dynamic';

interface PageProps {
  searchParams: Promise<{ period?: string }>;
}

export default async function DashboardPage({ searchParams }: PageProps) {
  await requireAuth();
  
  // Parse time period from URL params, default to 'all'
  const params = await searchParams;
  const period = (params.period && isValidTimePeriod(params.period) 
    ? params.period 
    : 'all') as TimePeriod;
  
  const indexStats = getIndexStats(period);
  const namespaces = getNamespaceDistribution(period);
  const logs = getRecentQueries(10, period);

  return (
    <div className="p-8">
      <div className="max-w-[1600px] mx-auto">
        {/* Header */}
        <div className="mb-10">
          <div className="flex justify-between items-start mb-6">
            <div>
              <h1 className="text-5xl font-bold text-white tracking-tight mb-2">RAGMcp Dashboard</h1>
              <p className="text-slate-400 text-sm">Real-time analytics and monitoring</p>
            </div>
            <div className="flex items-center gap-4">
              <RefreshButton />
            </div>
          </div>
          
          {/* Time Period Filters */}
          <TimeFilter />
        </div>

        {/* Stats Cards Grid */}
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-8 mb-10">
          <StatsCard title="Documents" value={indexStats.docCount} status="Stable" color="emerald" />
          <StatsCard title="Chunks" value={indexStats.chunkCount} status="Active" color="sky" />
          <StatsCard title="Embeddings" value={indexStats.embeddedCount} status="Active" color="emerald" />
          <StatsCard 
            title="Coverage" 
            value={indexStats.chunkCount > 0 ? `${Math.round((indexStats.embeddedCount / indexStats.chunkCount) * 100)}%` : '0%'} 
            status="High"
            color="amber" 
          />
        </div>

        {/* Main Content Grid */}
        <div className="grid grid-cols-1 xl:grid-cols-5 gap-8">
          {/* Namespace Distribution - 2 columns on xl */}
          <div className="xl:col-span-2 bg-slate-900/50 border border-slate-800 rounded-xl shadow-2xl p-8">
            <h2 className="text-2xl font-bold text-white mb-6 tracking-tight">Namespace Distribution</h2>
            <NamespaceTable namespaces={namespaces} />
          </div>

          {/* Recent Queries - 3 columns on xl */}
          <div className="xl:col-span-3 bg-slate-900/50 border border-slate-800 rounded-xl shadow-2xl p-8">
            <h2 className="text-2xl font-bold text-white mb-6 tracking-tight">Recent Queries</h2>
            <LogsTable logs={logs} />
          </div>
        </div>
      </div>
    </div>
  );
}
