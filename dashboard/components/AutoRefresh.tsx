'use client';

import { useEffect } from 'react';
import { useRouter, usePathname } from 'next/navigation';

export function AutoRefresh() {
  const router = useRouter();
  const pathname = usePathname();

  useEffect(() => {
    // Only auto-refresh on dashboard page, not login
    if (pathname === '/') {
      const interval = setInterval(() => {
        router.refresh();
      }, 30000); // 30 seconds for stats

      return () => clearInterval(interval);
    }
  }, [pathname, router]);

  return null;
}
