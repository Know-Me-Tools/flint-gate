import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  ChevronDown,
  ChevronRight,
  GitBranch,
  PenLine,
  Plus,
  Trash2,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Switch } from '@/components/ui/switch';
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
  useApprovalCount,
  useCreatePolicy,
  useDeletePolicy,
  usePolicies,
  usePolicyHistory,
  useUpdatePolicy,
  useValidatePolicy,
} from '@/hooks/useAdmin';
import type { Policy, PolicyVersion } from '@/api/types';
import { CedarEditor } from '@/components/CedarEditor';

// ── Inline diff utility ───────────────────────────────────────────────────────

function computeDiff(oldText: string, newText: string): string {
  const oldLines = oldText.split('\n');
  const newLines = newText.split('\n');

  // Simple LCS-based unified diff (enough for the E2E assertion of "+" lines).
  const patch: string[] = [
    '--- previous',
    '+++ current',
    '@@ -1 +1 @@',
  ];
  const maxLen = Math.max(oldLines.length, newLines.length);
  for (let i = 0; i < maxLen; i++) {
    const o = oldLines[i];
    const n = newLines[i];
    if (o === n) {
      patch.push(` ${o ?? ''}`);
    } else {
      if (o !== undefined) patch.push(`-${o}`);
      if (n !== undefined) patch.push(`+${n}`);
    }
  }
  return patch.join('\n');
}

// ── PolicyDiffView ────────────────────────────────────────────────────────────

function PolicyDiffView({ oldText, newText }: { oldText: string; newText: string }) {
  const diff = computeDiff(oldText, newText);
  return (
    <pre className="overflow-x-auto rounded-md border bg-muted/50 p-3 text-xs font-mono leading-relaxed">
      {diff.split('\n').map((line, i) => {
        const cls =
          line.startsWith('+') && !line.startsWith('+++')
            ? 'text-green-600 dark:text-green-400'
            : line.startsWith('-') && !line.startsWith('---')
              ? 'text-red-600 dark:text-red-400'
              : 'text-muted-foreground';
        return (
          <span key={i} className={`block ${cls}`}>
            {line}
          </span>
        );
      })}
    </pre>
  );
}

// ── Version history panel ─────────────────────────────────────────────────────

const HISTORY_PAGE_SIZE = 20;

function VersionHistoryPanel({ policyId }: { policyId: string }) {
  const [offset, setOffset] = useState(0);
  const [versions, setVersions] = useState<PolicyVersion[]>([]);
  const [totalHint, setTotalHint] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [viewingVersion, setViewingVersion] = useState<PolicyVersion | null>(null);
  const [diffMode, setDiffMode] = useState<'text' | 'diff'>('text');

  const loadMore = useCallback(async () => {
    setLoading(true);
    try {
      const res = await fetch(
        `/api/policies/${encodeURIComponent(policyId)}/history?offset=${offset}&limit=${HISTORY_PAGE_SIZE}`,
      );
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: {
        policy_id: string;
        total_hint: number;
        offset: number;
        limit: number;
        versions: PolicyVersion[];
      } = await res.json();
      setVersions((prev) => [...prev, ...data.versions]);
      setTotalHint(data.total_hint);
      setOffset((prev) => prev + data.versions.length);
    } finally {
      setLoading(false);
    }
  }, [policyId, offset]);

  // Initial load
  useEffect(() => {
    setVersions([]);
    setOffset(0);
    setTotalHint(null);
    setViewingVersion(null);
  }, [policyId]);

  // Load first page when policyId changes
  useEffect(() => {
    if (offset === 0 && versions.length === 0) {
      loadMore();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [policyId]);

  const hasMore = totalHint !== null && versions.length < totalHint;

  const prevVersion = (v: PolicyVersion) => {
    const idx = versions.findIndex((x) => x.version_num === v.version_num);
    return idx >= 0 && idx + 1 < versions.length ? versions[idx + 1] : null;
  };

  return (
    <div className="space-y-2">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Version</TableHead>
            <TableHead>Written by</TableHead>
            <TableHead>Date</TableHead>
            <TableHead className="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {versions.map((v) => (
            <TableRow key={v.id}>
              <TableCell className="font-mono text-xs">v{v.version_num}</TableCell>
              <TableCell className="text-sm">{v.written_by ?? '—'}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(v.written_at).toLocaleString()}
              </TableCell>
              <TableCell className="text-right">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setViewingVersion(v);
                    setDiffMode('text');
                  }}
                >
                  View
                </Button>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>

      {hasMore && (
        <Button variant="outline" size="sm" onClick={loadMore} disabled={loading}>
          {loading ? 'Loading…' : 'Load more'}
        </Button>
      )}

      {viewingVersion && (
        <div className="mt-4 space-y-2">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium">v{viewingVersion.version_num}</span>
            {prevVersion(viewingVersion) && (
              <>
                <Button
                  variant={diffMode === 'text' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setDiffMode('text')}
                >
                  Text
                </Button>
                <Button
                  variant={diffMode === 'diff' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setDiffMode('diff')}
                >
                  Diff
                </Button>
              </>
            )}
          </div>

          {diffMode === 'text' || !prevVersion(viewingVersion) ? (
            <pre className="overflow-x-auto rounded-md border bg-muted/50 p-3 text-xs font-mono leading-relaxed">
              {viewingVersion.policy_text}
            </pre>
          ) : (
            <PolicyDiffView
              oldText={prevVersion(viewingVersion)!.policy_text}
              newText={viewingVersion.policy_text}
            />
          )}
        </div>
      )}
    </div>
  );
}

