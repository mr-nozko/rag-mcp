'use client';

import { useState } from 'react';
import { useRouter } from 'next/navigation';

export default function LoginPage() {
  const router = useRouter();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError('');

    const res = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password }),
    });

    if (res.ok) {
      router.push('/');
      router.refresh();
    } else {
      setError('Invalid credentials');
      setLoading(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-slate-950 relative overflow-hidden">
      {/* Subtle background accent */}
      <div className="absolute inset-0 bg-gradient-radial from-slate-800/20 via-slate-950 to-slate-950" />
      
      <div className="relative glass rounded-2xl p-12 w-[28rem] shadow-2xl animate-fadeIn">
        <div className="mb-10 text-center">
          <h1 className="text-4xl font-bold text-white tracking-tight mb-2">RAGMcp</h1>
          <p className="text-slate-400 text-sm">Management Dashboard</p>
        </div>
        
        <form onSubmit={handleSubmit} className="space-y-6">
          <div>
            <label className="block text-xs font-semibold text-slate-300 uppercase tracking-wider mb-3">
              Username
            </label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-slate-900/50 text-white border border-slate-700 focus:border-emerald-500 focus:outline-none transition-colors placeholder:text-slate-500"
              placeholder="Enter username"
              required
            />
          </div>
          
          <div>
            <label className="block text-xs font-semibold text-slate-300 uppercase tracking-wider mb-3">
              Password
            </label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-slate-900/50 text-white border border-slate-700 focus:border-emerald-500 focus:outline-none transition-colors placeholder:text-slate-500"
              placeholder="Enter password"
              required
            />
          </div>
          
          {error && (
            <div className="bg-red-500/10 border border-red-500/30 rounded-lg px-4 py-3">
              <p className="text-red-400 text-sm">{error}</p>
            </div>
          )}
          
          <button
            type="submit"
            disabled={loading}
            className="w-full py-3 px-6 bg-gradient-to-r from-emerald-600 to-emerald-500 hover:from-emerald-500 hover:to-emerald-400 text-white font-semibold rounded-lg transition-all transform hover:-translate-y-0.5 hover:shadow-xl hover:shadow-emerald-500/20 disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none"
          >
            {loading ? 'Signing in...' : 'Sign In'}
          </button>
        </form>
        
        <div className="mt-8 pt-6 border-t border-slate-700/50 text-center">
          <p className="text-slate-500 text-xs">Secure admin access only</p>
        </div>
      </div>
    </div>
  );
}
