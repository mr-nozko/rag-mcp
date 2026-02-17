'use client';

interface Props {
  status: 'excellent' | 'good' | 'degraded' | 'no_data';
  p50: number;
  p95: number;
}

const statusConfig = {
  excellent: {
    label: 'Active',
    dotColor: 'bg-emerald-500',
    bgColor: 'bg-emerald-500/10',
    textColor: 'text-emerald-400',
    borderColor: 'border-emerald-500/30',
  },
  good: {
    label: 'Good',
    dotColor: 'bg-sky-500',
    bgColor: 'bg-sky-500/10',
    textColor: 'text-sky-400',
    borderColor: 'border-sky-500/30',
  },
  degraded: {
    label: 'Degraded',
    dotColor: 'bg-red-500',
    bgColor: 'bg-red-500/10',
    textColor: 'text-red-400',
    borderColor: 'border-red-500/30',
  },
  no_data: {
    label: 'No Data',
    dotColor: 'bg-slate-500',
    bgColor: 'bg-slate-500/10',
    textColor: 'text-slate-400',
    borderColor: 'border-slate-500/30',
  },
};

export default function HealthIndicator({ status, p50, p95 }: Props) {
  const config = statusConfig[status];

  return (
    <div className={`flex items-center gap-4 ${config.bgColor} border ${config.borderColor} rounded-full px-5 py-2.5`}>
      <div className="flex items-center gap-2.5">
        <div className="relative">
          <div className={`w-2.5 h-2.5 rounded-full ${config.dotColor} animate-pulse-slow`} />
          <div className={`absolute inset-0 w-2.5 h-2.5 rounded-full ${config.dotColor} animate-ping opacity-75`} />
        </div>
        <span className={`text-sm font-bold ${config.textColor} uppercase tracking-wide`}>
          {config.label}
        </span>
      </div>
      {status !== 'no_data' && (
        <div className="flex items-center gap-3 text-xs font-medium text-slate-400 border-l border-slate-700/50 pl-4">
          <span>
            <span className="text-slate-500">P50:</span> <span className="text-slate-300 font-bold">{p50}ms</span>
          </span>
          <span>
            <span className="text-slate-500">P95:</span> <span className="text-slate-300 font-bold">{p95}ms</span>
          </span>
        </div>
      )}
    </div>
  );
}
