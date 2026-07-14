import { useEffect, useRef, useState } from 'react';
import { CheckCircle2, ChevronDown, ChevronRight, Clock, Loader2, Pencil, Plus, Trash2, X } from 'lucide-react';
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
import { serializeKey, useGraphStore } from '@prometheus-ags/prometheus-entity-management';
import { createPatch } from 'diff';
import { fetchPolicyHistory, rollbackPolicy, validatePolicy } from '@/api/admin';
import type { AdminEvent, PolicyHistoryResponse, PolicyParseError, PolicyRow, PolicyVersionRow, ToolScopeRequest, ValidateResponse } from '@/api/types';

const TOOL_SCOPE_ID_PREFIX = 'tool_scope::';
const VALIDATE_DEBOUNCE_MS = 500;

function agentFromToolScopeId(id: string): string {
  return id.startsWith(TOOL_SCOPE_ID_PREFIX) ? id.slice(TOOL_SCOPE_ID_PREFIX.length) : id;
}

function parseToolList(raw: string): string[] {
  return raw
    .split(/[,\s]+/)
    .map((t) => t.trim())
    .filter(Boolean);
}

function emptyPolicy(): PolicyRow {
  return { id: '', policy_text: '', schema_json: null, entities_json: null, enabled: true };
}

// ── Hot-reload banner ─────────────────────────────────────────────────────────

function useAdminSSE(adminApiBase: string) {
  const [reloadError, setReloadError] = useState<string | null>(null);

  useEffect(() => {
    const es = new EventSource(`${adminApiBase}/events`);

    es.onmessage = (event) => {
      let parsed: AdminEvent;
      try {
        parsed = JSON.parse(event.data as string) as AdminEvent;
      } catch {
        return;
      }
      if (parsed.type === 'policy_reload_error') {
        const msg = parsed.db_error ?? `${parsed.skipped_count} row(s) skipped`;
        setReloadError(`Policy reload error — ${msg}. Check server logs.`);
      } else if (parsed.type === 'policy_reload_ok') {
        setReloadError(null);
      }
    };

    return () => es.close();
  }, [adminApiBase]);

  return { reloadError, dismissReloadError: () => setReloadError(null) };
}

