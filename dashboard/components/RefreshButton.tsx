'use client';

import { useRouter } from 'next/navigation';
import { useState } from 'react';
import { animate } from 'animejs';

export default function RefreshButton() {
  const router = useRouter();
  const [loading, setLoading] = useState(false);

  async function handleRefresh() {
    setLoading(true);
    
    // Trigger anime.js rotation animation
    animate('.refresh-icon', {
      rotate: '1turn',
      duration: 600,
      ease: 'easeInOutQuad',
    });

    router.refresh();
    
    setTimeout(() => setLoading(false), 600);
  }

  return (
    <button
      onClick={handleRefresh}
      disabled={loading}
      className="px-5 py-2.5 bg-transparent hover:bg-slate-700/50 text-white border border-slate-600 hover:border-slate-500 rounded-lg transition-all flex items-center gap-2.5 disabled:opacity-50 disabled:cursor-not-allowed font-medium text-sm"
    >
      <svg 
        className="refresh-icon w-4 h-4" 
        fill="none" 
        stroke="currentColor" 
        viewBox="0 0 24 24"
      >
        <path 
          strokeLinecap="round" 
          strokeLinejoin="round" 
          strokeWidth={2} 
          d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" 
        />
      </svg>
      Refresh
    </button>
  );
}
