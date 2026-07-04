import { useState } from 'react';
import { Pencil, Plus, Trash2 } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Modal } from '@/components/ui/dialog';
import { Textarea } from '@/components/ui/textarea';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { useToast } from '@/components/ui/toast';
import { useDeletePolicy, usePolicies, useUpsertPolicy } from '@/hooks/useAdmin';
import type { PolicyRow } from '@/api/types';

function emptyPolicy(): PolicyRow {
  return {
    id: '',
    policy_text: '',
    schema_json: null,
    entities_json: null,
    enabled: true,
  };
}

export default function Policies() {
  const { data, isLoading, error } = usePolicies();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<PolicyRow | null>(null);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Authorization Policies</h1>
          <p className="text-muted-foreground">Manage Cedar authorization policies.</p>
        </div>
        <Button onClick={() => { setEditing(null); setOpen(true); }}>
          <Plus className="mr-2 h-4 w-4" />
          Add Policy
        </Button>
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading policies…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load policies: {error.message}</p>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>Policies</CardTitle>
            <CardDescription>{data?.policies.length ?? 0} policy(s)</CardDescription>
          </CardHeader>
          <CardContent>
            {(data?.policies.length ?? 0) === 0 ? (
              <p className="text-sm text-muted-foreground">No policies configured.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>ID</TableHead>
                    <TableHead>Policy</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data?.policies.map((policy) => (
                    <PolicyRow key={policy.id} policy={policy} onEdit={() => { setEditing(policy); setOpen(true); }} />
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      )}

      <PolicyModal open={open} onClose={() => { setOpen(false); setEditing(null); }} policy={editing} />
    </div>
  );
}

function PolicyRow({ policy, onEdit }: { policy: PolicyRow; onEdit: () => void }) {
  const remove = useDeletePolicy();
  const { toast } = useToast();

  const handleDelete = async () => {
    if (!confirm(`Delete policy "${policy.id}"?`)) return;
    try {
      await remove.mutateAsync(policy.id);
      toast({ title: 'Policy deleted', variant: 'success' });
    } catch (e) {
      toast({ title: 'Failed to delete policy', description: (e as Error).message, variant: 'error' });
    }
  };

  return (
    <TableRow>
      <TableCell className="font-medium">{policy.id}</TableCell>
      <TableCell className="max-w-md truncate font-mono text-xs">{policy.policy_text}</TableCell>
      <TableCell>
        {policy.enabled ? (
          <Badge variant="default" className="bg-green-600 hover:bg-green-700">Enabled</Badge>
        ) : (
          <Badge variant="secondary">Disabled</Badge>
        )}
      </TableCell>
      <TableCell className="text-right">
        <Button variant="ghost" size="icon" onClick={onEdit}><Pencil className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon" onClick={handleDelete}><Trash2 className="h-4 w-4 text-destructive" /></Button>
      </TableCell>
    </TableRow>
  );
}

function PolicyModal({ open, onClose, policy }: { open: boolean; onClose: () => void; policy: PolicyRow | null }) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={policy ? 'Edit Policy' : 'Add Policy'}
      description={policy ? `Update policy ${policy.id}` : 'Create a new Cedar authorization policy.'}
    >
      <PolicyForm policy={policy} onClose={onClose} />
    </Modal>
  );
}

function PolicyForm({ policy, onClose }: { policy: PolicyRow | null; onClose: () => void }) {
  const upsert = useUpsertPolicy();
  const { toast } = useToast();
  const [form, setForm] = useState<PolicyRow>(policy ?? emptyPolicy());
  const [schemaJson, setSchemaJson] = useState(() => JSON.stringify(form.schema_json ?? {}, null, 2));
  const [entitiesJson, setEntitiesJson] = useState(() => JSON.stringify(form.entities_json ?? {}, null, 2));

  const update = <K extends keyof PolicyRow>(key: K, value: PolicyRow[K]) => setForm((f) => ({ ...f, [key]: value }));

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();

    let schema_json: Record<string, unknown> | null = null;
    let entities_json: Record<string, unknown> | null = null;
    try {
      schema_json = schemaJson.trim() && schemaJson !== '{}' ? JSON.parse(schemaJson) : null;
      entities_json = entitiesJson.trim() && entitiesJson !== '{}' ? JSON.parse(entitiesJson) : null;
    } catch {
      toast({ title: 'Invalid JSON in schema or entities', variant: 'error' });
      return;
    }

    const payload: PolicyRow = { ...form, schema_json, entities_json };
    try {
      const res = await upsert.mutateAsync(payload);
      toast({
        title: policy ? 'Policy updated' : 'Policy created',
        description: res.warnings?.length ? `Warnings: ${res.warnings.join(', ')}` : undefined,
        variant: res.warnings?.length ? 'warning' : 'success',
      });
      onClose();
    } catch (err) {
      toast({ title: 'Failed to save policy', description: (err as Error).message, variant: 'error' });
    }
  };

  return (
    <form id="policy-form" onSubmit={submit} className="space-y-4 max-h-[70vh] overflow-y-auto pr-1">
      <div className="space-y-1.5">
        <label className="text-sm font-medium">Policy ID</label>
        <Input value={form.id} onChange={(e) => update('id', e.target.value)} placeholder="allow-users" required disabled={!!policy} />
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium">Policy Text (Cedar)</label>
        <Textarea value={form.policy_text} onChange={(e) => update('policy_text', e.target.value)} rows={8} required />
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Schema JSON</label>
          <Textarea value={schemaJson} onChange={(e) => setSchemaJson(e.target.value)} rows={5} />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Entities JSON</label>
          <Textarea value={entitiesJson} onChange={(e) => setEntitiesJson(e.target.value)} rows={5} />
        </div>
      </div>

      <div className="flex items-center gap-2">
        <input id="policy-enabled" type="checkbox" checked={form.enabled} onChange={(e) => update('enabled', e.target.checked)} />
        <label htmlFor="policy-enabled" className="text-sm font-medium">Enabled</label>
      </div>

      <div className="flex justify-end gap-2 pt-2">
        <Button type="button" variant="ghost" onClick={onClose}>Cancel</Button>
        <Button type="submit" disabled={upsert.isPending}>{upsert.isPending ? 'Saving…' : policy ? 'Save Changes' : 'Create Policy'}</Button>
      </div>
    </form>
  );
}
