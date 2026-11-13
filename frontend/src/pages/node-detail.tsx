import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Label } from "@/components/ui/label";
import { MachineSummary } from "@/components/shared/machine-summary";
import { useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
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
import {
  pushNodeCredentialSchema,
  type PushNodeCredentialFormData,
  type PushNodeCredentialFormInput,
} from "@/schemas/nodes";
import { formatDate, formatRelativeTime } from "@/lib/utils";
import { buildStandaloneCredentialAcceptUrl } from "@/lib/credential-accept-url";
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
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
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
import type { NodePendingCredentialInjectionMethod } from "@/types/nodes";
import {
  adminDisplayName,
  canManageNode,
  defaultFieldNameForMethod,
  injectionMethodLabel,
  keyOwnerId,
  nodeOwnerLabel,
} from "./node-detail.helpers";

const NODE_CREDENTIAL_PUSH_DEFAULT_VALUES = {
  service_slug: "",
  injection_method: "header",
  field_name: "X-API-Key",
  target_url: "",
  label: "",
  remote_crypto: true,
} satisfies PushNodeCredentialFormInput;

export function NodeDetailPage() {
  useUiTranslation();
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
  const credentialPushForm = useAppForm<
    PushNodeCredentialFormInput,
    unknown,
    PushNodeCredentialFormData
  >({
    resolver: zodResolver(pushNodeCredentialSchema),
    defaultValues: NODE_CREDENTIAL_PUSH_DEFAULT_VALUES,
  });
  const watchedCredentialSlug = credentialPushForm.watch("service_slug");
  const watchedCredentialFieldName = credentialPushForm.watch("field_name");

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
      toast.success(translateUi("Node deleted"));
      void navigate({ to: "/nodes" });
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to delete node"),
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
      toast.success(translateUi("Node credentials rotated"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to rotate token"),
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
        translateUi("Node transferred to {{value1}}", { value1: nodeOwnerLabel(result.new_owner, currentUserId) }),
      );
      setShowTransferDialog(false);
      setTransferOwnerId(null);
      setTransferConfirmed(false);
      void navigate({ to: "/nodes" });
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to transfer node"),
      );
    }
  }

  async function handlePushCredential(data: PushNodeCredentialFormData) {
    try {
      const created = await pushCredentialMutation.mutateAsync(data);
      toast.success(translateUi("Credential push created"));
      credentialPushForm.reset(NODE_CREDENTIAL_PUSH_DEFAULT_VALUES);
      window.location.assign(
        await buildStandaloneCredentialAcceptUrl(
          nodeId,
          created.id,
          `/nodes/${nodeId}`,
        ),
      );
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to push credential"),
      );
    }
  }

  async function handleCancelPendingCredential(pendingCredentialId: string) {
    try {
      await cancelPendingCredentialMutation.mutateAsync(pendingCredentialId);
      toast.success(translateUi("Pending credential canceled"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError
          ? err.message
          : "Failed to cancel pending credential"),
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
        <PageHeader title={translateUi("Node Not Found")} />
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : translateUi("Node not found or failed to load.")
          }
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
            ? translateUi("Manage node settings and credentials.")
            : translateUi("View node status and metrics.")
        }
        actions={
          canManage ? (
            <div className="flex gap-2">
              <Button
                variant="outline"
                className="text-text-tertiary hover:text-muted-foreground"
                onClick={() => {
                  setTransferOwnerId(null);
                  setTransferConfirmed(false);
                  setShowTransferDialog(true);
                }}
              >
                <ButtonIcon>
                  <ArrowRightLeft className="h-3 w-3" />
                </ButtonIcon>{translateUi("Transfer")}</Button>
              <Button
                variant="outline"
                className="text-text-tertiary hover:text-muted-foreground"
                onClick={() => setShowRotateDialog(true)}
              >
                <ButtonIcon>
                  <KeyRound className="h-3 w-3" />
                </ButtonIcon>{translateUi("Rotate Credentials")}</Button>
              <Button
                variant="destructive"
                onClick={() => setShowDeleteDialog(true)}
              >
                <ButtonIcon variant="destructive">
                  <Trash2 className="h-3 w-3 text-destructive" />
                </ButtonIcon>{translateUi("Delete")}</Button>
            </div>
          ) : undefined
        }
      />

      {node.machine ? <MachineSummary node={node} /> : null}
      {/* Node Info */}
      <DetailSection title={translateUi("Node Information")}>
        <DetailRow
          label={translateUi("Owner")}
          value={nodeOwnerLabel(node.owner, currentUserId)}
        />
        <div className="flex items-center justify-between px-5 py-3 text-15">
          <span className="text-muted-foreground">{translateUi("Status")}</span>
          <NodeStatusBadge
            status={node.status}
            isConnected={node.is_connected}
          />
        </div>
        <DetailRow label={translateUi("Created")} value={formatDate(node.created_at)} />
        <DetailRow
          label={translateUi("Last Heartbeat")}
          value={formatRelativeTime(node.last_heartbeat_at)}
        />
        {node.dispatch && (
          <DetailRow
            label={translateUi("Proxy Dispatch")}
            value={
              node.dispatch.dispatchable
                ? "Ready"
                : `Blocked: ${node.dispatch.reason}`
            }
          />
        )}
        {node.capabilities && (
          <DetailRow
            label={translateUi("Capabilities")}
            value={
              node.capabilities_resolved === false
                ? "Pending status update"
                : [
                    node.capabilities.credential_ack_correlation
                      ? "credential ack correlation"
                      : null,
                    node.capabilities.remote_credential_crypto_v1
                      ? "remote credential crypto v1"
                      : null,
                  ]
                    .filter(Boolean)
                    .join(", ") || "None advertised"
            }
          />
        )}
        {node.connected_at && (
          <DetailRow
            label={translateUi("Connected Since")}
            value={formatRelativeTime(node.connected_at)}
          />
        )}
        {node.metadata?.agent_version && (
          <DetailRow
            label={translateUi("Agent Version")}
            value={node.metadata.agent_version}
          />
        )}
        {node.metadata?.os && (
          <DetailRow
            label={translateUi("OS")}
            value={`${node.metadata.os}${node.metadata.arch ? ` (${node.metadata.arch})` : ""}`}
          />
        )}
        {node.metadata?.ip_address && (
          <DetailRow label={translateUi("IP Address")} value={node.metadata.ip_address} />
        )}
      </DetailSection>

      {/* Shared with */}
      <DetailSection title={translateUi("Shared with")}>
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
                className="flex items-center justify-between px-5 py-3 text-15"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <Users className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate text-foreground font-medium">
                    {adminDisplayName(admin, currentUserId)}
                  </span>
                </div>
                <Badge variant="secondary">
                  {admin.role === "owner" ? translateUi("Owner") : translateUi("Admin")}
                </Badge>
              </div>
            ))}
          </>
        ) : (
          <p className="px-5 py-3 text-14 text-muted-foreground">{translateUi("No admins are currently listed for this node.")}</p>
        )}
      </DetailSection>

      {/* Metrics */}
      <DetailSection title={translateUi("Metrics")}>
        {node.metrics && node.metrics.total_requests > 0 ? (
          <>
            <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,12rem),1fr))] gap-3 p-4">
              <div className="rounded-xl border border-border/50 bg-overlay p-4 text-center">
                <p
                  className="break-words type-metric text-foreground"
                  style={{ letterSpacing: "-0.02em" }}
                >
                  {String(node.metrics.total_requests)}
                </p>
                <p className="text-13 text-muted-foreground mt-1">{translateUi("Total Requests")}</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-overlay p-4 text-center">
                <p
                  className="break-words type-metric text-foreground"
                  style={{ letterSpacing: "-0.02em" }}
                >
                  {(node.metrics.success_rate * 100).toFixed(1)}%
                </p>
                <p className="text-13 text-muted-foreground mt-1">{translateUi("Success Rate")}</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-overlay p-4 text-center">
                <p
                  className="break-words type-metric text-foreground"
                  style={{ letterSpacing: "-0.02em" }}
                >
                  {node.metrics.avg_latency_ms.toFixed(0)}{translateUi("ms")}</p>
                <p className="text-13 text-muted-foreground mt-1">{translateUi("Avg Latency")}</p>
              </div>
              <div className="rounded-xl border border-border/50 bg-overlay p-4 text-center">
                <p
                  className="break-words type-metric text-foreground"
                  style={{ letterSpacing: "-0.02em" }}
                >
                  {String(node.metrics.error_count)}
                </p>
                <p className="text-13 text-muted-foreground mt-1">{translateUi("Errors")}</p>
              </div>
            </div>
            {node.metrics.last_error && (
              <div className="mx-4 mb-4 rounded-xl bg-destructive/10 p-4">
                <div className="flex items-center gap-2 type-secondary-emphasis text-destructive">
                  <Activity className="h-3 w-3" />{translateUi("Last Error")}{node.metrics.last_error_at && (
                    <span className="font-normal text-muted-foreground">
                      {formatRelativeTime(node.metrics.last_error_at)}
                    </span>
                  )}
                </div>
                <p className="mt-1 text-13 text-destructive/80">
                  {node.metrics.last_error}
                </p>
              </div>
            )}
            {node.metrics.last_success_at && (
              <DetailRow
                label={translateUi("Last Successful Request")}
                value={
                  formatRelativeTime(node.metrics.last_success_at) ?? "Never"
                }
              />
            )}
          </>
        ) : (
          <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
            <SolarPanelIcon className="h-48 w-48 text-muted-foreground" />
            <p className="text-14 text-muted-foreground">{translateUi("No metrics recorded yet. Metrics will appear after the first proxy request.")}</p>
          </div>
        )}
      </DetailSection>

      {canManage && (
        <div className="space-y-6">
          <DetailSection title={translateUi("Push Credential to Node")}>
            <Form {...credentialPushForm}>
              <form
                className="p-5 space-y-4"
                onSubmit={(event) =>
                  void credentialPushForm.handleSubmit(handlePushCredential)(
                    event,
                  )
                }
              >
              <p className="text-14 text-muted-foreground">{translateUi("Create pending credential metadata, then enter the secret on the accept page for browser-side encryption.")}</p>
              <div className="grid gap-4 md:grid-cols-2">
                <FormField
                  control={credentialPushForm.control}
                  name="service_slug"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Service slug")}</FormLabel>
                      <FormControl>
                        <Input {...field} placeholder="openclaw" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={credentialPushForm.control}
                  name="field_name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Field name")}</FormLabel>
                      <FormControl>
                        <Input readOnly={credentialPushForm.watch("injection_method") === "ifttt-webhook"} {...field} placeholder="X-API-Key" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={credentialPushForm.control}
                  name="injection_method"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Injection method")}</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={(value) => {
                          const method =
                            value as NodePendingCredentialInjectionMethod;
                          const previousDefault = defaultFieldNameForMethod(
                            field.value,
                          );
                          const currentFieldName =
                            credentialPushForm.getValues("field_name");
                          field.onChange(method);
                          if (
                            method === "ifttt-webhook" ||
                            currentFieldName.trim() === "" ||
                            currentFieldName === previousDefault
                          ) {
                            credentialPushForm.setValue(
                              "field_name",
                              defaultFieldNameForMethod(method),
                            );
                          }
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="header">{translateUi("Header")}</SelectItem>
                          <SelectItem value="query-param">{translateUi("Query param")}</SelectItem>
                          <SelectItem value="path-prefix">{translateUi("Path prefix")}</SelectItem>
                          <SelectItem value="ifttt-webhook">{translateUi("IFTTT Webhooks")}</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={credentialPushForm.control}
                  name="target_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Target URL")}</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          value={field.value ?? ""}
                          placeholder="https://gateway.example.com/v1"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={credentialPushForm.control}
                  name="label"
                  render={({ field }) => (
                    <FormItem className="md:col-span-2">
                      <FormLabel>{translateUi("Label")}</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          value={field.value ?? ""}
                          placeholder={translateUi("Production gateway")}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <div className="flex justify-end">
                <Button
                  variant="primary"
                  type="submit"
                  disabled={
                    !watchedCredentialSlug.trim() ||
                    !watchedCredentialFieldName.trim()
                  }
                  isLoading={pushCredentialMutation.isPending}
                >
                  <ButtonIcon variant="primary">
                    <Send className="h-3 w-3" />
                  </ButtonIcon>{translateUi("Push")}</Button>
              </div>
              </form>
            </Form>
          </DetailSection>

          <DetailSection title={translateUi("Pending Credentials")}>
            {pendingCredentialsLoading ? (
              <div className="px-5 py-3 space-y-2">
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-2/3" />
              </div>
            ) : !pendingCredentials || pendingCredentials.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
                <SwitchIcon className="h-48 w-48 text-muted-foreground" />
                <p className="text-14 text-muted-foreground">{translateUi("No pending credentials are waiting for this node.")}</p>
              </div>
            ) : (
              <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{translateUi("Slug")}</TableHead>
                      <TableHead>{translateUi("Method")}</TableHead>
                      <TableHead>{translateUi("Field")}</TableHead>
                      <TableHead>{translateUi("Age")}</TableHead>
                      <TableHead>{translateUi("Target")}</TableHead>
                      <TableHead className="w-[96px]">{translateUi("Actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {pendingCredentials.map((credential) => (
                      <TableRow key={credential.id}>
                        <TableCell>
                          <div className="space-y-1">
                            <code className="text-14">
                              {credential.service_slug}
                            </code>
                            {credential.label && (
                              <p className="text-13 text-muted-foreground">
                                {credential.label}
                              </p>
                            )}
                          </div>
                        </TableCell>
                        <TableCell>
                          {injectionMethodLabel(credential.injection_method)}
                        </TableCell>
                        <TableCell>
                          <code className="text-14 text-muted-foreground">
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
                          <div className="flex items-center gap-1">
                            <Button
                              variant="ghost"
                              className="text-muted-foreground hover:text-foreground"
                              onClick={() => {
                                void buildStandaloneCredentialAcceptUrl(
                                  nodeId,
                                  credential.id,
                                  `/nodes/${nodeId}`,
                                ).then((url) => window.location.assign(url));
                              }}
                            >{translateUi("Accept")}</Button>
                            <Button
                              variant="ghost"
                              className="text-muted-foreground hover:text-destructive"
                              onClick={() =>
                                void handleCancelPendingCredential(
                                  credential.id,
                                )
                              }
                              isLoading={
                                cancelPendingCredentialMutation.isPending
                              }
                            >{translateUi("Cancel")}</Button>
                          </div>
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
            <DialogTitle>{translateUi("Transfer Ownership")}</DialogTitle>
            <DialogDescription>{translateUi("Move \"")}{node.name}{translateUi("\" to another owner. Existing node credentials keep working, but service routing is detached where ownership no longer matches.")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <p className="type-row-title text-foreground">{translateUi("Destination owner")}</p>
              <OrgScopeSelect
                value={transferOwnerId}
                onChange={(value) => {
                  setTransferOwnerId(value);
                  setTransferConfirmed(false);
                }}
                label={translateUi("Destination owner")}
              />
            </div>
            <div className="rounded-lg border border-border bg-muted/40 p-3 text-14">
              <p className="font-medium text-foreground">{translateUi("Transfer preview")}</p>
              <ul className="mt-2 space-y-1 text-muted-foreground">
                <li>
                  {String(transferServiceDetachCount)}{translateUi(" AI Services will lose their node routing.")}</li>
              </ul>
              {transferIsNoop && (
                <p className="mt-2 text-13 text-destructive">{translateUi("Choose a different owner before transferring.")}</p>
              )}
            </div>
            <Label className="flex items-start gap-2 text-muted-foreground">
              <Checkbox
                checked={transferConfirmed}
                onCheckedChange={(checked) =>
                  setTransferConfirmed(checked === true)
                }
              />
              <span>{translateUi("I understand that cross-owner AI Services will stop routing through this node.")}</span>
            </Label>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setShowTransferDialog(false);
                setTransferOwnerId(null);
                setTransferConfirmed(false);
              }}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="primary"
              onClick={() => void handleTransferNode()}
              disabled={
                !transferConfirmed || transferIsNoop || !transferTargetOwnerId
              }
              isLoading={transferMutation.isPending}
            >{translateUi("Transfer")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Delete Node")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to delete \"")}{node.name}{translateUi("\"? This will disconnect the node and detach any AI Services routed through it. This action cannot be undone.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowDeleteDialog(false)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >{translateUi("Delete")}</Button>
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
                <DialogTitle>{translateUi("Node Credentials Rotated")}</DialogTitle>
                <DialogDescription>{translateUi("Copy both values now. The old credentials have been invalidated immediately, and these secrets will not be shown again.")}</DialogDescription>
              </DialogHeader>
              <div className="space-y-3">
                <CopyableField
                  label={translateUi("New Auth Token")}
                  value={rotatedCredentials.auth_token}
                />
                <CopyableField
                  label={translateUi("New Signing Secret")}
                  value={rotatedCredentials.signing_secret}
                />
                <div className="rounded-lg bg-muted p-3">
                  <p className="mb-1 type-secondary-emphasis text-text-tertiary">{translateUi("Run on your node")}</p>
                  <code className="text-14 text-foreground break-all">
                    nyxid node rekey --auth-token{" "}
                    {rotatedCredentials.auth_token} --signing-secret{" "}
                    {rotatedCredentials.signing_secret}
                  </code>
                </div>
                <p className="text-13 text-muted-foreground">
                  <strong>{translateUi("Lost it?")}</strong>{translateUi(" If you don't copy this now, you'll need to rotate the credential to get a new one. (Old value stops working immediately on rotate.)")}</p>
              </div>
              <DialogFooter>
                <Button
                  variant="primary"
                  onClick={() => {
                    setShowRotateDialog(false);
                    setRotatedCredentials(null);
                  }}
                >{translateUi("Done")}</Button>
              </DialogFooter>
            </>
          ) : (
            <>
              <DialogHeader>
                <DialogTitle>{translateUi("Rotate Node Credentials")}</DialogTitle>
                <DialogDescription>{translateUi("This will generate a new auth token and signing secret, then invalidate the current credentials immediately. The node must be updated and restarted with the new values.")}</DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button
                  variant="outline"
                  onClick={() => setShowRotateDialog(false)}
                >{translateUi("Cancel")}</Button>
                <Button
                  variant="primary"
                  onClick={() => void handleRotateToken()}
                  isLoading={rotateMutation.isPending}
                >{translateUi("Rotate Token")}</Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
