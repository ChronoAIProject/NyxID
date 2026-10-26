import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  useMachineGrant,
  useSaveMachineGrant,
  useRevokeMachineGrant,
  machineGrantUnavailable,
} from "@/hooks/use-machine-grants";
import { useApiKey } from "@/hooks/use-api-keys";
import type {
  MachineGrantSelection,
  MachineGrantSource,
} from "@/schemas/machine-grants";
import type { ApiKey } from "@/types/api";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { SHARED_LEGACY_WARNING } from "@/schemas/machines";
import { formatDate } from "@/lib/utils";
import { MachineGrantEditor } from "./machine-grant-editor";
import { ErrorBanner } from "./error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

function reasonLabel(reason: string | null | undefined): string {
  switch (reason) {
    case "missing":
      return "Not found";
    case "disabled":
      return "Disabled";
    case "expired":
      return "Expired";
    case "revoked":
      return "Revoked";
    case "changed":
      return "Changed";
    case "ineligible":
      return "Not eligible";
    default:
      return "Unknown";
  }
}

function liveLabel(
  valid: boolean | undefined,
  reason: string | null | undefined,
): string {
  if (valid === true) return "Valid";
  if (reason) return reasonLabel(reason);
  return "Unknown";
}

interface MachineGrantCardProps {
  readonly source: MachineGrantSource;
  /** When mounted from a Machine detail page, scope actions to this Machine. */
  readonly machineId?: string;
  readonly canWrite?: boolean;
  readonly eligible?: boolean;
  readonly stateVersion?: number;
  readonly allowedNodeIds?: readonly string[];
  readonly allowedServiceIds?: readonly string[];
}
export function MachineGrantCard(props: MachineGrantCardProps) {
  const owner = useAuthStore((state) => state.user?.id);
  return (
    <MachineGrantCardBoundary
      key={`${owner}:${props.source.kind}:${props.source.id}`}
      {...props}
    />
  );
}
function MachineGrantCardBoundary(props: MachineGrantCardProps) {
  if (
    props.machineId &&
    props.source.kind === "api_key" &&
    props.stateVersion === undefined
  ) {
    return <MachineGrantCardWithKey {...props} />;
  }
  return <MachineGrantCardContent {...props} />;
}
function MachineGrantCardWithKey(props: MachineGrantCardProps) {
  const query = useApiKey(
    props.source.kind === "api_key" ? props.source.id : "",
  );
  return (
    <MachineGrantCardContent
      {...props}
      keyQuery={{
        data: query.data,
        isPending: query.isPending,
        error: query.error,
      }}
    />
  );
}
type SourceKeyQuery = {
  readonly data?: ApiKey;
  readonly isPending: boolean;
  readonly error?: unknown;
};
function MachineGrantCardContent({
  source,
  machineId,
  canWrite = true,
  eligible = true,
  stateVersion,
  allowedNodeIds,
  allowedServiceIds,
  keyQuery,
}: MachineGrantCardProps & { readonly keyQuery?: SourceKeyQuery }) {
  const query = useMachineGrant(source);
  const save = useSaveMachineGrant(source);
  const revoke = useRevokeMachineGrant(source);
  const [editing, setEditing] = useState(false);
  const [revokeReview, setRevokeReview] = useState<{
    id: string;
    revision: number;
  } | null>(null);
  const [removeReview, setRemoveReview] = useState<{
    id: string;
    revision: number;
  } | null>(null);
  const [removeLegacyAccepted, setRemoveLegacyAccepted] = useState(false);
  const [error, setError] = useState<string>();
  const [needsReview, setNeedsReview] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const grant = query.error ? undefined : query.data?.grant;
  const active =
    grant?.status === "active" && new Date(grant.expires_at).getTime() > now;
  const independentValidity =
    query.data?.source_valid !== undefined &&
    query.data?.grant_valid !== undefined;
  const sourceValid = !query.isFetching && query.data?.source_valid;
  const grantValid = !query.isFetching && active && query.data?.grant_valid;
  const grantReason =
    grant?.status === "revoked"
      ? "revoked"
      : grant && new Date(grant.expires_at).getTime() <= now
        ? "expired"
        : query.data?.grant_validity_reason;
  const combinedLiveValid =
    !query.isFetching &&
    active &&
    query.data?.source_and_grant_valid === true &&
    (!independentValidity || (sourceValid === true && grantValid === true));
  const revokeReviewCurrent =
    revokeReview !== null &&
    grant?.id === revokeReview.id &&
    grant.revision === revokeReview.revision;
  const currentKey = source.kind === "api_key" ? keyQuery?.data : undefined;
  const currentStateVersion = stateVersion ?? currentKey?.state_version;
  const sourceVersionAvailable =
    source.kind !== "api_key" || Number.isSafeInteger(currentStateVersion);
  const sourceMetadataPending =
    source.kind === "api_key" &&
    stateVersion === undefined &&
    (keyQuery?.isPending ?? true);
  const sourceMetadataError =
    source.kind === "api_key" &&
    stateVersion === undefined &&
    Boolean(keyQuery?.error);
  const effectiveNodeIds =
    allowedNodeIds ??
    (currentKey && !currentKey.allow_all_nodes
      ? currentKey.allowed_node_ids
      : undefined);
  const effectiveServiceIds =
    allowedServiceIds ??
    (currentKey && !currentKey.allow_all_services
      ? currentKey.allowed_service_ids
      : undefined);
  const canApprove = source.kind === "api_key" && sourceValid === true;
  const canEdit =
    !needsReview &&
    canWrite &&
    eligible &&
    sourceVersionAvailable &&
    !query.error &&
    !query.isFetching &&
    Boolean(query.data) &&
    (active ? combinedLiveValid : canApprove);
  const selection: MachineGrantSelection | undefined = grant
    ? {
        assignments: Object.fromEntries(
          Object.entries(grant.assignments).map(([id, assignment]) => [
            id,
            {
              capabilities: assignment.capabilities,
              file_roots: assignment.file_roots,
              service_ids: assignment.service_ids,
            },
          ]),
        ),
        expires_at: active
          ? grant.expires_at
          : new Date(now + 30 * 86400000).toISOString(),
      }
    : undefined;
  const currentAssignment =
    machineId && grant ? grant.assignments[machineId] : undefined;
  const removeReviewCurrent =
    removeReview !== null &&
    grant?.id === removeReview.id &&
    grant.revision === removeReview.revision &&
    Boolean(machineId && grant.assignments[machineId]);
  const canManageCurrent = Boolean(
    machineId && currentAssignment && active && canEdit,
  );
  async function submit(value: MachineGrantSelection) {
    setError(undefined);
    if (machineId && !value.assignments[machineId]) {
      setError(
        "Remove this Machine with the explicit removal action so the affected grant scope can be reviewed.",
      );
      return;
    }
    try {
      await save.mutateAsync({
        expected_grant_id: active && grant ? grant.id : null,
        expected_revision: active && grant ? grant.revision : null,
        ...(source.kind === "api_key" &&
        Number.isSafeInteger(currentStateVersion)
          ? { expected_state_version: currentStateVersion }
          : {}),
        selection: value,
      });
      setEditing(false);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        setNeedsReview(true);
        setEditing(false);
      }
      setError(
        err instanceof ApiError && err.status === 409
          ? "The grant or its authentication source changed. Reload and review the latest permissions before saving again."
          : err instanceof Error
            ? err.message
            : "Could not save Machine permissions.",
      );
    }
  }
  async function removeCurrentMachine() {
    if (!machineId || !grant || !active || !removeReviewCurrent) return;
    setError(undefined);
    try {
      if (Object.keys(grant.assignments).length > 1) {
        if (!removeLegacyAccepted) return;
        const assignments = Object.fromEntries(
          Object.entries(grant.assignments).filter(
            ([nodeId]) => nodeId !== machineId,
          ),
        );
        await save.mutateAsync({
          expected_grant_id: grant.id,
          expected_revision: grant.revision,
          ...(source.kind === "api_key" &&
          Number.isSafeInteger(currentStateVersion)
            ? { expected_state_version: currentStateVersion }
            : {}),
          selection: {
            assignments,
            expires_at: grant.expires_at,
            separation: "shared_legacy",
            legacy_warning_acknowledged: true,
          },
        });
      } else {
        await revoke.mutateAsync({
          expected_grant_id: grant.id,
          expected_revision: grant.revision,
        });
      }
      setRemoveReview(null);
      setEditing(false);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        setNeedsReview(true);
        setEditing(false);
      }
      setError(
        err instanceof ApiError && err.status === 409
          ? "The grant or its authentication source changed. Reload and review before removing this Machine."
          : err instanceof Error
            ? err.message
            : "Could not remove this Machine from the source grant.",
      );
      setRemoveReview(null);
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Machine permissions</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-12 text-muted-foreground">
          {source.kind === "api_key"
            ? "This parent Key and all its valid login credentials share one Machine grant. Logging out a child credential stops only that child’s work; revoking or rotating the parent affects all children and requires new approval for a successor."
            : "This account and OAuth client share one Machine grant across connections. First approval and reauthorization use the application’s OAuth consent flow."}
        </p>
        <p className="text-11 text-muted-foreground">
          Revoking only Machine permissions leaves other service consent intact
          and does not pause or delete the Machine. Changing a grant invalidates
          its earlier work across every assigned Machine; independent subjects
          and Assistant assignments are separate.
        </p>
        {!eligible && (
          <p className="text-12 text-muted-foreground">
            This source is not eligible for external Machine permissions. Use an
            active personal general-purpose Key or the application’s explicit
            Machine consent.
          </p>
        )}
        <>
          {query.isPending && <p role="status">Loading Machine permissions…</p>}
          {query.error && (
            <ErrorBanner
              message={
                machineGrantUnavailable(query.error)
                  ? "Machine grant management is not available on this server yet. No permission state has been inferred."
                  : "Could not verify Machine permissions. Reload before changing access."
              }
              onRetry={() => void query.refetch()}
            />
          )}
          {!query.error && query.data && (
            <>
              <div className="flex flex-wrap gap-2">
                <Badge variant={combinedLiveValid ? "success" : "warning"}>
                  Source + grant:{" "}
                  {query.isFetching
                    ? "checking"
                    : combinedLiveValid
                      ? "valid"
                      : "not currently valid"}
                </Badge>
                {independentValidity && (
                  <>
                    <Badge variant={sourceValid ? "success" : "warning"}>
                      Source:{" "}
                      {query.isFetching
                        ? "Checking"
                        : liveLabel(
                            sourceValid,
                            query.data.source_validity_reason,
                          )}
                    </Badge>
                    <Badge variant={grantValid ? "success" : "warning"}>
                      Grant validity:{" "}
                      {query.isFetching
                        ? "Checking"
                        : liveLabel(grantValid, grantReason)}
                    </Badge>
                  </>
                )}
                <Badge variant={active ? "secondary" : "warning"}>
                  Grant:{" "}
                  {!grant
                    ? "none"
                    : active
                      ? "active"
                      : grant.status === "revoked"
                        ? "revoked"
                        : "expired"}
                </Badge>
              </div>
              <p className="text-11 text-muted-foreground">
                {independentValidity
                  ? "Source authentication and Machine grant validity are live checks over their authoritative records. Unknown or stale results do not establish execution authority, Machine readiness, or the client’s local token state."
                  : "The server reports one combined source and grant check; it does not report source validity separately. This result is separate from Machine resource readiness and from the client’s local token or transport state."}
              </p>
              {source.kind === "api_key" ? (
                <div className="space-y-2 rounded-lg border border-border/50 bg-muted/30 p-3 text-12">
                  <p className="font-medium">Current parent Key review</p>
                  {sourceMetadataPending ? (
                    <p role="status" className="text-muted-foreground">
                      Loading current Key state and ceilings…
                    </p>
                  ) : null}
                  {sourceMetadataError ? (
                    <p role="alert" className="text-warning">
                      The current parent Key could not be read. Editing is
                      unavailable until its live state can be reviewed.
                    </p>
                  ) : null}
                  {!sourceMetadataPending &&
                  !sourceMetadataError &&
                  currentKey ? (
                    <dl className="grid gap-2 sm:grid-cols-2">
                      <div>
                        <dt className="text-muted-foreground">
                          Key state version
                        </dt>
                        <dd>{currentKey.state_version ?? "Not reported"}</dd>
                      </div>
                      <div>
                        <dt className="text-muted-foreground">Node ceiling</dt>
                        <dd>
                          {currentKey.allow_all_nodes
                            ? "All eligible Nodes"
                            : currentKey.allowed_node_ids.join(", ") || "None"}
                        </dd>
                      </div>
                      <div>
                        <dt className="text-muted-foreground">
                          Service ceiling
                        </dt>
                        <dd>
                          {currentKey.allow_all_services
                            ? "All eligible services"
                            : currentKey.allowed_service_ids.join(", ") ||
                              "None"}
                        </dd>
                      </div>
                    </dl>
                  ) : null}
                </div>
              ) : null}
              {grant && (
                <>
                  <dl className="grid gap-x-4 gap-y-2 text-12 sm:grid-cols-2">
                    <div>
                      <dt className="text-muted-foreground">Grant ID</dt>
                      <dd className="break-all font-mono">{grant.id}</dd>
                    </div>
                    <div>
                      <dt className="text-muted-foreground">
                        Revision / policy revision
                      </dt>
                      <dd>
                        {grant.revision} / {grant.policy_revision}
                      </dd>
                    </div>
                    <div>
                      <dt className="text-muted-foreground">Expires</dt>
                      <dd>{formatDate(grant.expires_at)}</dd>
                    </div>
                    <div>
                      <dt className="text-muted-foreground">Source</dt>
                      <dd className="break-all font-mono">
                        {source.kind === "api_key"
                          ? "Parent Key"
                          : "OAuth client"}
                        : {source.id}
                      </dd>
                    </div>
                  </dl>
                  <div
                    className="space-y-2"
                    aria-label="Affected machine assignments"
                  >
                    {Object.entries(grant.assignments).map(
                      ([nodeId, assignment]) => (
                        <article
                          key={nodeId}
                          className={`space-y-1 rounded-lg border p-3 text-12 ${nodeId === machineId ? "border-primary/50" : "border-border"}`}
                        >
                          <div className="flex flex-wrap items-center gap-2">
                            <Link
                              to="/assistant/machines"
                              search={{ machine: nodeId }}
                              className="break-all font-mono text-primary-text hover:underline"
                            >
                              {nodeId}
                            </Link>
                            {nodeId === machineId ? (
                              <Badge variant="info">Current Machine</Badge>
                            ) : null}
                          </div>
                          <p>
                            {[
                              assignment.capabilities.shell && "Commands",
                              assignment.capabilities.files && "Files",
                            ]
                              .filter(Boolean)
                              .join(" · ") || "No execution capability"}{" "}
                            · {assignment.mode} · assignment revision{" "}
                            {assignment.revision}
                          </p>
                          <p className="break-all">
                            File roots:{" "}
                            {assignment.file_roots.join(", ") || "None"}
                          </p>
                          <p className="break-all">
                            Gateway service ceiling:{" "}
                            {assignment.service_ids.join(", ") || "None"}
                          </p>
                        </article>
                      ),
                    )}
                  </div>
                  {machineId && !currentAssignment ? (
                    <p role="alert" className="text-12 text-warning">
                      This source grant no longer includes the current Machine.
                      Reload the Machine permissions list before taking an
                      action.
                    </p>
                  ) : null}
                </>
              )}
              {(query.data.source_valid === false ||
                (active && query.data.grant_valid === false)) && (
                <p className="text-12 text-warning">
                  Current source or grant checks prevent editing. Reload and
                  review the original authentication source before approving new
                  Machine permissions.
                </p>
              )}
              {!sourceVersionAvailable && (
                <p role="alert" className="text-12 text-warning">
                  This server did not expose the parent Key revision required
                  for safe approval. Reload after server integration; approvals
                  are unavailable.
                </p>
              )}
              {source.kind === "oauth" && !active && (
                <p className="text-12 text-muted-foreground">
                  Reconnect from this application and explicitly approve Machine
                  access to create a new grant. Existing consent cannot silently
                  restore revoked or expired Machine permissions.
                </p>
              )}
              <div className="flex flex-wrap gap-2">
                <Button
                  disabled={
                    !canEdit || Boolean(machineId && !currentAssignment)
                  }
                  onClick={() => setEditing((value) => !value)}
                >
                  {editing
                    ? "Close editor"
                    : active
                      ? machineId
                        ? "Edit current source grant"
                        : "Edit Machine permissions"
                      : "Approve Machine permissions"}
                </Button>
                {canManageCurrent ? (
                  <Button
                    variant="outline"
                    disabled={save.isPending || revoke.isPending || needsReview}
                    onClick={() => {
                      setRemoveLegacyAccepted(false);
                      setRemoveReview({
                        id: grant!.id,
                        revision: grant!.revision,
                      });
                    }}
                  >
                    Remove current Machine
                  </Button>
                ) : null}
                {active && grant && canWrite && (
                  <Button
                    variant="destructive"
                    disabled={needsReview || query.isFetching}
                    onClick={() =>
                      setRevokeReview({
                        id: grant.id,
                        revision: grant.revision,
                      })
                    }
                  >
                    Revoke Machine permissions
                  </Button>
                )}
                <Button
                  variant="outline"
                  onClick={() => {
                    setEditing(false);
                    setError(undefined);
                    void query.refetch().then(() => setNeedsReview(false));
                  }}
                >
                  Reload permissions
                </Button>
              </div>
            </>
          )}
        </>
        {error && <ErrorBanner message={error} />}
        {editing && canEdit && (
          <MachineGrantEditor
            key={`${grant?.id ?? "new"}-${grant?.revision ?? 0}`}
            initial={selection}
            onSubmit={submit}
            submitting={save.isPending}
            allowedNodeIds={effectiveNodeIds}
            allowedServiceIds={effectiveServiceIds}
          />
        )}
        <Dialog
          open={revokeReview !== null}
          onOpenChange={(open) => {
            if (!open) setRevokeReview(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Revoke Machine permissions?</DialogTitle>
              <DialogDescription>
                This stops work authorized by this grant across all its
                Machines. Other service access remains unchanged. The Machine
                and workspace are retained.
              </DialogDescription>
            </DialogHeader>
            {revokeReview && !revokeReviewCurrent && (
              <p role="alert" className="text-12 text-warning">
                Machine permissions changed since this review. Close this
                dialog, reload, and review the current grant before revoking.
              </p>
            )}
            <DialogFooter>
              <Button variant="outline" onClick={() => setRevokeReview(null)}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                disabled={
                  Boolean(query.error) ||
                  query.isFetching ||
                  !revokeReviewCurrent ||
                  revoke.isPending
                }
                isLoading={revoke.isPending}
                onClick={() => {
                  if (
                    query.error ||
                    query.isFetching ||
                    !revokeReviewCurrent ||
                    !revokeReview
                  )
                    return;
                  setError(undefined);
                  void revoke
                    .mutateAsync({
                      expected_grant_id: revokeReview.id,
                      expected_revision: revokeReview.revision,
                    })
                    .then(() => {
                      setRevokeReview(null);
                      setEditing(false);
                    })
                    .catch((err: unknown) => {
                      if (err instanceof ApiError && err.status === 409) {
                        setNeedsReview(true);
                        setEditing(false);
                      }
                      setError(
                        err instanceof ApiError && err.status === 409
                          ? "Machine permissions changed. Reload and review before revoking."
                          : err instanceof Error
                            ? err.message
                            : "Could not revoke Machine permissions.",
                      );
                      setRevokeReview(null);
                    });
                }}
              >
                Revoke Machine permissions
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
        <Dialog
          open={removeReview !== null}
          onOpenChange={(open) => {
            if (!open) {
              setRemoveReview(null);
              setRemoveLegacyAccepted(false);
            }
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>
                {grant && Object.keys(grant.assignments).length === 1
                  ? "Remove the last Machine and revoke this source grant?"
                  : "Remove this Machine from the source grant?"}
              </DialogTitle>
              <DialogDescription>
                {grant && Object.keys(grant.assignments).length === 1
                  ? "This source is currently assigned only to this Machine. The complete source Machine grant will be revoked. The parent Key or OAuth client and its independent service access remain unchanged."
                  : "This removes the current Machine from the source grant and submits the complete remaining assignment map. Other Machines keep their current mappings; the parent Key or OAuth client remains."}
              </DialogDescription>
            </DialogHeader>
            {removeReview && !removeReviewCurrent ? (
              <p role="alert" className="text-12 text-warning">
                The source grant changed since this review. Close this dialog,
                reload, and review the current assignments before removing the
                Machine.
              </p>
            ) : null}
            <div className="space-y-1 rounded-lg border border-border p-3 text-12">
              <p className="font-medium">Affected assignments</p>
              {grant
                ? Object.keys(grant.assignments).map((nodeId) => (
                    <p
                      key={nodeId}
                      className={
                        nodeId === machineId
                          ? "font-medium text-warning"
                          : "text-muted-foreground"
                      }
                    >
                      {nodeId}
                      {nodeId === machineId
                        ? " · current Machine"
                        : " · retained"}
                    </p>
                  ))
                : null}
            </div>
            {grant && Object.keys(grant.assignments).length > 1 ? (
              <div className="space-y-2 text-12">
                <p>{SHARED_LEGACY_WARNING}</p>
                <label className="flex items-start gap-2">
                  <Checkbox
                    checked={removeLegacyAccepted}
                    onCheckedChange={(checked) =>
                      setRemoveLegacyAccepted(checked === true)
                    }
                  />
                  I explicitly allow shared legacy access for the retained
                  Machines and acknowledge this warning.
                </label>
              </div>
            ) : null}
            <DialogFooter>
              <Button variant="outline" onClick={() => setRemoveReview(null)}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                disabled={
                  !removeReviewCurrent ||
                  (Object.keys(grant?.assignments ?? {}).length > 1 &&
                    !removeLegacyAccepted) ||
                  query.isFetching ||
                  save.isPending ||
                  revoke.isPending
                }
                isLoading={save.isPending || revoke.isPending}
                onClick={() => void removeCurrentMachine()}
              >
                {grant && Object.keys(grant.assignments).length === 1
                  ? "Remove and revoke source grant"
                  : "Remove current Machine"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  );
}
