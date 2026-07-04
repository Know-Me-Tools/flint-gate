import { useMemo, useState } from 'react';
import { Pencil, Plus, Trash2 } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Modal } from '@/components/ui/dialog';
import { Select } from '@/components/ui/select';
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
import { useConfig, useDeleteRoute, useRoutes, useUpsertRoute } from '@/hooks/useAdmin';
import type { DbRoute, PreRequestHook, PostResponseHook, RouteConfig } from '@/api/types';

function emptyRoute(): RouteConfig {
  return {
    id: '',
    site: '',
    match: { path: '', methods: [] },
    upstream: '',
    auth: '',
    hooks: { pre_request: [], post_response: [] },
    stream: { enabled: false, protocol: 'sse' },
    priority: 0,
    enabled: true,
  };
}

export default function Routes() {
  const { data, isLoading, error } = useRoutes();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<DbRoute | null>(null);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Routes</h1>
          <p className="text-muted-foreground">Manage proxy routes and their matching rules.</p>
        </div>
        <Button onClick={() => { setEditing(null); setOpen(true); }}>
          <Plus className="mr-2 h-4 w-4" />
          Add Route
        </Button>
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading routes…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load routes: {error.message}</p>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>All Routes</CardTitle>
            <CardDescription>
              {data?.routes.length ?? 0} route(s) from {data?.source ?? '—'}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {(data?.routes.length ?? 0) === 0 ? (
              <p className="text-sm text-muted-foreground">No routes configured.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>ID</TableHead>
                    <TableHead>Site</TableHead>
                    <TableHead>Path</TableHead>
                    <TableHead>Methods</TableHead>
                    <TableHead>Upstream</TableHead>
                    <TableHead>Auth</TableHead>
                    <TableHead>Priority</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data?.routes.map((route) => (
                    <RouteRow
                      key={route.id}
                      route={route}
                      onEdit={() => { setEditing(route); setOpen(true); }}
                    />
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      )}

      <RouteModal
        open={open}
        onClose={() => { setOpen(false); setEditing(null); }}
        route={editing}
      />
    </div>
  );
}

function RouteRow({ route, onEdit }: { route: DbRoute; onEdit: () => void }) {
  const remove = useDeleteRoute();
  const { toast } = useToast();
  const cfg = route.config;

  const handleDelete = async () => {
    if (!confirm(`Delete route "${cfg.id}"?`)) return;
    try {
      await remove.mutateAsync(cfg.id);
      toast({ title: 'Route deleted', variant: 'success' });
    } catch (e) {
      toast({ title: 'Failed to delete route', description: (e as Error).message, variant: 'error' });
    }
  };

  return (
    <TableRow>
      <TableCell className="font-medium">{cfg.id}</TableCell>
      <TableCell>{cfg.site}</TableCell>
      <TableCell className="font-mono text-xs">{cfg.match.path}</TableCell>
      <TableCell>
        <div className="flex flex-wrap gap-1">
          {(cfg.match.methods?.length ? cfg.match.methods : ['ALL']).map((m) => (
            <Badge key={m} variant="outline">{m}</Badge>
          ))}
        </div>
      </TableCell>
      <TableCell className="max-w-[150px] truncate text-muted-foreground">
        {cfg.upstream ?? '—'}
      </TableCell>
      <TableCell>{cfg.auth ? <Badge variant="secondary">{cfg.auth}</Badge> : '—'}</TableCell>
      <TableCell>{route.priority}</TableCell>
      <TableCell>
        {route.enabled ? (
          <Badge variant="default" className="bg-green-600 hover:bg-green-700">Enabled</Badge>
        ) : (
          <Badge variant="secondary">Disabled</Badge>
        )}
      </TableCell>
      <TableCell className="text-right">
        <Button variant="ghost" size="icon" onClick={onEdit}>
          <Pencil className="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon" onClick={handleDelete}>
          <Trash2 className="h-4 w-4 text-destructive" />
        </Button>
      </TableCell>
    </TableRow>
  );
}

function RouteModal({ open, onClose, route }: { open: boolean; onClose: () => void; route: DbRoute | null }) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={route ? 'Edit Route' : 'Add Route'}
      description={route ? `Update route ${route.config.id}` : 'Create a new proxy route.'}
    >
      <RouteForm route={route} onClose={onClose} />
    </Modal>
  );
}

