import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useRoutes } from '@/hooks/useAdmin';
import type { DbRoute, PreRequestHook, PostResponseHook } from '@/api/types';

export default function Hooks() {
  const { data, isLoading, error } = useRoutes();

  if (isLoading) return <p className="text-muted-foreground">Loading hooks…</p>;
  if (error) return <p className="text-destructive">Failed to load hooks: {error.message}</p>;

  const rows: { route: DbRoute; phase: 'pre_request' | 'post_response'; hook: PreRequestHook | PostResponseHook }[] = [];
  for (const route of data?.routes ?? []) {
    for (const hook of route.config.hooks?.pre_request ?? []) {
      rows.push({ route, phase: 'pre_request', hook });
    }
    for (const hook of route.config.hooks?.post_response ?? []) {
      rows.push({ route, phase: 'post_response', hook });
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Hooks</h1>
        <p className="text-muted-foreground">Pre-request and post-response hooks configured on routes.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Hook Instances</CardTitle>
          <CardDescription>{rows.length} hook(s) across {data?.routes.length ?? 0} route(s)</CardDescription>
        </CardHeader>
        <CardContent>
          {rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">No hooks configured.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Route</TableHead>
                  <TableHead>Phase</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Details</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row, idx) => (
                  <TableRow key={`${row.route.id}-${row.phase}-${idx}`}>
                    <TableCell className="font-medium">{row.route.id}</TableCell>
                    <TableCell>
                      <Badge variant={row.phase === 'pre_request' ? 'default' : 'secondary'}>
                        {row.phase === 'pre_request' ? 'pre-request' : 'post-response'}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">{row.hook.type}</Badge>
                    </TableCell>
                    <TableCell className="max-w-md truncate text-sm text-muted-foreground">
                      {row.hook.config ? JSON.stringify(row.hook.config) : '—'}
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
