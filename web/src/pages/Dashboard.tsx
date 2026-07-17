import { Link } from 'react-router-dom';
import {
  Activity,
  Key,
  Layers,
  Route,
  Server,
  Shield,
  ShieldCheck,
  Users,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import {
  useApiKeys,
  useConfig,
  useHealth,
  usePolicies,
  useReady,
  useRoutes,
} from '@/hooks/useAdmin';

function StatusBadge({
  loading,
  ok,
  label,
}: {
  loading: boolean;
  ok: boolean;
  label: string;
}) {
  if (loading) return <Badge variant="outline">Checking…</Badge>;
  return ok ? (
    <Badge variant="default" className="bg-green-600 hover:bg-green-700">
      {label} OK
    </Badge>
  ) : (
    <Badge variant="destructive">{label} Unhealthy</Badge>
  );
}

function StatCard({
  title,
  value,
  icon: Icon,
  href,
}: {
  title: string;
  value: number | string;
  icon: React.ElementType;
  href: string;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
        <Icon className="h-4 w-4 text-muted-foreground" />
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        <Button variant="link" size="sm" className="px-0" asChild>
          <Link to={href}>Manage</Link>
        </Button>
      </CardContent>
    </Card>
  );
}

export default function Dashboard() {
  const routes = useRoutes();
  const policies = usePolicies();
  const keys = useApiKeys();
  const config = useConfig();
  const health = useHealth();
  const ready = useReady();

  const routeCount = routes.data?.routes.length ?? '—';
  const policyCount = policies.data?.policies.length ?? '—';
  const keyCount = keys.data?.api_keys.length ?? '—';
  const siteCount = config.data?.sites.length ?? '—';
  const authCount =
    config.data?.auth_providers
      ? Object.keys(config.data.auth_providers).length
      : '—';

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">
          Overview of your Flint Gate deployment.
        </p>
      </div>

      <div className="flex flex-wrap gap-3">
        <StatusBadge loading={health.isLoading} ok={health.data?.status === 'ok'} label="Health" />
        <StatusBadge loading={ready.isLoading} ok={ready.data?.status === 'ready'} label="Ready" />
        <Badge variant="outline">Source: {routes.data?.source ?? '—'}</Badge>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <StatCard title="Routes" value={routeCount} icon={Route} href="/routes" />
        <StatCard title="Authz Policies" value={policyCount} icon={ShieldCheck} href="/policies" />
        <StatCard title="API Keys" value={keyCount} icon={Key} href="/api-keys" />
        <StatCard title="Sites" value={siteCount} icon={Server} href="/auth" />
        <StatCard title="Auth Providers" value={authCount} icon={Users} href="/auth" />
        <StatCard title="Hook Types" value="5" icon={Layers} href="/hooks" />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Activity className="h-4 w-4" />
              Quick Actions
            </CardTitle>
            <CardDescription>Common admin tasks</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-2">
            <Button size="sm" asChild>
              <Link to="/routes">Add Route</Link>
            </Button>
            <Button size="sm" variant="secondary" asChild>
              <Link to="/policies">Add Policy</Link>
            </Button>
            <Button size="sm" variant="secondary" asChild>
              <Link to="/api-keys">Create API Key</Link>
            </Button>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Shield className="h-4 w-4" />
              Security Notes
            </CardTitle>
            <CardDescription>Keep the admin API private</CardDescription>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            The admin API is unauthenticated and binds to loopback by default. Do not expose port 4457 to the public internet.
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
