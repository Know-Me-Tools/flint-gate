import { useState } from 'react';
import { Bot, Plus, RotateCw, Trash2 } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Modal } from '@/components/ui/dialog';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { useToast } from '@/components/ui/toast';
import {
  useAgentIdentities,
  useIssueAgentIdentity,
  useRevokeAgentIdentity,
  useRotateAgentIdentity,
} from '@/hooks/useAdmin';
import type { AgentIdentityKind, AgentIdentityStatus } from '@/api/types';

const STATUS_VARIANT: Record<AgentIdentityStatus, 'default' | 'destructive'> = {
  active: 'default',
  revoked: 'destructive',
};

function IssueForm({ onClose }: { onClose: () => void }) {
  const [id, setId] = useState('');
  const [kind, setKind] = useState<AgentIdentityKind>('agent');
  const [label, setLabel] = useState('');
  const issue = useIssueAgentIdentity();
  const { toast } = useToast();

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      await issue.mutateAsync({ id: id.trim(), kind, label: label.trim() || undefined });
      toast({ title: `Issued ${kind} identity “${id}”`, variant: 'success' });
      onClose();
    } catch (err) {
      toast({ title: 'Failed to issue identity', description: err instanceof Error ? err.message : undefined, variant: 'error' });
    }
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <div className="space-y-1">
        <label className="text-sm font-medium">Identity ID</label>
        <Input value={id} onChange={(e) => setId(e.target.value)} required placeholder="bot-7" />
      </div>
      <div className="space-y-1">
        <label className="text-sm font-medium">Kind</label>
        <div className="flex gap-1 rounded-md border p-1" role="radiogroup" aria-label="Identity kind">
          {(['agent', 'service'] as const).map((k) => (
            <Button
              key={k}
              type="button"
              size="sm"
              variant={kind === k ? 'default' : 'ghost'}
              aria-pressed={kind === k}
              onClick={() => setKind(k)}
            >
              {k}
            </Button>
          ))}
        </div>
      </div>
      <div className="space-y-1">
        <label className="text-sm font-medium">Label (optional)</label>
        <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="CI deploy bot" />
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={issue.isPending}>
          {issue.isPending ? 'Issuing…' : 'Issue Identity'}
        </Button>
      </div>
    </form>
  );
}

export default function AgentIdentities() {
  const { data, isLoading, error } = useAgentIdentities();
  const [open, setOpen] = useState(false);
  const rotate = useRotateAgentIdentity();
  const revoke = useRevokeAgentIdentity();
  const { toast } = useToast();

  const rows = data?.agent_identities ?? [];

  const onRotate = async (id: string) => {
    try {
      await rotate.mutateAsync(id);
      toast({ title: `Rotated “${id}”`, variant: 'success' });
    } catch (err) {
      toast({ title: 'Rotate failed', description: err instanceof Error ? err.message : undefined, variant: 'error' });
    }
  };

  const onRevoke = async (id: string) => {
    if (!confirm(`Revoke identity “${id}”? It will be denied on its next request.`)) return;
    try {
      await revoke.mutateAsync(id);
      toast({ title: `Revoked “${id}”`, variant: 'success' });
    } catch (err) {
      toast({ title: 'Revoke failed', description: err instanceof Error ? err.message : undefined, variant: 'error' });
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Agent Identities</h1>
          <p className="text-muted-foreground">
            Non-human identities (agents &amp; services) that policies can name as principals.
            Revoking one denies it on its next authorize.
          </p>
        </div>
        <Button onClick={() => setOpen(true)}>
          <Plus className="mr-2 h-4 w-4" />
          Issue Identity
        </Button>
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading identities…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load: {error.message}</p>
      ) : rows.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-2 py-10 text-muted-foreground">
            <Bot className="h-8 w-8" />
            <p>No agent or service identities yet.</p>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ID</TableHead>
                  <TableHead>Kind</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Label</TableHead>
                  <TableHead>Rotated</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((r) => (
                  <TableRow key={r.id}>
                    <TableCell className="font-mono text-sm">{r.id}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{r.kind}</Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={STATUS_VARIANT[r.status]}>{r.status}</Badge>
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">{r.label ?? '—'}</TableCell>
                    <TableCell className="text-xs tabular-nums text-muted-foreground">
                      {r.rotated_at ? new Date(r.rotated_at).toLocaleString() : '—'}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={r.status !== 'active'}
                        onClick={() => onRotate(r.id)}
                        title="Rotate"
                      >
                        <RotateCw className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={r.status !== 'active'}
                        onClick={() => onRevoke(r.id)}
                        title="Revoke"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <Modal open={open} onClose={() => setOpen(false)} title="Issue Agent Identity">
        <IssueForm onClose={() => setOpen(false)} />
      </Modal>
    </div>
  );
}
