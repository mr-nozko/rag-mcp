/**
 * Time range utilities for filtering dashboard data
 */

export type TimePeriod = 'today' | '1d' | '1w' | '1m' | '1y' | '5y' | 'all';

/**
 * Convert time period to SQL datetime filter
 * Returns SQL expression for filtering timestamps
 */
export function getTimeRangeFilter(period: TimePeriod): string | null {
  switch (period) {
    case 'today':
      return "datetime('now', 'start of day')";
    case '1d':
      return "datetime('now', '-1 day')";
    case '1w':
      return "datetime('now', '-7 days')";
    case '1m':
      return "datetime('now', '-1 month')";
    case '1y':
      return "datetime('now', '-1 year')";
    case '5y':
      return "datetime('now', '-5 years')";
    case 'all':
      return null; // No filter
    default:
      return null;
  }
}

/**
 * Get display label for time period
 */
export function getTimePeriodLabel(period: TimePeriod): string {
  const labels: Record<TimePeriod, string> = {
    today: 'Today',
    '1d': '1D',
    '1w': '1W',
    '1m': '1M',
    '1y': '1Y',
    '5y': '5Y',
    all: 'All',
  };
  return labels[period] || 'All';
}

/**
 * Validate time period string
 */
export function isValidTimePeriod(value: string): value is TimePeriod {
  return ['today', '1d', '1w', '1m', '1y', '5y', 'all'].includes(value);
}
