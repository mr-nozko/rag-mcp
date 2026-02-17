'use client';

import { useSearchParams, useRouter, usePathname } from 'next/navigation';
import type { TimePeriod } from '@/lib/time-utils';
import { getTimePeriodLabel } from '@/lib/time-utils';

const TIME_PERIODS: TimePeriod[] = ['today', '1d', '1w', '1m', '1y', '5y', 'all'];

export default function TimeFilter() {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const currentPeriod = (searchParams.get('period') || 'all') as TimePeriod;

  function handlePeriodChange(period: TimePeriod) {
    const params = new URLSearchParams(searchParams);
    params.set('period', period);
    router.push(`${pathname}?${params.toString()}`);
  }

  return (
    <div className="flex items-center gap-3">
      <span className="text-slate-400 text-sm font-medium">Time Period:</span>
      <div className="flex gap-2">
        {TIME_PERIODS.map((period) => (
          <button
            key={period}
            onClick={() => handlePeriodChange(period)}
            className={`
              px-4 py-1.5 rounded-full text-sm font-medium transition-all
              ${
                currentPeriod === period
                  ? 'bg-emerald-600 text-white'
                  : 'bg-slate-800 text-slate-300 border border-slate-700 hover:bg-slate-700'
              }
            `}
          >
            {getTimePeriodLabel(period)}
          </button>
        ))}
      </div>
    </div>
  );
}
