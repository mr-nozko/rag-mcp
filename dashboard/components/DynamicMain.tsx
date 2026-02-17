'use client';

import { useSidebar } from './SidebarContext';
import { ReactNode } from 'react';

export default function DynamicMain({ children }: { children: ReactNode }) {
  const { collapsed } = useSidebar();

  return (
    <main
      className={`flex-1 min-h-screen bg-slate-950 transition-all duration-300 ${
        collapsed ? 'ml-20' : 'ml-64'
      }`}
    >
      {children}
    </main>
  );
}
