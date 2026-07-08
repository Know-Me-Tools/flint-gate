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
import {
  useDeletePolicy,
  useDeleteToolScope,
  usePolicies,
  useToolScopes,
  useUpsertPolicy,
  useUpsertToolScope,
} from '@/hooks/useAdmin';
import type { PolicyRow, ToolScopeRequest } from '@/api/types';

/** Prefix of the DB policy id under which UI-authored tool-scopes are stored. */
const TOOL_SCOPE_ID_PREFIX = 'tool_scope::';

/** Extract the agent name from a `tool_scope::<agent>` policy id. */
function agentFromToolScopeId(id: string): string {
  return id.startsWith(TOOL_SCOPE_ID_PREFIX) ? id.slice(TOOL_SCOPE_ID_PREFIX.length) : id;
}

/** Parse a comma/whitespace-separated tool list into trimmed, non-empty tokens. */
function parseToolList(raw: string): string[] {
  return raw
    .split(/[,\s]+/)
    .map((t) => t.trim())
    .filter(Boolean);
}

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
  const [scopeOpen, setScopeOpen] = useState(false);
  const [editingScope, setEditingScope] = useState<PolicyRow | null>(null);

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

      <ToolScopesSection
        onAdd={() => { setEditingScope(null); setScopeOpen(true); }}
        onEdit={(row) => { setEditingScope(row); setScopeOpen(true); }}
      />

      <PolicyModal open={open} onClose={() => { setOpen(false); setEditing(null); }} policy={editing} />
      <ToolScopeModal open={scopeOpen} onClose={() => { setScopeOpen(false); setEditingScope(null); }} row={editingScope} />
    </div>
  );
}

function ToolScopesSection({ onAdd, onEdit }: { onAdd: () => void; onEdit: (row: PolicyRow) => void }) {
  const { data, isLoading, error } = useToolScopes();
  const remove = useDeleteToolScope();
  const { toast } = useToast();
  const scopes = data?.tool_scopes ?? [];

  const handleDelete = async (row: PolicyRow) => {
    const agent = agentFromToolScopeId(row.id);
    if (!confirm(`Delete tool scope for agent "${agent}"?`)) return;
    try {
      await remove.mutateAsync(agent);
      toast({ title: 'Tool scope deleted', variant: 'success' });
    } catch (e) {
      toast({ title: 'Failed to delete tool scope', description: (e as Error).message, variant: 'error' });
    }
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div>
          <CardTitle>Agent Tool Scopes</CardTitle>
          <CardDescription>
            Author per-agent allow/deny tool scopes — compiled to Cedar server-side (deny wins; <code>*</code> globs).
          </CardDescription>
        </div>
        <Button size="sm" onClick={onAdd}>
          <Plus className="mr-2 h-4 w-4" />
          Add Tool Scope
        </Button>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <p className="text-sm text-muted-foreground">Loading tool scopes…</p>
        ) : error ? (
          <p className="text-sm text-destructive">Failed to load tool scopes: {error.message}</p>
        ) : scopes.length === 0 ? (
          <p className="text-sm text-muted-foreground">No agent tool scopes configured.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Agent</TableHead>
                <TableHead>Compiled policy</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {scopes.map((row) => (
                <TableRow key={row.id}>
                  <TableCell className="font-medium">{agentFromToolScopeId(row.id)}</TableCell>
                  <TableCell className="max-w-md truncate font-mono text-xs">{row.policy_text}</TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="icon" onClick={() => onEdit(row)}><Pencil className="h-4 w-4" /></Button>
                    <Button variant="ghost" size="icon" onClick={() => handleDelete(row)}><Trash2 className="h-4 w-4 text-destructive" /></Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}

function ToolScopeModal({ open, onClose, row }: { open: boolean; onClose: () => void; row: PolicyRow | null }) {
  const editingAgent = row ? agentFromToolScopeId(row.id) : null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={editingAgent ? 'Edit Tool Scope' : 'Add Tool Scope'}
      description={
        editingAgent
          ? `Re-author the tool scope for ${editingAgent}.`
          : 'Grant/deny tools for an agent. Compiles to validated Cedar — no raw Cedar needed.'
      }
    >
      <ToolScopeForm editingAgent={editingAgent} onClose={onClose} />
    </Modal>
  );
}

function ToolScopeForm({ editingAgent, onClose }: { editingAgent: string | null; onClose: () => void }) {
  const upsert = useUpsertToolScope();
  const { toast } = useToast();
  const [agent, setAgent] = useState(editingAgent ?? '');
  const [allow, setAllow] = useState('');
  const [deny, setDeny] = useState('');

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const payload: ToolScopeRequest = {
      agent: agent.trim(),
      allow: parseToolList(allow),
      deny: parseToolList(deny),
    };
    if (!payload.agent) {
      toast({ title: 'Agent id is required', variant: 'error' });
      return;
    }
    if (payload.allow.length === 0 && payload.deny.length === 0) {
      toast({ title: 'Add at least one allow or deny tool', variant: 'error' });
      return;
    }
    try {
      await upsert.mutateAsync(payload);
      toast({ title: editingAgent ? 'Tool scope updated' : 'Tool scope created', variant: 'success' });
      onClose();
    } catch (err) {
      toast({ title: 'Failed to save tool scope', description: (err as Error).message, variant: 'error' });
    }
  };

  return (
    <form id="tool-scope-form" onSubmit={submit} className="space-y-4">
      <div className="space-y-1.5">
        <label className="text-sm font-medium">Agent ID</label>
        <Input
          value={agent}
          onChange={(e) => setAgent(e.target.value)}
          placeholder="ci-bot"
          required
          disabled={!!editingAgent}
        />
      </div>
      <div className="space-y-1.5">
        <label className="text-sm font-medium">Allow tools</label>
        <Textarea value={allow} onChange={(e) => setAllow(e.target.value)} rows={2} placeholder="deploy, run_tests, read_*" />
        <p className="text-xs text-muted-foreground">Comma or space separated. <code>*</code> is a glob.</p>
      </div>
      <div className="space-y-1.5">
        <label className="text-sm font-medium">Deny tools <span className="text-muted-foreground">(wins over allow)</span></label>
        <Textarea value={deny} onChange={(e) => setDeny(e.target.value)} rows={2} placeholder="delete_*" />
      </div>
      <div className="flex justify-end gap-2 pt-2">
        <Button type="button" variant="ghost" onClick={onClose}>Cancel</Button>
        <Button type="submit" disabled={upsert.isPending}>
          {upsert.isPending ? 'Saving…' : editingAgent ? 'Save Changes' : 'Create Tool Scope'}
        </Button>
      </div>
    </form>
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
