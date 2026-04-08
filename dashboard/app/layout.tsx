import type { Metadata } from 'next';
import './globals.css';
import Sidebar from '@/components/Sidebar';
import { SidebarProvider } from '@/components/SidebarContext';
import DynamicMain from '@/components/DynamicMain';

export const metadata: Metadata = {
  title: 'RAGMcp Dashboard',
  description: 'Visual management dashboard for RAGMcp server',
};

// Force dynamic rendering globally — this dashboard is auth-protected and reads
// live SQLite data, so static prerendering makes no sense. This also avoids
// the styled-jsx readlink EISDIR bug in Next.js 15 static gen on Windows.
export const dynamic = 'force-dynamic';


export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="flex min-h-screen">
        <SidebarProvider>
          <Sidebar />
          <DynamicMain>
            {children}
          </DynamicMain>
        </SidebarProvider>
      </body>
    </html>
  );
}