function ReloadErrorBanner({ message, onDismiss }: { message: string; onDismiss: () => void }) {
  return (
    <div className="flex items-start gap-3 rounded-md border border-yellow-500 bg-yellow-50 px-4 py-3 text-sm text-yellow-800 dark:border-yellow-600 dark:bg-yellow-950 dark:text-yellow-200">
      <span className="flex-1">{message}</span>
      <button
        onClick={onDismiss}
        className="shrink-0 opacity-70 hover:opacity-100"
        aria-label="Dismiss"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}

// ── Inline validation error list ──────────────────────────────────────────────

function InlineErrors({ errors }: { errors: PolicyParseError[] }) {
  if (errors.length === 0) return null;
  return (
    <ul className="mt-1.5 space-y-0.5 text-xs text-destructive">
      {errors.map((e, i) => (
        <li key={i}>
          Line {e.line}, Col {e.column}: {e.message}
        </li>
      ))}
    </ul>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export default function Policies() {
  const { data, isLoading, error } = usePolicies();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<PolicyRow | null>(null);
  const [scopeOpen, setScopeOpen] = useState(false);
  const [editingScope, setEditingScope] = useState<PolicyRow | null>(null);
  const { reloadError, dismissReloadError } = useAdminSSE('/api');

  return (
    <div className="space-y-6">
      {reloadError && <ReloadErrorBanner message={reloadError} onDismiss={dismissReloadError} />}

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
                    <TableHead>Last by</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data?.policies.map((policy) => (
                    <PolicyTableRow
                      key={policy.id}
                      policy={policy}
                      onEdit={() => { setEditing(policy); setOpen(true); }}
                    />
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

// ── Tool scopes section ───────────────────────────────────────────────────────

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

// ── Policy table row ──────────────────────────────────────────────────────────

function PolicyTableRow({ policy, onEdit }: { policy: PolicyRow; onEdit: () => void }) {
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
      <TableCell className="text-muted-foreground text-xs">{policy.written_by ?? '—'}</TableCell>
      <TableCell className="text-right">
        <Button variant="ghost" size="icon" onClick={onEdit}><Pencil className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon" onClick={handleDelete}><Trash2 className="h-4 w-4 text-destructive" /></Button>
      </TableCell>
    </TableRow>
  );
}

// ── Policy version history panel ──────────────────────────────────────────────

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleString(undefined, {
      dateStyle: 'short',
      timeStyle: 'short',
    });
  } catch {
    return iso;
  }
}

function PolicyDiffView({
  current,
  prior,
}: {
  current: PolicyVersionRow;
  prior: PolicyVersionRow | null;
}) {
  if (!prior) {
    return (
      <p className="text-xs text-muted-foreground py-2">
        No prior version available in this page — load more to compare.
      </p>
    );
  }

  const patch = createPatch(
    `v${prior.version_num} → v${current.version_num}`,
    prior.policy_text,
    current.policy_text,
  );

  const lines = patch.split('\n');

  return (
    <pre className="overflow-x-auto rounded-md bg-muted/30 p-3 text-xs font-mono leading-5 max-h-64 overflow-y-auto">
      {lines.map((line: string, i: number) => {
        let cls = '';
        if (line.startsWith('+') && !line.startsWith('+++')) cls = 'text-green-700 dark:text-green-400';
        else if (line.startsWith('-') && !line.startsWith('---')) cls = 'text-destructive';
        else if (line.startsWith('@')) cls = 'text-muted-foreground';
        return (
          <span key={i} className={cls || undefined}>
            {line}
            {'\n'}
          </span>
        );
      })}
    </pre>
  );
}

interface PolicyVersionHistoryProps {
  policyId: string;
  onRestoreToEditor: (text: string) => void;
}

function PolicyVersionHistory({ policyId, onRestoreToEditor }: PolicyVersionHistoryProps) {
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState<PolicyHistoryResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [viewedVersion, setViewedVersion] = useState<PolicyVersionRow | null>(null);
  const [rollingBack, setRollingBack] = useState<number | null>(null);
  const [rollbackError, setRollbackError] = useState<string | null>(null);
  const [rollbackSuccess, setRollbackSuccess] = useState<string | null>(null);
  const [offset, setOffset] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [viewMode, setViewMode] = useState<'text' | 'diff'>('text');
  const { toast } = useToast();

  const PAGE = 20;

  const computeHasMore = (versions: PolicyVersionRow[], totalHint: number | null | undefined) => {
    if (totalHint != null) return totalHint > versions.length;
    return versions.length >= PAGE;
  };

  const loadHistory = async () => {
    setLoading(true);
    setLoadError(null);
    setOffset(0);
    try {
      const result = await fetchPolicyHistory(policyId, 0, PAGE);
      setHistory(result);
      setHasMore(computeHasMore(result.versions, result.total_hint));
    } catch (e) {
      setLoadError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  const loadMoreHistory = async () => {
    if (!history) return;
    setLoadingMore(true);
    const nextOffset = offset + PAGE;
    try {
      const result = await fetchPolicyHistory(policyId, nextOffset, PAGE);
      const combined = [...history.versions, ...result.versions];
      setHistory({ ...history, versions: combined, total_hint: result.total_hint });
      setOffset(nextOffset);
      setHasMore(computeHasMore(combined, result.total_hint));
    } catch (e) {
      toast({ title: 'Failed to load more history', description: (e as Error).message, variant: 'error' });
    } finally {
      setLoadingMore(false);
    }
  };

  const handleToggle = () => {
    const next = !open;
    setOpen(next);
    if (next && !history && !loading) {
      void loadHistory();
    }
  };

  const handleRollback = async (versionNum: number) => {
    if (!confirm(`Restore version ${versionNum} of "${policyId}"? A new version entry will be created.`)) return;
    setRollingBack(versionNum);
    setRollbackError(null);
    setRollbackSuccess(null);
    try {
      const res = await rollbackPolicy(policyId, versionNum);
      setRollbackSuccess(`Rolled back to v${res.from_version} → new version v${res.to_version}`);
      toast({ title: 'Policy rolled back', description: `v${res.from_version} → v${res.to_version}`, variant: 'success' });
      useGraphStore.getState().setListStale(serializeKey(['policies']), true);
      // Refresh the history panel to show the new version row.
      await loadHistory();
    } catch (e) {
      const msg = (e as Error).message;
      setRollbackError(msg);
      toast({ title: 'Rollback failed', description: msg, variant: 'error' });
    } finally {
      setRollingBack(null);
    }
  };

  return (
    <div className="rounded-md border border-border">
      <button
        type="button"
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-4 py-3 text-sm font-medium text-left hover:bg-muted/50 transition-colors"
        aria-expanded={open}
      >
        {open ? <ChevronDown className="h-4 w-4 shrink-0" /> : <ChevronRight className="h-4 w-4 shrink-0" />}
        <Clock className="h-4 w-4 shrink-0 text-muted-foreground" />
        Version History
      </button>

      {open && (
        <div className="border-t border-border px-4 pb-4 pt-3 space-y-3">
          {loading && (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading history…
            </div>
          )}
          {loadError && (
            <p className="text-sm text-destructive">Failed to load history: {loadError}</p>
          )}
          {rollbackSuccess && (
            <div className="flex items-center gap-2 text-sm text-green-700 dark:text-green-400">
              <CheckCircle2 className="h-4 w-4" />
              {rollbackSuccess}
            </div>
          )}
          {rollbackError && (
            <p className="text-sm text-destructive">Rollback failed: {rollbackError}</p>
          )}

          {history && history.versions.length === 0 && (
            <p className="text-sm text-muted-foreground">No version history yet.</p>
          )}

          {history && history.versions.length > 0 && (
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-left text-muted-foreground border-b border-border">
                    <th className="pb-1.5 pr-3 font-medium">Version</th>
                    <th className="pb-1.5 pr-3 font-medium">Saved at</th>
                    <th className="pb-1.5 pr-3 font-medium">Saved by</th>
                    <th className="pb-1.5 text-right font-medium">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {history.versions.map((v) => (
                    <tr key={v.version_num} className="border-b border-border/50 last:border-0">
                      <td className="py-1.5 pr-3 font-mono font-medium">v{v.version_num}</td>
                      <td className="py-1.5 pr-3 text-muted-foreground">{formatDate(v.written_at)}</td>
                      <td className="py-1.5 pr-3 text-muted-foreground">{v.written_by ?? '—'}</td>
                      <td className="py-1.5 text-right space-x-1">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs"
                          onClick={() => { setViewedVersion(viewedVersion?.version_num === v.version_num ? null : v); setViewMode('text'); }}
                        >
                          {viewedVersion?.version_num === v.version_num ? 'Hide' : 'View'}
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs text-destructive hover:text-destructive"
                          disabled={rollingBack !== null}
                          onClick={() => void handleRollback(v.version_num)}
                        >
                          {rollingBack === v.version_num ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            'Rollback'
                          )}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {hasMore && (
            <div className="pt-1">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full text-xs h-7"
                disabled={loadingMore}
                onClick={() => void loadMoreHistory()}
              >
                {loadingMore ? (
                  <Loader2 className="h-3 w-3 animate-spin mr-1" />
                ) : null}
                {loadingMore ? 'Loading…' : 'Load more'}
              </Button>
            </div>
          )}

          {viewedVersion && (
            <div className="space-y-2 pt-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium text-muted-foreground">
                  v{viewedVersion.version_num} — {formatDate(viewedVersion.written_at)}
                </span>
                <div className="flex items-center gap-1">
                  {viewedVersion.version_num > 1 && (
                    <div className="flex rounded-md border border-border overflow-hidden text-xs">
                      <button
                        type="button"
                        className={`px-2 py-0.5 transition-colors ${viewMode === 'text' ? 'bg-muted font-medium' : 'hover:bg-muted/50'}`}
                        onClick={() => setViewMode('text')}
                      >
                        Text
                      </button>
                      <button
                        type="button"
                        className={`px-2 py-0.5 border-l border-border transition-colors ${viewMode === 'diff' ? 'bg-muted font-medium' : 'hover:bg-muted/50'}`}
                        onClick={() => setViewMode('diff')}
                      >
                        Diff
                      </button>
                    </div>
                  )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-6 px-2 text-xs"
                    onClick={() => {
                      onRestoreToEditor(viewedVersion.policy_text);
                      toast({ title: 'Restored to editor', description: `v${viewedVersion.version_num} text copied to editor — review and save to apply.`, variant: 'success' });
                    }}
                  >
                    Restore to editor
                  </Button>
                </div>
              </div>
              {viewMode === 'text' ? (
                <Textarea
                  value={viewedVersion.policy_text}
                  readOnly
                  rows={6}
                  className="font-mono text-xs bg-muted/30 cursor-default"
                />
              ) : (
                <PolicyDiffView
                  current={viewedVersion}
                  prior={history?.versions.find((v) => v.version_num === viewedVersion.version_num - 1) ?? null}
                />
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Policy modal + form ───────────────────────────────────────────────────────

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

  // Validation state
  const [validateResult, setValidateResult] = useState<ValidateResponse | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const runValidation = async (text: string) => {
    if (!text.trim()) {
      setValidateResult(null);
      return;
    }
    setIsValidating(true);
    try {
      const result = await validatePolicy(text);
      setValidateResult(result);
    } catch {
      // Network/auth errors don't block editing; clear stale result
      setValidateResult(null);
    } finally {
      setIsValidating(false);
    }
  };

  const handlePolicyTextChange = (text: string) => {
    setForm((f) => ({ ...f, policy_text: text }));
    if (debounceRef.current) clearTimeout(debounceRef.current);
    setValidateResult(null);
    if (text.trim()) {
      debounceRef.current = setTimeout(() => void runValidation(text), VALIDATE_DEBOUNCE_MS);
    }
  };

  const handleValidateClick = () => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    void runValidation(form.policy_text);
  };

  // Clean up debounce timer on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const update = <K extends keyof PolicyRow>(key: K, value: PolicyRow[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const isInvalid = validateResult !== null && !validateResult.valid;

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
        <Input
          value={form.id}
          onChange={(e) => update('id', e.target.value)}
          placeholder="allow-users"
          required
          disabled={!!policy}
        />
      </div>

      <div className="space-y-1.5">
        <div className="flex items-center justify-between">
          <label className="text-sm font-medium">Policy Text (Cedar)</label>
          <div className="flex items-center gap-2">
            {isValidating && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
            {validateResult?.valid && !isValidating && (
              <span className="flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
                <CheckCircle2 className="h-3.5 w-3.5" />
                Policy is valid
              </span>
            )}
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-7 px-2 text-xs"
              onClick={handleValidateClick}
              disabled={isValidating || !form.policy_text.trim()}
            >
              Validate
            </Button>
          </div>
        </div>
        <Textarea
          value={form.policy_text}
          onChange={(e) => handlePolicyTextChange(e.target.value)}
          rows={8}
          required
          className={isInvalid ? 'border-destructive focus-visible:ring-destructive' : ''}
        />
        <InlineErrors errors={validateResult?.errors ?? []} />
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
        <input
          id="policy-enabled"
          type="checkbox"
          checked={form.enabled}
          onChange={(e) => update('enabled', e.target.checked)}
        />
        <label htmlFor="policy-enabled" className="text-sm font-medium">Enabled</label>
      </div>

      {policy && (
        <PolicyVersionHistory
          policyId={policy.id}
          onRestoreToEditor={(text) => handlePolicyTextChange(text)}
        />
      )}

      <div className="flex justify-end gap-2 pt-2">
        <Button type="button" variant="ghost" onClick={onClose}>Cancel</Button>
        <Button type="submit" disabled={upsert.isPending || isInvalid}>
          {upsert.isPending ? 'Saving…' : policy ? 'Save Changes' : 'Create Policy'}
        </Button>
      </div>
    </form>
  );
}
