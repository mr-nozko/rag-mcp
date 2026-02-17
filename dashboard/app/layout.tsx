import type { Metadata } from 'next';
import './globals.css';
import Sidebar from '@/components/Sidebar';
import { SidebarProvider } from '@/components/SidebarContext';
import DynamicMain from '@/components/DynamicMain';

export const metadata: Metadata = {
  title: 'RAGMcp Dashboard',
  description: 'Visual management dashboard for RAGMcp server',
};

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
