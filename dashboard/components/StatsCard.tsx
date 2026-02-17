'use client';

import { useEffect, useRef } from 'react';
import { animate } from 'animejs';

interface Props {
  title: string;
  value: number | string;
  status: string;
  color: 'emerald' | 'sky' | 'amber' | 'red';
}

const statusColorMap = {
  emerald: {
    badge: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
    border: 'border-emerald-500/30',
  },
  sky: {
    badge: 'bg-sky-500/10 text-sky-400 border-sky-500/20',
    border: 'border-sky-500/30',
  },
  amber: {
    badge: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
    border: 'border-amber-500/30',
  },
  red: {
    badge: 'bg-red-500/10 text-red-400 border-red-500/20',
    border: 'border-red-500/30',
  },
};

export default function StatsCard({ title, value, status, color }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (cardRef.current) {
      animate(cardRef.current, {
        scale: [0.95, 1],
        opacity: [0, 1],
        duration: 500,
        ease: 'easeOutQuad',
      });
    }
  }, []);

  return (
    <div
      ref={cardRef}
      className={`bg-slate-800/50 border ${statusColorMap[color].border} rounded-xl shadow-lg p-6 text-white transition-all hover:shadow-2xl hover:shadow-${color}-500/10 hover:-translate-y-1 relative overflow-hidden group`}
    >
      {/* Status badge in top-right */}
      <div className="absolute top-4 right-4">
        <span className={`px-3 py-1 rounded-full text-xs font-semibold border ${statusColorMap[color].badge}`}>
          {status}
        </span>
      </div>

      {/* Title */}
      <div className="mb-4">
        <h3 className="text-xs font-bold text-slate-400 uppercase tracking-wider">
          {title}
        </h3>
      </div>

      {/* Value */}
      <div className="text-5xl font-extrabold tracking-tight mt-2">
        {value}
      </div>

      {/* Subtle hover accent line */}
      <div className={`absolute bottom-0 left-0 right-0 h-1 bg-gradient-to-r from-transparent via-${color}-500 to-transparent opacity-0 group-hover:opacity-100 transition-opacity`} />
    </div>
  );
}
