# RAGMcp Dashboard

Visual management dashboard for the RAGMcp server, built with Next.js 15.

## Features

- 🔐 Secure authentication (credentials set via `ADMIN_USERNAME` / `ADMIN_PASSWORD` env vars)
- 📊 Real-time stats with 30-second auto-refresh
- 📝 Query logs with 10-second updates
- 🎯 Health monitoring with P50/P95 latency metrics
- 🎨 Animated UI components with anime.js
- 🌙 Dark theme with gradient backgrounds
- 📱 Responsive design

## Quick Start

1. **Install dependencies:**
   ```bash
   npm install
   ```

2. **Start development server:**
   ```bash
   npm run dev
   ```

3. **Configure credentials** (copy `.env.example` to `.env.local`):
   ```bash
   cp .env.example .env.local
   # Edit .env.local: set ADMIN_USERNAME and ADMIN_PASSWORD
   ```

4. **Access dashboard:**
   - Open http://localhost:3001
   - Login with your configured `ADMIN_USERNAME` / `ADMIN_PASSWORD`

## Architecture

- **Port:** 3001 (separate from MCP server at 8081)
- **Database:** Read-only access to ../ragmcp.db via better-sqlite3
- **Authentication:** Session-based with httpOnly cookies
- **Auto-refresh:** 30s for stats, manual refresh button available

## Production Build

```bash
npm run build
npm start
```

## Tech Stack

- Next.js 15 (App Router, Server Components)
- React 19
- TypeScript
- Tailwind CSS 4.x
- anime.js 4.0+
- better-sqlite3 (read-only mode)

## Project Structure

```
dashboard/
├── app/              # Next.js App Router pages
│   ├── api/         # API routes (auth, stats, logs, health)
│   ├── login/       # Login page
│   └── page.tsx     # Main dashboard
├── components/      # React components
├── lib/            # Utilities (db, queries, auth)
└── public/         # Static assets
```

## Notes

- Database access is read-only to prevent conflicts with MCP server
- Auto-refresh can be disabled by removing AutoRefresh component
- Health status based on P95 latency: <1s (excellent), <2s (good), ≥2s (degraded)
