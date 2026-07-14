import { useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { CheckCircle, Clock, XCircle } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { useToast } from '@/components/ui/toast';
import { useApprovals, useDecideApproval } from '@/hooks/useAdmin';
import type { PendingApproval } from '@/api/types';

function expiresIn(isoStr: string): string {
  const ms = new Date(isoStr).getTime() - Date.now();
  if (ms <= 0) return 'expired';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

function ApprovalRow({ row }: { row: PendingApproval }) {
  const decide = useDecideApproval();
  const { toast } = useToast();

  const onDecide = async (decision: 'approve' | 'deny') => {
    try {
      await decide.mutateAsync({ id: row.approval_id, decision });
      toast({
        title: decision === 'approve' ? `Approved "${row.approval_id}"` : `Denied "${row.approval_id}"`,
        variant: 'success',
      });
    } catch (err) {
      toast({
        title: 'Decision failed',
        description: err instanceof Error ? err.message : undefined,
        variant: 'error',
      });
    }
  };

  const busy = decide.isPending;

  return (
    <TableRow>
      <TableCell className="font-mono text-xs">{row.approval_id}</TableCell>
      <TableCell className="text-sm">{row.principal_id}</TableCell>
      <TableCell>
        <Badge variant="outline">{row.action}</Badge>
      </TableCell>
      <TableCell className="text-sm text-muted-foreground">{row.resource_id}</TableCell>
      <TableCell className="text-sm text-muted-foreground">{row.reason ?? '—'}</TableCell>
      <TableCell className="text-xs tabular-nums text-muted-foreground">
        {expiresIn(row.expires_at)}
      </TableCell>
      <TableCell className="text-right">
        <Button
          variant="ghost"
          size="icon"
          disabled={busy}
          onClick={() => onDecide('approve')}
          title="Approve"
          className="text-green-600 hover:text-green-700"
        >
          <CheckCircle className="h-4 w-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          disabled={busy}
          onClick={() => onDecide('deny')}
          title="Deny"
          className="text-destructive hover:text-destructive/80"
        >
          <XCircle className="h-4 w-4" />
        </Button>
      </TableCell>
    </TableRow>
  );
}

export default function Approvals() {
  const [searchParams] = useSearchParams();
  const policyFilter = searchParams.get('policy');

  const { data, isLoading, error, refetch } = useApprovals();
  const allRows = data?.approvals ?? [];
  const rows = policyFilter
    ? allRows.filter((r) => r.resource_id === policyFilter)
    : allRows;

  useEffect(() => {
    const id = setInterval(() => { refetch(); }, 5_000);
    return () => clearInterval(id);
  }, [refetch]);

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Pending Approvals</h1>
        {policyFilter ? (
          <p className="text-muted-foreground">
            Showing approvals for policy <code className="text-xs">{policyFilter}</code>.{' '}
            <a href="/approvals" className="underline">
              View all
            </a>
          </p>
        ) : (
          <p className="text-muted-foreground">
            Tool calls paused by a Cedar <code>RequireApproval</code> policy. Approve or deny each
            request before its timeout expires. This view reflects the current replica only — in
            multi-replica deployments use a shared store or pin the admin session to one instance.
          </p>
        )}
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading approvals…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load: {error.message}</p>
      ) : rows.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-2 py-10 text-muted-foreground">
            <Clock className="h-8 w-8" />
            <p>No pending approvals{policyFilter ? ' for this policy' : ''}.</p>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Approval ID</TableHead>
                  <TableHead>Principal</TableHead>
                  <TableHead>Action</TableHead>
                  <TableHead>Resource</TableHead>
                  <TableHead>Reason</TableHead>
                  <TableHead>Expires in</TableHead>
                  <TableHead className="text-right">Decision</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((r) => (
                  <ApprovalRow key={r.approval_id} row={r} />
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
