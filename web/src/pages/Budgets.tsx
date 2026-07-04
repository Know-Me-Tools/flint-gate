import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useRoutes } from '@/hooks/useAdmin';
import type { DbRoute } from '@/api/types';

interface BudgetHook {
  route: DbRoute;
  limit: number;
  user_id_expr: string;
  window: string;
  scope: string;
}

export default function Budgets() {
  const { data, isLoading, error } = useRoutes();

  if (isLoading) return <p className="text-muted-foreground">Loading budgets…</p>;
  if (error) return <p className="text-destructive">Failed to load budgets: {error.message}</p>;

  const budgets: BudgetHook[] = [];
  for (const route of data?.routes ?? []) {
    for (const hook of route.config.hooks?.pre_request ?? []) {
      if (hook.type === 'max_token_budget' && typeof hook.config?.limit === 'number') {
        budgets.push({
          route,
          limit: hook.config.limit as number,
          user_id_expr: String(hook.config.user_id_expr ?? 'identity.id'),
          window: String(hook.config.window ?? 'lifetime'),
          scope: String(hook.config.scope ?? 'user'),
        });
      }
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Token Budgets</h1>
        <p className="text-muted-foreground">Max-token-budget hooks across routes.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Budget Rules</CardTitle>
          <CardDescription>{budgets.length} budget rule(s) configured</CardDescription>
        </CardHeader>
        <CardContent>
          {budgets.length === 0 ? (
            <p className="text-sm text-muted-foreground">No token budgets configured.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Route</TableHead>
                  <TableHead>Limit</TableHead>
                  <TableHead>User ID Expr</TableHead>
                  <TableHead>Window</TableHead>
                  <TableHead>Scope</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {budgets.map((b, idx) => (
                  <TableRow key={`${b.route.id}-${idx}`}>
                    <TableCell className="font-medium">{b.route.id}</TableCell>
                    <TableCell>{b.limit.toLocaleString()}</TableCell>
                    <TableCell className="font-mono text-xs">{b.user_id_expr}</TableCell>
                    <TableCell><Badge variant="outline">{b.window}</Badge></TableCell>
                    <TableCell><Badge variant="secondary">{b.scope}</Badge></TableCell>
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
