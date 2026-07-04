import { useState } from 'react';
import { Copy, Plus, Trash2 } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
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
import { useApiKeys, useCreateApiKey, useRevokeApiKey } from '@/hooks/useAdmin';

export default function ApiKeys() {
  const { data, isLoading, error } = useApiKeys();
  const [open, setOpen] = useState(false);
  const [created, setCreated] = useState<{ key: string; client_id: string } | null>(null);
  const { toast } = useToast();

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">API Keys</h1>
          <p className="text-muted-foreground">Manage client API keys for route authentication.</p>
        </div>
        <Button onClick={() => setOpen(true)}>
          <Plus className="mr-2 h-4 w-4" />
          Create Key
        </Button>
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading keys…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load keys: {error.message}</p>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>Active Keys</CardTitle>
            <CardDescription>{data?.api_keys.length ?? 0} key(s)</CardDescription>
          </CardHeader>
          <CardContent>
            {(data?.api_keys.length ?? 0) === 0 ? (
              <p className="text-sm text-muted-foreground">No API keys configured.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>ID</TableHead>
                    <TableHead>Client ID</TableHead>
                    <TableHead>Scopes</TableHead>
                    <TableHead>Expires</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data?.api_keys.map((key) => (
                    <KeyRow key={key.id} apiKey={key} />
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      )}

      <CreateKeyModal
        open={open}
        onClose={() => {
          setOpen(false);
          setCreated(null);
        }}
        onCreated={(k) => setCreated(k)}
      />

      {created && (
        <Modal
          open={!!created}
          onClose={() => setCreated(null)}
          title="API Key Created"
          description="Copy the key now. It will not be shown again."
          footer={
            <Button onClick={() => setCreated(null)} variant="secondary">
              Done
            </Button>
          }
        >
          <div className="space-y-3">
            <div className="rounded-md bg-muted p-3 font-mono text-sm break-all">{created.key}</div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(created.key);
                toast({ title: 'Copied to clipboard', variant: 'success' });
              }}
            >
              <Copy className="mr-2 h-4 w-4" />
              Copy
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function KeyRow({ apiKey }: { apiKey: { id: string; client_id: string; scopes: string[]; expires_at?: string | null } }) {
  const revoke = useRevokeApiKey();
  const { toast } = useToast();

  const handleRevoke = async () => {
    if (!confirm('Revoke this API key?')) return;
    try {
      await revoke.mutateAsync(apiKey.id);
      toast({ title: 'API key revoked', variant: 'success' });
    } catch (e) {
      toast({ title: 'Failed to revoke key', description: (e as Error).message, variant: 'error' });
    }
  };

  return (
    <TableRow>
      <TableCell className="font-mono text-xs">{apiKey.id}</TableCell>
      <TableCell className="font-medium">{apiKey.client_id}</TableCell>
      <TableCell>
        {apiKey.scopes.length === 0 ? (
          <span className="text-muted-foreground">—</span>
        ) : (
          <div className="flex flex-wrap gap-1">
            {apiKey.scopes.map((s) => (
              <Badge key={s} variant="outline">
                {s}
              </Badge>
            ))}
          </div>
        )}
      </TableCell>
      <TableCell className="text-muted-foreground">
        {apiKey.expires_at ? new Date(apiKey.expires_at).toLocaleString() : 'Never'}
      </TableCell>
      <TableCell className="text-right">
        <Button variant="ghost" size="icon" onClick={handleRevoke}>
          <Trash2 className="h-4 w-4 text-destructive" />
        </Button>
      </TableCell>
    </TableRow>
  );
}

function CreateKeyModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (k: { key: string; client_id: string }) => void;
}) {
  const create = useCreateApiKey();
  const { toast } = useToast();
  const [clientId, setClientId] = useState('');
  const [scopes, setScopes] = useState('');
  const [expiresAt, setExpiresAt] = useState('');

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const res = await create.mutateAsync({
        client_id: clientId,
        scopes: scopes
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        expires_at: expiresAt ? new Date(expiresAt).toISOString() : null,
      });
      toast({ title: 'API key created', variant: 'success' });
      setClientId('');
      setScopes('');
      setExpiresAt('');
      onClose();
      onCreated({ key: res.key, client_id: res.client_id });
    } catch (err) {
      toast({ title: 'Failed to create key', description: (err as Error).message, variant: 'error' });
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Create API Key"
      description="Generate a new client API key."
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" form="create-key-form" disabled={!clientId || create.isPending}>
            {create.isPending ? 'Creating…' : 'Create'}
          </Button>
        </>
      }
    >
      <form id="create-key-form" onSubmit={submit} className="space-y-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Client ID</label>
          <Input value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="my-service" required />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Scopes (comma-separated)</label>
          <Input value={scopes} onChange={(e) => setScopes(e.target.value)} placeholder="read, write" />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">Expires At (optional)</label>
          <Input type="datetime-local" value={expiresAt} onChange={(e) => setExpiresAt(e.target.value)} />
        </div>
      </form>
    </Modal>
  );
}