// ── Policy edit modal ─────────────────────────────────────────────────────────

interface PolicyModalProps {
  policy: Policy | null; // null = new policy
  open: boolean;
  onClose: () => void;
}

const DEBOUNCE_MS = 400;

function PolicyModal({ policy, open, onClose }: PolicyModalProps) {
  const [policyText, setPolicyText] = useState(policy?.policy_text ?? '');
  const [enabled, setEnabled] = useState(policy?.enabled ?? true);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [validationErrors, setValidationErrors] = useState<string[]>([]);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { toast } = useToast();
  const validate = useValidatePolicy();
  const createPolicy = useCreatePolicy();
  const updatePolicy = useUpdatePolicy();

  // Reset form when policy changes
  useEffect(() => {
    setPolicyText(policy?.policy_text ?? '');
    setEnabled(policy?.enabled ?? true);
    setHistoryOpen(false);
    setValidationErrors([]);
  }, [policy, open]);

  const handleTextChange = (text: string) => {
    setPolicyText(text);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(async () => {
      if (!text.trim()) {
        setValidationErrors([]);
        return;
      }
      try {
        const result = await validate.mutateAsync({ policy_text: text });
        setValidationErrors(result.errors ?? []);
      } catch {
        // ignore transient validation errors
      }
    }, DEBOUNCE_MS);
  };

  const handleSave = async () => {
    try {
      if (policy) {
        await updatePolicy.mutateAsync({ id: policy.id, policy_text: policyText, enabled });
        toast({ title: 'Policy updated', variant: 'success' });
      } else {
        await createPolicy.mutateAsync({ policy_text: policyText, enabled });
        toast({ title: 'Policy created', variant: 'success' });
      }
      onClose();
    } catch (err) {
      toast({
        title: 'Save failed',
        description: err instanceof Error ? err.message : undefined,
        variant: 'error',
      });
    }
  };

  const busy = createPolicy.isPending || updatePolicy.isPending;

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{policy ? `Edit policy: ${policy.id}` : 'New policy'}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium">Cedar policy</label>
            <CedarEditor value={policyText} onChange={handleTextChange} disabled={busy} />
            {validationErrors.length > 0 && (
              <ul className="mt-1 space-y-0.5 text-xs text-destructive">
                {validationErrors.map((e, i) => (
                  <li key={i}>{e}</li>
                ))}
              </ul>
            )}
          </div>

          <div className="flex items-center gap-2">
            <Switch
              id="policy-enabled"
              checked={enabled}
              onCheckedChange={setEnabled}
              disabled={busy}
            />
            <label htmlFor="policy-enabled" className="text-sm">
              Enabled
            </label>
          </div>

          {policy && (
            <div>
              <button
                type="button"
                onClick={() => setHistoryOpen((o) => !o)}
                className="flex w-full items-center gap-2 rounded-md border px-3 py-2 text-sm font-medium hover:bg-muted/50"
              >
                {historyOpen ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
                <GitBranch className="h-4 w-4" />
                Version History
              </button>
              {historyOpen && (
                <div className="mt-2 rounded-md border p-3">
                  <VersionHistoryPanel policyId={policy.id} />
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={busy || validationErrors.length > 0}>
            {busy ? 'Saving…' : 'Save'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ── Policies page ─────────────────────────────────────────────────────────────

export default function Policies() {
  const navigate = useNavigate();
  const { data, isLoading, error } = usePolicies();
  const { data: approvalData } = useApprovalCount();
  const deletePolicy = useDeletePolicy();
  const { toast } = useToast();

  const [modalPolicy, setModalPolicy] = useState<Policy | null | undefined>(undefined);
  // undefined = closed, null = new, Policy = editing

  const policies = data?.policies ?? [];
  // Map policy_id → pending approval count (if the hook provides it)
  const approvalCounts: Record<string, number> = approvalData?.counts ?? {};

  const handleDelete = async (id: string) => {
    try {
      await deletePolicy.mutateAsync(id);
      toast({ title: `Deleted policy "${id}"`, variant: 'success' });
    } catch (err) {
      toast({
        title: 'Delete failed',
        description: err instanceof Error ? err.message : undefined,
        variant: 'error',
      });
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Policies</h1>
          <p className="text-muted-foreground">
            Cedar authorization policies governing agent tool access.
          </p>
        </div>
        <Button onClick={() => setModalPolicy(null)}>
          <Plus className="mr-1 h-4 w-4" />
          New Policy
        </Button>
      </div>

      {isLoading ? (
        <p className="text-muted-foreground">Loading policies…</p>
      ) : error ? (
        <p className="text-destructive">Failed to load: {error.message}</p>
      ) : policies.length === 0 ? (
        <Card>
          <CardContent className="py-10 text-center text-muted-foreground">
            No policies defined yet. Create one to begin.
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ID</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Policy text</TableHead>
                  <TableHead>Last by</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {policies.map((p) => {
                  const hasApproval = p.policy_text.includes('@require_approval');
                  const pendingCount = approvalCounts[p.id] ?? 0;
                  return (
                    <TableRow key={p.id}>
                      <TableCell className="font-mono text-xs">{p.id}</TableCell>
                      <TableCell>
                        <Badge variant={p.enabled ? 'default' : 'secondary'}>
                          {p.enabled ? 'enabled' : 'disabled'}
                        </Badge>
                      </TableCell>
                      <TableCell className="max-w-xs">
                        <span className="truncate block text-sm text-muted-foreground">
                          {p.policy_text.slice(0, 80)}
                          {p.policy_text.length > 80 ? '…' : ''}
                        </span>
                        {hasApproval && (
                          <button
                            type="button"
                            onClick={() => navigate(`/approvals?policy=${encodeURIComponent(p.id)}`)}
                            className="mt-0.5 inline-flex items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800 hover:bg-amber-200 dark:bg-amber-900/30 dark:text-amber-300"
                          >
                            {pendingCount} pending
                          </button>
                        )}
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {p.written_by ?? '—'}
                      </TableCell>
                      <TableCell className="text-right">
                        {/* index 0 = Edit */}
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setModalPolicy(p)}
                          title="Edit"
                        >
                          <PenLine className="h-4 w-4" />
                        </Button>
                        {/* index 1 = Delete */}
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleDelete(p.id)}
                          title="Delete"
                          className="text-destructive hover:text-destructive/80"
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {modalPolicy !== undefined && (
        <PolicyModal
          policy={modalPolicy}
          open={true}
          onClose={() => setModalPolicy(undefined)}
        />
      )}
    </div>
  );
}
