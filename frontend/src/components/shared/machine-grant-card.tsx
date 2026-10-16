import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useMachineGrant, useSaveMachineGrant, useRevokeMachineGrant, machineGrantUnavailable } from "@/hooks/use-machine-grants";
import type { MachineGrantSelection, MachineGrantSource } from "@/schemas/machine-grants";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { formatDate } from "@/lib/utils";
import { MachineGrantEditor } from "./machine-grant-editor";
import { ErrorBanner } from "./error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

function reasonLabel(reason: string | null | undefined): string {
  switch (reason) {
    case "missing": return "Not found";
    case "disabled": return "Disabled";
    case "expired": return "Expired";
    case "revoked": return "Revoked";
    case "changed": return "Changed";
    case "ineligible": return "Not eligible";
    default: return "Unknown";
  }
}

function liveLabel(valid: boolean | undefined, reason: string | null | undefined): string {
  if (valid === true) return "Valid";
  if (reason) return reasonLabel(reason);
  return "Unknown";
}

interface MachineGrantCardProps {
  readonly source: MachineGrantSource;
  readonly canWrite?: boolean;
  readonly eligible?: boolean;
  readonly stateVersion?: number;
  readonly allowedNodeIds?: readonly string[];
  readonly allowedServiceIds?: readonly string[];
}
export function MachineGrantCard(props: MachineGrantCardProps) {
  const owner = useAuthStore((state) => state.user?.id);
  return <MachineGrantCardContent key={`${owner}:${props.source.kind}:${props.source.id}`} {...props} />;
}
function MachineGrantCardContent({ source, canWrite = true, eligible = true, stateVersion, allowedNodeIds, allowedServiceIds }: MachineGrantCardProps) {
  const query = useMachineGrant(source);
  const save = useSaveMachineGrant(source);
  const revoke = useRevokeMachineGrant(source);
  const [editing, setEditing] = useState(false);
  const [revokeReview, setRevokeReview] = useState<{ id: string; revision: number } | null>(null);
  const [error, setError] = useState<string>();
  const [needsReview, setNeedsReview] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  const grant = query.error ? undefined : query.data?.grant;
  const active = grant?.status === "active" && new Date(grant.expires_at).getTime() > now;
  const independentValidity = query.data?.source_valid !== undefined && query.data?.grant_valid !== undefined;
  const sourceValid = !query.isFetching && query.data?.source_valid;
  const grantValid = !query.isFetching && active && query.data?.grant_valid;
  const grantReason = grant?.status === "revoked" ? "revoked" : grant && new Date(grant.expires_at).getTime() <= now ? "expired" : query.data?.grant_validity_reason;
  const combinedLiveValid = !query.isFetching && active && query.data?.source_and_grant_valid === true && (!independentValidity || (sourceValid === true && grantValid === true));
  const revokeReviewCurrent = revokeReview !== null && grant?.id === revokeReview.id && grant.revision === revokeReview.revision;
  const sourceVersionAvailable = source.kind !== "api_key" || Number.isSafeInteger(stateVersion);
  const canEdit = !needsReview && canWrite && eligible && sourceVersionAvailable && !query.error && !query.isFetching && Boolean(query.data) && query.data?.source_valid !== false && (!active || query.data?.grant_valid !== false) && (source.kind === "api_key" || Boolean(active && combinedLiveValid));
  const selection: MachineGrantSelection | undefined = grant ? {
    assignments: Object.fromEntries(Object.entries(grant.assignments).map(([id, assignment]) => [id, { capabilities: assignment.capabilities, file_roots: assignment.file_roots, service_ids: assignment.service_ids }])),
    expires_at: active ? grant.expires_at : new Date(now + 30 * 86400000).toISOString(),
  } : undefined;
  async function submit(value: MachineGrantSelection) {
    setError(undefined);
    try {
      await save.mutateAsync({
        expected_grant_id: active && grant ? grant.id : null,
        expected_revision: active && grant ? grant.revision : null,
        ...(source.kind === "api_key" ? { expected_state_version: stateVersion } : {}),
        selection: value,
      });
      setEditing(false);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) { setNeedsReview(true); setEditing(false); }
      setError(err instanceof ApiError && err.status === 409 ? "The grant or its authentication source changed. Reload and review the latest permissions before saving again." : err instanceof Error ? err.message : "Could not save Machine permissions.");
    }
  }
  return <Card>
    <CardHeader><CardTitle>Machine permissions</CardTitle></CardHeader>
    <CardContent className="space-y-4">
      <p className="text-12 text-muted-foreground">{source.kind === "api_key" ? "This parent Key and all its valid login credentials share one Machine grant. Logging out a child credential stops only that child’s work; revoking or rotating the parent affects all children and requires new approval for a successor." : "This account and OAuth client share one Machine grant across connections. First approval and reauthorization use the application’s OAuth consent flow."}</p>
      <p className="text-11 text-muted-foreground">Revoking only Machine permissions leaves other service consent intact and does not pause or delete the Machine. Changing a grant invalidates its earlier work across every assigned Machine; independent subjects and Assistant assignments are separate.</p>
      {!eligible && <p className="text-12 text-muted-foreground">This source is not eligible for external Machine permissions. Use an active personal general-purpose Key or the application’s explicit Machine consent.</p>}
      <>
        {query.isPending && <p role="status">Loading Machine permissions…</p>}
        {query.error && <ErrorBanner message={machineGrantUnavailable(query.error) ? "Machine grant management is not available on this server yet. No permission state has been inferred." : "Could not verify Machine permissions. Reload before changing access."} onRetry={() => void query.refetch()} />}
        {!query.error && query.data && <>
          <div className="flex flex-wrap gap-2">
            <Badge variant={combinedLiveValid ? "success" : "warning"}>Source + grant: {query.isFetching ? "checking" : combinedLiveValid ? "valid" : "not currently valid"}</Badge>
            {independentValidity && <><Badge variant={sourceValid ? "success" : "warning"}>Source: {query.isFetching ? "Checking" : liveLabel(sourceValid, query.data.source_validity_reason)}</Badge><Badge variant={grantValid ? "success" : "warning"}>Grant validity: {query.isFetching ? "Checking" : liveLabel(grantValid, grantReason)}</Badge></>}
            <Badge variant={active ? "secondary" : "warning"}>Grant: {!grant ? "none" : active ? "active" : grant.status === "revoked" ? "revoked" : "expired"}</Badge>
          </div>
          <p className="text-11 text-muted-foreground">{independentValidity ? "Source authentication and Machine grant validity are live checks over their authoritative records. Unknown or stale results do not establish execution authority, Machine readiness, or the client’s local token state." : "The server reports one combined source and grant check; it does not report source validity separately. This result is separate from Machine resource readiness and from the client’s local token or transport state."}</p>
          {grant && <>
            <dl className="grid gap-x-4 gap-y-2 text-12 sm:grid-cols-2"><div><dt className="text-muted-foreground">Grant ID</dt><dd className="break-all font-mono">{grant.id}</dd></div><div><dt className="text-muted-foreground">Revision / policy revision</dt><dd>{grant.revision} / {grant.policy_revision}</dd></div><div><dt className="text-muted-foreground">Expires</dt><dd>{formatDate(grant.expires_at)}</dd></div><div><dt className="text-muted-foreground">Source</dt><dd className="break-all font-mono">{source.kind === "api_key" ? "Parent Key" : "OAuth client"}: {source.id}</dd></div></dl>
            {Object.entries(grant.assignments).map(([nodeId, assignment]) => <article key={nodeId} className="space-y-1 rounded-lg border border-border p-3 text-12"><Link to="/assistant/machines" search={{ machine: nodeId }} className="break-all font-mono text-primary-text hover:underline">{nodeId}</Link><p>{[assignment.capabilities.shell && "Commands", assignment.capabilities.files && "Files"].filter(Boolean).join(" · ")} · {assignment.mode} · assignment revision {assignment.revision}</p><p className="break-all">File roots: {assignment.file_roots.join(", ") || "None"}</p><p className="break-all">Gateway service ceiling: {assignment.service_ids.join(", ") || "None"}</p></article>)}
          </>}
          {(query.data.source_valid === false || (active && query.data.grant_valid === false)) && <p className="text-12 text-warning">Current source or grant checks prevent editing. Reload and review the original authentication source before approving new Machine permissions.</p>}
          {!sourceVersionAvailable && <p role="alert" className="text-12 text-warning">This server did not expose the parent Key revision required for safe approval. Reload after server integration; approvals are unavailable.</p>}
          {source.kind === "oauth" && !active && <p className="text-12 text-muted-foreground">Reconnect from this application and explicitly approve Machine access to create a new grant. Existing consent cannot silently restore revoked or expired Machine permissions.</p>}
          <div className="flex flex-wrap gap-2"><Button disabled={!canEdit} onClick={() => setEditing((value) => !value)}>{editing ? "Close editor" : active ? "Edit Machine permissions" : "Approve Machine permissions"}</Button>{active && grant && canWrite && <Button variant="destructive" disabled={needsReview || query.isFetching} onClick={() => setRevokeReview({ id: grant.id, revision: grant.revision })}>Revoke Machine permissions</Button>}<Button variant="outline" onClick={() => { setEditing(false); setError(undefined); void query.refetch().then(() => setNeedsReview(false)); }}>Reload permissions</Button></div>
        </>}
      </>
      {error && <ErrorBanner message={error} />}
      {editing && canEdit && <MachineGrantEditor key={`${grant?.id ?? "new"}-${grant?.revision ?? 0}`} initial={selection} onSubmit={submit} submitting={save.isPending} allowedNodeIds={allowedNodeIds} allowedServiceIds={allowedServiceIds} />}
      <Dialog open={revokeReview !== null} onOpenChange={(open) => { if (!open) setRevokeReview(null); }}><DialogContent><DialogHeader><DialogTitle>Revoke Machine permissions?</DialogTitle><DialogDescription>This stops work authorized by this grant across all its Machines. Other service access remains unchanged. The Machine and workspace are retained.</DialogDescription></DialogHeader>{revokeReview && !revokeReviewCurrent && <p role="alert" className="text-12 text-warning">Machine permissions changed since this review. Close this dialog, reload, and review the current grant before revoking.</p>}<DialogFooter><Button variant="outline" onClick={() => setRevokeReview(null)}>Cancel</Button><Button variant="destructive" disabled={Boolean(query.error) || query.isFetching || !revokeReviewCurrent || revoke.isPending} isLoading={revoke.isPending} onClick={() => {
        if (query.error || query.isFetching || !revokeReviewCurrent || !revokeReview) return;
        setError(undefined);
        void revoke.mutateAsync({ expected_grant_id: revokeReview.id, expected_revision: revokeReview.revision }).then(() => { setRevokeReview(null); setEditing(false); }).catch((err: unknown) => { if (err instanceof ApiError && err.status === 409) { setNeedsReview(true); setEditing(false); } setError(err instanceof ApiError && err.status === 409 ? "Machine permissions changed. Reload and review before revoking." : err instanceof Error ? err.message : "Could not revoke Machine permissions."); setRevokeReview(null); });
      }}>Revoke Machine permissions</Button></DialogFooter></DialogContent></Dialog>
    </CardContent>
  </Card>;
}
