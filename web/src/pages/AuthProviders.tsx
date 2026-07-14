import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useConfig } from '@/hooks/useAdmin';
import { ReadOnlyBanner } from '@/components/ReadOnlyBanner';

export default function AuthProviders() {
  const { data, isLoading, error } = useConfig();

  if (isLoading) return <p className="text-muted-foreground">Loading auth providers…</p>;
  if (error) return <p className="text-destructive">Failed to load providers: {error.message}</p>;

  const entries = Object.entries(data?.auth_providers ?? {});

  return (
    <div className="space-y-6">
      <ReadOnlyBanner />
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Auth Providers</h1>
        <p className="text-muted-foreground">Configured identity providers from the loaded config.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Providers</CardTitle>
          <CardDescription>{entries.length} provider(s) configured</CardDescription>
        </CardHeader>
        <CardContent>
          {entries.length === 0 ? (
            <p className="text-sm text-muted-foreground">No auth providers configured.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ID</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Details</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {entries.map(([id, cfg]) => (
                  <TableRow key={id}>
                    <TableCell className="font-medium">{id}</TableCell>
                    <TableCell>
                      <Badge variant="secondary">{cfg.type}</Badge>
                    </TableCell>
                    <TableCell>
                      <Details config={cfg} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Details({ config }: { config: { type: string } & Record<string, unknown> }) {
  const { type, ...rest } = config;
  if (type === 'kratos') {
    return <span className="text-sm text-muted-foreground">{String(rest.base_url ?? '—')}</span>;
  }
  if (type === 'jwt' || type === 'mcp') {
    return <span className="text-sm text-muted-foreground">{String(rest.jwks_url ?? '—')}</span>;
  }
  if (type === 'api_key') {
    return (
      <span className="text-sm text-muted-foreground">
        Header: {String(rest.header ?? 'X-API-Key')}
      </span>
    );
  }
  if (type === 'anonymous') {
    return (
      <span className="text-sm text-muted-foreground">
        Default subject: {String(rest.default_subject ?? 'anonymous')}
      </span>
    );
  }
  return <span className="text-sm text-muted-foreground">—</span>;
}