function RouteForm({ route, onClose }: { route: DbRoute | null; onClose: () => void }) {
  const config = useConfig();
  const upsert = useUpsertRoute();
  const { toast } = useToast();

  const initial = useMemo<RouteConfig>(() => {
    if (!route) return emptyRoute();
    return {
      ...route.config,
      upstream: route.config.upstream ?? '',
      auth: route.config.auth ?? '',
      stream: route.config.stream ?? { enabled: false, protocol: 'sse' },
      hooks: route.config.hooks ?? { pre_request: [], post_response: [] },
    };
  }, [route]);

  const [form, setForm] = useState<RouteConfig>(initial);
  const [preRequestJson, setPreRequestJson] = useState(
    JSON.stringify(initial.hooks?.pre_request ?? [], null, 2),
  );
  const [postResponseJson, setPostResponseJson] = useState(
    JSON.stringify(initial.hooks?.post_response ?? [], null, 2),
  );

  const sites = config.data?.sites ?? [];
  const authProviders = Object.keys(config.data?.auth_providers ?? {});

  const update = <K extends keyof RouteConfig>(key: K, value: RouteConfig[K]) => {
    setForm((f) => ({ ...f, [key]: value }));
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();

    let pre_request: PreRequestHook[] = [];
    let post_response: PostResponseHook[] = [];
    try {
      pre_request = preRequestJson.trim() ? JSON.parse(preRequestJson) : [];
      post_response = postResponseJson.trim() ? JSON.parse(postResponseJson) : [];
    } catch {
      toast({ title: 'Invalid hooks JSON', variant: 'error' });
      return;
    }

    const payload: RouteConfig = {
      ...form,
      upstream: form.upstream?.trim() || undefined,
      auth: form.auth?.trim() || undefined,
      match: {
        path: form.match.path,
        methods: form.match.methods,
        host: form.match.host?.trim() || undefined,
      },
      hooks: { pre_request, post_response },
      stream: {
        enabled: form.stream?.enabled ?? false,
        protocol: form.stream?.protocol ?? 'sse',
      },
    };

    try {
      await upsert.mutateAsync(payload);
      toast({ title: route ? 'Route updated' : 'Route created', variant: 'success' });
      onClose();
    } catch (err) {
      toast({ title: 'Failed to save route', description: (err as Error).message, variant: 'error' });
    }
  };

  return (
    <form id="route-form" onSubmit={submit} className="space-y-4 max-h-[70vh] overflow-y-auto pr-1">
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Route ID</label>
          <Input value={form.id} onChange={(e) => update('id', e.target.value)} placeholder="my-route" required />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Site</label>
          <Select value={form.site} onChange={(e) => update('site', e.target.value)} required>
            <option value="">Select site…</option>
            {sites.map((s) => (
              <option key={s.id} value={s.id}>{s.id}</option>
            ))}
          </Select>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Path</label>
          <Input value={form.match.path} onChange={(e) => update('match', { ...form.match, path: e.target.value })} placeholder="/api/**" required />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Methods (comma-separated)</label>
          <Input
            value={(form.match.methods ?? []).join(', ')}
            onChange={(e) => update('match', { ...form.match, methods: e.target.value.split(',').map((m) => m.trim()).filter(Boolean) })}
            placeholder="GET, POST"
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Host (optional)</label>
          <Input value={form.match.host ?? ''} onChange={(e) => update('match', { ...form.match, host: e.target.value })} placeholder="api.example.com" />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Upstream URL (optional)</label>
          <Input value={form.upstream ?? ''} onChange={(e) => update('upstream', e.target.value)} placeholder="http://localhost:8080" />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Auth Provider</label>
          <Select value={form.auth ?? ''} onChange={(e) => update('auth', e.target.value || undefined)}>
            <option value="">None</option>
            {authProviders.map((id) => (
              <option key={id} value={id}>{id}</option>
            ))}
          </Select>
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Priority</label>
          <Input type="number" value={form.priority ?? 0} onChange={(e) => update('priority', parseInt(e.target.value, 10) || 0)} />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Stream Protocol</label>
          <Select value={form.stream?.protocol ?? 'sse'} onChange={(e) => update('stream', { ...form.stream, protocol: e.target.value })}>
            <option value="sse">sse</option>
            <option value="websocket">websocket</option>
            <option value="ndjson">ndjson</option>
          </Select>
        </div>
        <div className="flex items-center gap-2 pt-6">
          <input
            id="enabled"
            type="checkbox"
            checked={form.enabled ?? true}
            onChange={(e) => update('enabled', e.target.checked)}
          />
          <label htmlFor="enabled" className="text-sm font-medium">Enabled</label>
        </div>
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium">Pre-request Hooks JSON</label>
        <Textarea value={preRequestJson} onChange={(e) => setPreRequestJson(e.target.value)} rows={5} />
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium">Post-response Hooks JSON</label>
        <Textarea value={postResponseJson} onChange={(e) => setPostResponseJson(e.target.value)} rows={3} />
      </div>

      <div className="flex justify-end gap-2 pt-2">
        <Button type="button" variant="ghost" onClick={onClose}>Cancel</Button>
        <Button type="submit" disabled={upsert.isPending}>
          {upsert.isPending ? 'Saving…' : route ? 'Save Changes' : 'Create Route'}
        </Button>
      </div>
    </form>
  );
}
