import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Link, Route, Routes } from 'react-router-dom';
import { lazy, Suspense, type ReactNode } from 'react';
import { Button } from '@/components/ui/button';
import { ToastProvider } from '@/components/ui/toast';
import Dashboard from '@/pages/Dashboard';
import RoutesPage from '@/pages/Routes';
import AuthProviders from '@/pages/AuthProviders';
import Hooks from '@/pages/Hooks';
import Policies from '@/pages/Policies';
import Budgets from '@/pages/Budgets';
import ApiKeys from '@/pages/ApiKeys';

// Code-split the analytics surface: Recharts is large and only this page needs
// it, so it must not weigh down the CRUD bundle (app-page JS budget ~300 KB).
const Analytics = lazy(() => import('@/pages/Analytics'));

const queryClient = new QueryClient();

function PageFallback() {
  return <p className="text-muted-foreground">Loading…</p>;
}

function Layout({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen flex flex-col">
      <header className="border-b px-4 py-3 flex items-center gap-4">
        <div className="font-semibold">Flint Gate Admin</div>
        <nav className="flex gap-2">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/">Dashboard</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/analytics">Analytics</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/routes">Routes</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/auth">Auth</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/hooks">Hooks</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/policies">Policies</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/budgets">Budgets</Link>
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/api-keys">API Keys</Link>
          </Button>
        </nav>
      </header>
      <main className="flex-1 p-6">{children}</main>
    </div>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <BrowserRouter>
          <Layout>
            <Suspense fallback={<PageFallback />}>
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/analytics" element={<Analytics />} />
              <Route path="/routes" element={<RoutesPage />} />
              <Route path="/auth" element={<AuthProviders />} />
              <Route path="/hooks" element={<Hooks />} />
              <Route path="/policies" element={<Policies />} />
              <Route path="/budgets" element={<Budgets />} />
              <Route path="/api-keys" element={<ApiKeys />} />
            </Routes>
            </Suspense>
          </Layout>
        </BrowserRouter>
      </ToastProvider>
    </QueryClientProvider>
  );
}
