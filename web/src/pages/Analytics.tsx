import { useState } from 'react';
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { useAudit, useTokenAnalytics, useUsageSummary } from '@/hooks/useAdmin';
import type {
  AnalyticsInterval,
  AuthzDecision,
  RouteUsage,
  UsageTimeSeriesPoint,
  UserUsage,
} from '@/api/types';

/** Semantic chart palette — drawn from the design-system `--chart-*` tokens so
 *  analytics reads as one system with the rest of the admin UI. */
const CHART = {
  tokens: 'var(--color-chart-1)',
  requests: 'var(--color-chart-2)',
  bars: ['var(--color-chart-1)', 'var(--color-chart-2)', 'var(--color-chart-3)', 'var(--color-chart-4)', 'var(--color-chart-5)'],
} as const;

const numberFmt = new Intl.NumberFormat();

/** Recharts tooltip/axis values are `number | string | (number|string)[]`.
 *  Format numerics with grouping; pass anything else through as a string. */
function formatChartValue(value: number | string | (number | string)[]): string {
  return typeof value === 'number' ? numberFmt.format(value) : String(value);
}

/** Render a timestamp, guarding against a malformed (non-RFC3339) value. */
function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

function StatTile({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">{label}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold tabular-nums">{value}</div>
        {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
      </CardContent>
    </Card>
  );
}

/** Format an RFC3339 bucket to a compact axis label. */
function bucketLabel(iso: string, interval: AnalyticsInterval): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return interval === 'hour'
    ? d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric' })
    : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function TimeSeriesCard({
  series,
  interval,
}: {
  series: UsageTimeSeriesPoint[];
  interval: AnalyticsInterval;
}) {
  const data = series.map((p) => ({ ...p, label: bucketLabel(p.bucket, interval) }));
  return (
    <Card className="lg:col-span-2">
      <CardHeader>
        <CardTitle>Token throughput</CardTitle>
        <CardDescription>Tokens and requests per {interval}</CardDescription>
      </CardHeader>
      <CardContent>
        {data.length === 0 ? (
          <EmptyChart label="No usage events in this window." />
        ) : (
          <ResponsiveContainer width="100%" height={280}>
            <AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <defs>
                <linearGradient id="tokensFill" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={CHART.tokens} stopOpacity={0.35} />
                  <stop offset="100%" stopColor={CHART.tokens} stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" vertical={false} />
              <XAxis dataKey="label" tick={{ fontSize: 12 }} tickLine={false} axisLine={false} />
              <YAxis tick={{ fontSize: 12 }} tickLine={false} axisLine={false} width={56} />
              <Tooltip
                contentStyle={{
                  background: 'var(--color-popover, white)',
                  border: '1px solid var(--color-border, #e5e7eb)',
                  borderRadius: 8,
                  fontSize: 12,
                }}
                formatter={(value, name) => [formatChartValue(value), String(name)]}
              />
              <Area
                type="monotone"
                dataKey="tokens"
                name="Tokens"
                stroke={CHART.tokens}
                fill="url(#tokensFill)"
                strokeWidth={2}
              />
              <Area
                type="monotone"
                dataKey="requests"
                name="Requests"
                stroke={CHART.requests}
                fill="none"
                strokeWidth={1.5}
                strokeDasharray="4 3"
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </CardContent>
    </Card>
  );
}

function TopBarCard({
  title,
  description,
  rows,
}: {
  title: string;
  description: string;
  rows: { label: string; tokens: number }[];
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        {rows.length === 0 ? (
          <EmptyChart label="No data yet." />
        ) : (
          <ResponsiveContainer width="100%" height={240}>
            <BarChart data={rows} layout="vertical" margin={{ top: 4, right: 8, left: 8, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" horizontal={false} />
              <XAxis type="number" tick={{ fontSize: 12 }} tickLine={false} axisLine={false} />
              <YAxis
                type="category"
                dataKey="label"
                width={130}
                tick={{ fontSize: 12 }}
                tickLine={false}
                axisLine={false}
              />
              <Tooltip
                cursor={{ fill: 'var(--color-muted, rgba(0,0,0,0.04))' }}
                formatter={(value) => [formatChartValue(value), 'Tokens']}
              />
              <Bar dataKey="tokens" radius={[0, 4, 4, 0]}>
                {rows.map((row, i) => (
                  <Cell key={row.label} fill={CHART.bars[i % CHART.bars.length]} />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        )}
      </CardContent>
    </Card>
  );
}

function EmptyChart({ label }: { label: string }) {
  return (
    <div className="flex h-[240px] items-center justify-center rounded-md border border-dashed text-sm text-muted-foreground">
      {label}
    </div>
  );
}

const DECISION_VARIANT: Record<AuthzDecision, 'default' | 'secondary' | 'destructive' | 'outline'> = {
  allow: 'default',
  deny: 'destructive',
  step_up: 'secondary',
  approval: 'outline',
};

function AuditTable() {
  const { data, isLoading, error } = useAudit({ limit: 50 });
  const rows = data?.audit ?? [];
  return (
    <Card>
      <CardHeader>
        <CardTitle>Authorization audit trail</CardTitle>
        <CardDescription>Most recent allow / deny / step-up / approval decisions</CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <p className="text-sm text-muted-foreground">Loading audit trail…</p>
        ) : error ? (
          <p className="text-sm text-destructive">Failed to load audit trail: {error.message}</p>
        ) : rows.length === 0 ? (
          <EmptyChart label="No authorization decisions recorded yet." />
        ) : (
          <div
            className="max-h-[420px] overflow-auto"
            tabIndex={0}
            role="region"
            aria-label="Authorization audit trail"
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Time</TableHead>
                  <TableHead>Decision</TableHead>
                  <TableHead>Principal</TableHead>
                  <TableHead>Action</TableHead>
                  <TableHead>Resource</TableHead>
                  <TableHead>Reason</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="whitespace-nowrap text-xs tabular-nums text-muted-foreground">
                      {formatTimestamp(row.created_at)}
                    </TableCell>
                    <TableCell>
                      <Badge variant={DECISION_VARIANT[row.decision]}>{row.decision}</Badge>
                    </TableCell>
                    <TableCell className="text-sm">{row.principal ?? '—'}</TableCell>
                    <TableCell className="text-sm">{row.action ?? '—'}</TableCell>
                    <TableCell className="max-w-[220px] truncate text-sm" title={row.resource ?? ''}>
                      {row.resource ?? '—'}
                    </TableCell>
                    <TableCell className="max-w-[260px] truncate text-sm text-muted-foreground" title={row.reason ?? ''}>
                      {row.reason ?? '—'}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export default function Analytics() {
  const [interval, setInterval] = useState<AnalyticsInterval>('day');
  const summary = useUsageSummary();
  const tokens = useTokenAnalytics(interval);

  const s = summary.data?.summary;
  const byRoute: RouteUsage[] = tokens.data?.by_route ?? [];
  const byUser: UserUsage[] = tokens.data?.by_user ?? [];

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Analytics</h1>
          <p className="text-muted-foreground">Token / cost throughput and authorization decisions.</p>
        </div>
        <div className="flex gap-1 rounded-md border p-1" role="group" aria-label="Bucket interval">
          {(['hour', 'day'] as const).map((opt) => (
            <Button
              key={opt}
              size="sm"
              variant={interval === opt ? 'default' : 'ghost'}
              aria-pressed={interval === opt}
              onClick={() => setInterval(opt)}
            >
              Per {opt}
            </Button>
          ))}
        </div>
      </div>

      {summary.error ? (
        <p className="text-sm text-destructive">Failed to load summary: {summary.error.message}</p>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <StatTile label="Total tokens" value={s ? numberFmt.format(s.total_tokens) : '—'} />
          <StatTile label="Total requests" value={s ? numberFmt.format(s.total_requests) : '—'} />
          <StatTile
            label="Avg tokens / request"
            value={s ? s.avg_tokens_per_request.toFixed(1) : '—'}
          />
          <StatTile
            label="Avg latency"
            value={s ? `${s.avg_duration_ms.toFixed(0)} ms` : '—'}
          />
        </div>
      )}

      {tokens.error ? (
        <p className="text-sm text-destructive">Failed to load token analytics: {tokens.error.message}</p>
      ) : (
        <div className="grid gap-4 lg:grid-cols-2">
          <TimeSeriesCard series={tokens.data?.timeseries ?? []} interval={interval} />
          <TopBarCard
            title="Top routes"
            description="By token usage"
            rows={byRoute.map((r) => ({ label: r.route_id, tokens: r.tokens }))}
          />
          <TopBarCard
            title="Top users"
            description="By token usage"
            rows={byUser.map((u) => ({ label: u.user_id, tokens: u.tokens }))}
          />
        </div>
      )}

      <AuditTable />
    </div>
  );
}
