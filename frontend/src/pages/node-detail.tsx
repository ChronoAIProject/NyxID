import { useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import {
  useNode,
  useNodeAdmins,
  useNodePendingCredentials,
  useDeleteNode,
  useRotateNodeToken,
  useTransferNode,
  usePushNodeCredential,
  useCancelNodePendingCredential,
} from "@/hooks/use-nodes";
import { useKeys } from "@/hooks/use-keys";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { pushNodeCredentialSchema } from "@/schemas/nodes";
import { formatDate, formatRelativeTime } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { CopyableField } from "@/components/shared/copyable-field";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Activity,
  ArrowRightLeft,
  KeyRound,
  Send,
  Trash2,
  Users,
} from "lucide-react";
import { SolarPanelIcon, SwitchIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import type {
  NodeAdminInfo,
  NodeInfo,
  NodePendingCredentialInjectionMethod,
} from "@/types/nodes";
import type { KeyInfo } from "@/types/keys";

function nodeOwnerLabel(
  owner: NodeInfo["owner"],
  currentUserId: string | null,
): string {
  if (owner.kind === "user" && owner.id === currentUserId) {
    return "You";
  }
  return owner.display_name;
}

function adminDisplayName(admin: NodeAdminInfo, currentUserId: string | null) {
  if (admin.user_id === currentUserId) {
    return "You";
  }
  return admin.display_name ?? admin.email ?? admin.user_id;
}

function canManageNode(
  node: NodeInfo | undefined,
  currentUserId: string | null,
  admins: readonly NodeAdminInfo[] | undefined,
): boolean {
  if (!node || !currentUserId) {
    return false;
  }
  if (node.owner.kind === "user") {
    return node.owner.id === currentUserId;
  }
  return (admins ?? []).some((admin) => admin.user_id === currentUserId);
}

function keyOwnerId(key: KeyInfo, currentUserId: string | null): string | null {
  const source = key.credential_source;
  if (!source || source.type === "personal") {
    return currentUserId;
  }
  return source.org_id;
}

function injectionMethodLabel(
  method: NodePendingCredentialInjectionMethod,
): string {
  switch (method) {
    case "query-param":
      return "Query param";
    case "path-prefix":
      return "Path prefix";
    case "header":
      return "Header";
  }
}

function defaultFieldNameForMethod(
  method: NodePendingCredentialInjectionMethod,
): string {
  switch (method) {
    case "query-param":
      return "api_key";
    case "path-prefix":
      return "api";
    case "header":
      return "X-API-Key";
  }
}

export function NodeDetailPage() {
  const { nodeId } = useParams({ strict: false }) as { nodeId: string };
  const navigate = useNavigate();

  const { data: node, isLoading, error, refetch } = useNode(nodeId);
  const { data: admins, isLoading: adminsLoading } = useNodeAdmins(nodeId);
  const { data: keys } = useKeys();
  const currentUserId = useAuthStore((state) => state.user?.id ?? null);

  const deleteMutation = useDeleteNode();
  const rotateMutation = useRotateNodeToken();
  const transferMutation = useTransferNode();
  const pushCredentialMutation = usePushNodeCredential(nodeId);
  const cancelPendingCredentialMutation =
    useCancelNodePendingCredential(nodeId);

  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showRotateDialog, setShowRotateDialog] = useState(false);
  const [showTransferDialog, setShowTransferDialog] = useState(false);
  const [transferOwnerId, setTransferOwnerId] = useState<string | null>(null);
  const [transferConfirmed, setTransferConfirmed] = useState(false);
  const [rotatedCredentials, setRotatedCredentials] = useState<{
    readonly auth_token: string;
    readonly signing_secret: string;
  } | null>(null);
  const [credentialSlug, setCredentialSlug] = useState("");
  const [credentialInjectionMethod, setCredentialInjectionMethod] =
    useState<NodePendingCredentialInjectionMethod>("header");
  const [credentialFieldName, setCredentialFieldName] = useState("X-API-Key");
  const [credentialTargetUrl, setCredentialTargetUrl] = useState("");
  const [credentialLabel, setCredentialLabel] = useState("");

  useBreadcrumbLabel(node?.name);
  const canManage = canManageNode(node, currentUserId, admins);
  const { data: pendingCredentials, isLoading: pendingCredentialsLoading } =
    useNodePendingCredentials(nodeId, canManage);
  const transferTargetOwnerId = transferOwnerId ?? currentUserId ?? "";
  const transferIsNoop =
    Boolean(node) && node?.owner.id === transferTargetOwnerId;
  const transferServiceDetachCount =
    node && transferTargetOwnerId
      ? (keys ?? []).filter(
          (key) =>
            key.node_id === node.id &&
            keyOwnerId(key, currentUserId) !== transferTargetOwnerId,
        ).length
      : 0;

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(nodeId);
      toast.success("Node deleted");
      void navigate({ to: "/nodes" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete node",
      );
    } finally {
      setShowDeleteDialog(false);
    }
  }

  async function handleRotateToken() {
    try {
      const result = await rotateMutation.mutateAsync(nodeId);
      setRotatedCredentials({
        auth_token: result.auth_token,
        signing_secret: result.signing_secret,
      });
      toast.success("Node credentials rotated");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to rotate token",
      );
      setShowRotateDialog(false);
    }
  }

  async function handleTransferNode() {
    if (!node || !transferTargetOwnerId) return;
    try {
      const result = await transferMutation.mutateAsync({
        nodeId,
        data: { new_owner_user_id: transferTargetOwnerId },
      });
      toast.success(
        `Node transferred to ${nodeOwnerLabel(result.new_owner, currentUserId)}`,
      );
      setShowTransferDialog(false);
      setTransferOwnerId(null);
      setTransferConfirmed(false);
      void navigate({ to: "/nodes" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to transfer node",
      );
    }
  }

  async function handlePushCredential() {
    const parsed = pushNodeCredentialSchema.safeParse({
      service_slug: credentialSlug,
      injection_method: credentialInjectionMethod,
      field_name: credentialFieldName,
      target_url: credentialTargetUrl,
      label: credentialLabel,
    });

    if (!parsed.success) {
      toast.error(parsed.error.issues[0]?.message ?? "Invalid credential push");
      return;
    }

    try {
      await pushCredentialMutation.mutateAsync(parsed.data);
      toast.success("Credential push created");
      setCredentialSlug("");
      setCredentialInjectionMethod("header");
      setCredentialFieldName("X-API-Key");
      setCredentialTargetUrl("");
      setCredentialLabel("");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to push credential",
      );
    }
  }

  async function handleCancelPendingCredential(pendingCredentialId: string) {
    try {
      await cancelPendingCredentialMutation.mutateAsync(pendingCredentialId);
      toast.success("Pending credential canceled");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to cancel pending credential",
      );
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-8">
        <Skeleton className="h-12 w-64" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !node) {
    return (
      <div className="space-y-8">
        <PageHeader title="Node Not Found" />
        <ErrorBanner
          message={error instanceof ApiError ? error.message : "Node not found or failed to load."}
          onRetry={refetch}
        />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={node.name}
        description={
          canManage
            ? "Manage node settings and credentials."
            : "View node status and metrics."
        }
        actions={
          canManage ? (
            <div className="flex gap-2">
              <Button
                variant="outline"
                onClick={() => {
                  setTransferOwnerId(null);
                  setTransferConfirmed(false);
                  setShowTransferDialog(true);
                }}
              >
                <ButtonIcon><ArrowRightLeft className="h-3 w-3" /></ButtonIcon>
                Transfer
              </Button>
              <Button
                variant="outline"
                onClick={() => setShowRotateDialog(true)}
              >
                <ButtonIcon><KeyRound className="h-3 w-3" /></ButtonIcon>
                Rotate Credentials
              </Button>
              <Button
                variant="destructive"
                onClick={() => setShowDeleteDialog(true)}
              >
                <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>
                Delete
              </Button>
            </div>
          ) : undefined
        }
      />

      {/* Node Info */}
      <DetailSection title="Node Information">
        <DetailRow
          label="Owner"
          value={nodeOwnerLabel(node.owner, currentUserId)}
        />
        <div className="flex items-center justify-between px-5 py-3 text-[13px]">
          <span className="text-muted-foreground">Status</span>
          <NodeStatusBadge
            status={node.status}
            isConnected={node.is_connected}
          />
        </div>
        <DetailRow label="Created" value={formatDate(node.created_at)} />
        <DetailRow
          label="Last Heartbeat"
          value={formatRelativeTime(node.last_heartbeat_at)}
        />
        {node.connected_at && (
          <DetailRow
            label="Connected Since"
            value={formatRelativeTime(node.connected_at)}
          />
        )}
        {node.metadata?.agent_version && (
          <DetailRow
            label="Agent Version"
            value={node.metadata.agent_version}
          />
        )}
        {node.metadata?.os && (
          <DetailRow
            label="OS"
            value={`${node.metadata.os}${node.metadata.arch ? ` (${node.metadata.arch})` : ""}`}
          />
        )}
        {node.metadata?.ip_address && (
          <DetailRow label="IP Address" value={node.metadata.ip_address} />
        )}
      </DetailSection>

      {/* Shared with */}
      <DetailSection title="Shared with">
        {adminsLoading ? (
          <div className="px-5 py-3 space-y-2">
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-2/3" />
          </div>
        ) : admins && admins.length > 0 ? (
          <>
            {admins.map((admin) => (
              <div
                key={admin.user_id}
                className="flex items-center justify-between px-5 py-3 text-[13px]"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <Users className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate text-foreground font-medium">
                    {adminDisplayName(admin, currentUserId)}
                  </span>
                </div>
                <Badge variant="secondary">
                  {admin.role === "owner" ? "Owner" : "Admin"}
                </Badge>
              </div>
            ))}
          </>
        ) : (
          <p className="px-5 py-3 text-[12px] text-muted-foreground">
            No admins are currently listed for this node.
          </p>
        )}
      </DetailSection>

      {/* Metrics */}
      <DetailSection title="Metrics">
        {node.metrics && node.metrics.total_requests > 0 ? (
          <>
            <div className="grid grid-cols-2 gap-3 p-4 sm:grid-cols-4">
              <div className="rounded-xl border border-border/50 bg-white/[0.02] p-4 text-center">
                <p className="text-[22px] font-bold text-foreground" style={{ letterSpacing: "-0.02em" }}>
                  {String(node.metrics.total_requests)}
                </p>
                <p className="text-[11px] text-muted-foreground mt-1">Total Requests</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-white/[0.02] p-4 text-center">
                <p className="text-[22px] font-bold text-foreground" style={{ letterSpacing: "-0.02em" }}>
                  {(node.metrics.success_rate * 100).toFixed(1)}%
                </p>
                <p className="text-[11px] text-muted-foreground mt-1">Success Rate</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-white/[0.02] p-4 text-center">
                <p className="text-[22px] font-bold text-foreground" style={{ letterSpacing: "-0.02em" }}>
                  {node.metrics.avg_latency_ms.toFixed(0)}ms
                </p>
                <p className="text-[11px] text-muted-foreground mt-1">Avg Latency</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-white/[0.02] p-4 text-center">
                <p className="text-[22px] font-bold text-foreground" style={{ letterSpacing: "-0.02em" }}>
                  {String(node.metrics.error_count)}
                </p>
                <p className="text-[11px] text-muted-foreground mt-1">Errors</p>
              </div>
            </div>
            {node.metrics.last_error && (
              <div className="mx-4 mb-4 rounded-xl bg-destructive/10 p-4">
                <div className="flex items-center gap-2 text-xs font-medium text-destructive">
                  <Activity className="h-3 w-3" />
                  Last Error
                  {node.metrics.last_error_at && (
                    <span className="font-normal text-muted-foreground">
                      {formatRelativeTime(node.metrics.last_error_at)}
                    </span>
                  )}
                </div>
                <p className="mt-1 text-xs text-destructive/80">
                  {node.metrics.last_error}
                </p>
              </div>
            )}
            {node.metrics.last_success_at && (
              <DetailRow
                label="Last Successful Request"
                value={
                  formatRelativeTime(node.metrics.last_success_at) ?? "Never"
                }
              />
            )}
          </>
        ) : (
          <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
            <SolarPanelIcon className="h-48 w-48 text-muted-foreground/30" />
            <p className="text-[12px] text-muted-foreground/30">
              No metrics recorded yet. Metrics will appear after the first proxy request.
            </p>
          </div>
        )}
      </DetailSection>

      {canManage && (
        <div className="space-y-6">
          <DetailSection title="Push Credential to Node">
            <div className="p-5 space-y-4">
              <p className="text-[12px] text-muted-foreground">
                The VM operator will be prompted for the secret value when they
                accept this on the VM. The secret never leaves the VM.
              </p>
              <div className="grid gap-4 md:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="credential-service-slug">Service slug</Label>
                  <Input
                    id="credential-service-slug"
                    value={credentialSlug}
                    onChange={(event) => setCredentialSlug(event.target.value)}
                    placeholder="openclaw"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="credential-field-name">Field name</Label>
                  <Input
                    id="credential-field-name"
                    value={credentialFieldName}
                    onChange={(event) =>
                      setCredentialFieldName(event.target.value)
                    }
                    placeholder="X-API-Key"
                  />
                </div>
                <div className="space-y-2">
                  <Label>Injection method</Label>
                  <Select
                    value={credentialInjectionMethod}
                    onValueChange={(value) => {
                      const method = value as NodePendingCredentialInjectionMethod;
                      const previousDefault = defaultFieldNameForMethod(
                        credentialInjectionMethod,
                      );
                      setCredentialInjectionMethod(method);
                      if (
                        credentialFieldName.trim() === "" ||
                        credentialFieldName === previousDefault
                      ) {
                        setCredentialFieldName(
                          defaultFieldNameForMethod(method),
                        );
                      }
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="header">Header</SelectItem>
                      <SelectItem value="query-param">Query param</SelectItem>
                      <SelectItem value="path-prefix">Path prefix</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="credential-target-url">Target URL</Label>
                  <Input
                    id="credential-target-url"
                    value={credentialTargetUrl}
                    onChange={(event) =>
                      setCredentialTargetUrl(event.target.value)
                    }
                    placeholder="https://gateway.example.com/v1"
                  />
                </div>
                <div className="space-y-2 md:col-span-2">
                  <Label htmlFor="credential-label">Label</Label>
                  <Input
                    id="credential-label"
                    value={credentialLabel}
                    onChange={(event) => setCredentialLabel(event.target.value)}
                    placeholder="Production gateway"
                  />
                </div>
              </div>
              <div className="flex justify-end">
                <Button
                  variant="primary"
                  onClick={() => void handlePushCredential()}
                  disabled={!credentialSlug.trim() || !credentialFieldName.trim()}
                  isLoading={pushCredentialMutation.isPending}
                >
                  <ButtonIcon variant="primary"><Send className="h-3 w-3" /></ButtonIcon>
                  Push
                </Button>
              </div>
            </div>
          </DetailSection>

          <DetailSection title="Pending Credentials">
            {pendingCredentialsLoading ? (
              <div className="px-5 py-3 space-y-2">
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-2/3" />
              </div>
            ) : !pendingCredentials || pendingCredentials.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
                <SwitchIcon className="h-48 w-48 text-muted-foreground/30" />
                <p className="text-[12px] text-muted-foreground/30">
                  No pending credentials are waiting for this node.
                </p>
              </div>
            ) : (
              <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Slug</TableHead>
                      <TableHead>Method</TableHead>
                      <TableHead>Field</TableHead>
                      <TableHead>Age</TableHead>
                      <TableHead>Target</TableHead>
                      <TableHead className="w-[96px]">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {pendingCredentials.map((credential) => (
                      <TableRow key={credential.id}>
                        <TableCell>
                          <div className="space-y-1">
                            <code className="text-xs">
                              {credential.service_slug}
                            </code>
                            {credential.label && (
                              <p className="text-xs text-muted-foreground">
                                {credential.label}
                              </p>
                            )}
                          </div>
                        </TableCell>
                        <TableCell>
                          {injectionMethodLabel(credential.injection_method)}
                        </TableCell>
                        <TableCell>
                          <code className="text-xs text-muted-foreground">
                            {credential.field_name}
                          </code>
                        </TableCell>
                        <TableCell>
                          {formatRelativeTime(credential.created_at)}
                        </TableCell>
                        <TableCell className="max-w-[240px] truncate text-muted-foreground">
                          {credential.target_url ?? "-"}
                        </TableCell>
                        <TableCell>
                          <Button
                            variant="ghost"
                            className="text-muted-foreground hover:text-destructive"
                            onClick={() =>
                              void handleCancelPendingCredential(credential.id)
                            }
                            isLoading={cancelPendingCredentialMutation.isPending}
                          >
                            Cancel
                          </Button>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </DetailSection>
        </div>
      )}

      {/* Transfer Ownership */}
      <Dialog
        open={showTransferDialog}
        onOpenChange={(open) => {
          setShowTransferDialog(open);
          if (!open) {
            setTransferOwnerId(null);
            setTransferConfirmed(false);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Transfer Ownership</DialogTitle>
            <DialogDescription>
              Move &quot;{node.name}&quot; to another owner. Existing node
              credentials keep working, but service routing is detached where
              ownership no longer matches.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <p className="text-[12px] font-medium text-foreground">
                Destination owner
              </p>
              <OrgScopeSelect
                value={transferOwnerId}
                onChange={(value) => {
                  setTransferOwnerId(value);
                  setTransferConfirmed(false);
                }}
                label="Destination owner"
              />
            </div>
            <div className="rounded-lg border border-border bg-muted/40 p-3 text-[12px]">
              <p className="font-medium text-foreground">Transfer preview</p>
              <ul className="mt-2 space-y-1 text-muted-foreground">
                <li>
                  {String(transferServiceDetachCount)} AI Services will lose
                  their node routing.
                </li>
              </ul>
              {transferIsNoop && (
                <p className="mt-2 text-xs text-destructive">
                  Choose a different owner before transferring.
                </p>
              )}
            </div>
            <label className="flex items-start gap-2 text-[12px] text-muted-foreground">
              <Checkbox
                checked={transferConfirmed}
                onCheckedChange={(checked) =>
                  setTransferConfirmed(checked === true)
                }
              />
              <span>
                I understand that cross-owner AI Services will stop routing
                through this node.
              </span>
            </label>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setShowTransferDialog(false);
                setTransferOwnerId(null);
                setTransferConfirmed(false);
              }}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleTransferNode()}
              disabled={
                !transferConfirmed || transferIsNoop || !transferTargetOwnerId
              }
              isLoading={transferMutation.isPending}
            >
              Transfer
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Node</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{node.name}&quot;? This will
              disconnect the node and detach any AI Services routed through it.
              This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowDeleteDialog(false)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Rotate Token Dialog */}
      <Dialog
        open={showRotateDialog}
        onOpenChange={(open) => {
          if (!open) {
            setShowRotateDialog(false);
            setRotatedCredentials(null);
          }
        }}
      >
        <DialogContent>
          {rotatedCredentials ? (
            <>
              <DialogHeader>
                <DialogTitle>Node Credentials Rotated</DialogTitle>
                <DialogDescription>
                  Copy both values now. The old credentials have been
                  invalidated immediately, and these secrets will not be shown
                  again.
                </DialogDescription>
              </DialogHeader>
              <div className="space-y-3">
                <CopyableField
                  label="New Auth Token"
                  value={rotatedCredentials.auth_token}
                />
                <CopyableField
                  label="New Signing Secret"
                  value={rotatedCredentials.signing_secret}
                />
                <div className="rounded-lg bg-muted p-3">
                  <p className="mb-1 text-xs font-medium text-text-tertiary">
                    Run on your node
                  </p>
                  <code className="text-xs text-foreground break-all">
                    nyxid node rekey --auth-token{" "}
                    {rotatedCredentials.auth_token} --signing-secret{" "}
                    {rotatedCredentials.signing_secret}
                  </code>
                </div>
              </div>
              <DialogFooter>
                <Button
                  variant="primary"
                  onClick={() => {
                    setShowRotateDialog(false);
                    setRotatedCredentials(null);
                  }}
                >
                  Done
                </Button>
              </DialogFooter>
            </>
          ) : (
            <>
              <DialogHeader>
                <DialogTitle>Rotate Node Credentials</DialogTitle>
                <DialogDescription>
                  This will generate a new auth token and signing secret, then
                  invalidate the current credentials immediately. The node must
                  be updated and restarted with the new values.
                </DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button
                  variant="outline"
                  onClick={() => setShowRotateDialog(false)}
                >
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  onClick={() => void handleRotateToken()}
                  isLoading={rotateMutation.isPending}
                >
                  Rotate Token
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
