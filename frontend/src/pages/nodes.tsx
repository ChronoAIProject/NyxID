import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { MachineIsolationBadge } from "@/components/shared/machine-isolation";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { ViewToggle, useViewMode } from "@/components/shared/view-toggle";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  createRegistrationTokenSchema,
  type CreateRegistrationTokenFormData,
} from "@/schemas/nodes";
import {
  useNodes,
  useCreateRegistrationToken,
  useDeleteNode,
} from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { formatRelativeTime } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { CopyableField } from "@/components/shared/copyable-field";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Trash2, Loader2 } from "lucide-react";
import { WifiRouterIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import type { NodeInfo } from "@/types/nodes";

function nodeOwnerLabel(
  owner: NodeInfo["owner"],
  currentUserId: string | null,
): string {
  if (owner.kind === "user" && owner.id === currentUserId) {
    return translateUi("You");
  }
  return owner.display_name;
}

function canManageNode(
  node: NodeInfo,
  currentUserId: string | null,
  adminOrgIds: ReadonlySet<string>,
): boolean {
  if (node.owner.kind === "user") {
    return node.owner.id === currentUserId;
  }
  return adminOrgIds.has(node.owner.id);
}

function RegisterNodeDialog({
  onRegistered,
}: {
  /**
   * Fired once the registration-token mutation has succeeded. NodesPage
   * uses this to start a 60s polling window for the new node so the user
   * is auto-navigated to its detail page as soon as it checks in.
   */
  readonly onRegistered?: () => void;
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const [createdToken, setCreatedToken] = useState<{
    readonly token: string;
    readonly name: string;
    readonly expires_at: string;
  } | null>(null);
  const createMutation = useCreateRegistrationToken();
  const { data: publicConfig } = usePublicConfig();

  // Show --url flag when the backend is not on localhost
  const nodeWsUrl = publicConfig?.node_ws_url ?? null;
  const isLocalhost =
    nodeWsUrl?.includes("localhost") || nodeWsUrl?.includes("127.0.0.1");
  const urlFlag = nodeWsUrl && !isLocalhost ? ` --url ${nodeWsUrl}` : "";

  const form = useAppForm<CreateRegistrationTokenFormData>({
    resolver: zodResolver(createRegistrationTokenSchema),
    defaultValues: { name: "", owner_user_id: null },
  });

  async function onSubmit(data: CreateRegistrationTokenFormData) {
    try {
      const result = await createMutation.mutateAsync(data);
      setCreatedToken({
        token: result.token,
        name: result.name,
        expires_at: result.expires_at,
      });
      toast.success(translateUi("Registration token created"));
      // Wake up NodesPage's polling window so the empty list refetches
      // every ~3s for 60s, auto-navigating once the node registers.
      onRegistered?.();
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error(translateErrorMessage("Failed to create registration token"));
      }
    }
  }

  function handleClose() {
    setOpen(false);
    setCreatedToken(null);
    form.reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => (o ? setOpen(true) : handleClose())}
    >
      <DialogTrigger asChild>
        <span>
          <AddCtaButton label={translateUi("Register Node")} onClick={() => setOpen(true)} />
        </span>
      </DialogTrigger>
      <DialogContent>
        {createdToken ? (
          <>
            <DialogHeader>
              <DialogTitle>{translateUi("Registration Token Created")}</DialogTitle>
              <DialogDescription>{translateUi("Copy the token and use it to register your node. This token will not be shown again and expires at")}{" "}
                {new Date(createdToken.expires_at).toLocaleString(getFormattingLocale())}.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <CopyableField
                label={translateUi("Registration Token")}
                value={createdToken.token}
              />
              <div className="rounded-lg bg-muted p-3 space-y-2">
                <p className="type-secondary-emphasis text-text-tertiary">{translateUi("Run on your node")}</p>
                <div>
                  <p className="text-12 text-muted-foreground mb-0.5">{translateUi("File-based storage (default, works on servers)")}</p>
                  <code className="text-14 text-foreground break-all">
                    nyxid node register --token {createdToken.token}
                    {urlFlag}
                  </code>
                </div>
                <div>
                  <p className="text-12 text-muted-foreground mb-0.5">{translateUi("OS keychain storage (macOS Keychain, Windows Credential Manager)")}</p>
                  <code className="text-14 text-foreground break-all">
                    nyxid node register --token {createdToken.token}
                    {urlFlag} --keychain
                  </code>
                </div>
              </div>
            </div>
            <DialogFooter>
              <Button variant="primary" onClick={handleClose}>{translateUi("Done")}</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>{translateUi("Register a New Node")}</DialogTitle>
              <DialogDescription>{translateUi("Create a registration token to connect a new credential node.")}</DialogDescription>
            </DialogHeader>
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-14 text-destructive">
                    {form.formState.errors.root.message}
                  </div>
                )}
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Node Name")}</FormLabel>
                      <FormControl>
                        <Input placeholder="my-home-server" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="owner_user_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Owner")}</FormLabel>
                      <FormControl>
                        <OrgScopeSelect
                          value={field.value ?? null}
                          onChange={field.onChange}
                          label={translateUi("Node owner")}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={handleClose}>{translateUi("Cancel")}</Button>
                  <Button variant="primary" type="submit" isLoading={createMutation.isPending} disabled={!form.formState.isValid || createMutation.isPending}>{translateUi("Create Token")}</Button>
                </DialogFooter>
              </form>
            </Form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function NodesPage() {
  useUiTranslation();
  const navigate = useNavigate();
  // Set to true when `RegisterNodeDialog` mint succeeds; stays true for
  // up to 60s (enforced by the timeout effect below) so the empty node
  // list polls every 3s for the newcomer. Cleared early when:
  //   - the first node appears (the auto-navigate effect below), OR
  //   - the 60s window elapses, OR
  //   - the component unmounts (the setTimeout is cleaned up).
  const [pollingActive, setPollingActive] = useState(false);
  // Track the node ids visible the last time the user was NOT polling, so
  // the auto-navigate effect can ignore pre-existing nodes and only jump
  // to a freshly-registered one. Captured once when the polling window opens.
  const [knownNodeIdsAtPollStart, setKnownNodeIdsAtPollStart] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const { data: nodes, isLoading, error, refetch } = useNodes({
    pollIntervalMs: pollingActive ? 3000 : 0,
  });
  const { data: orgs } = useOrgs();
  const currentUserId = useAuthStore((state) => state.user?.id ?? null);
  const deleteMutation = useDeleteNode();
  const [viewMode, setViewMode] = useViewMode("nodes");
  const [deleteTarget, setDeleteTarget] = useState<{
    readonly id: string;
    readonly name: string;
  } | null>(null);
  const adminOrgIds = new Set(
    (orgs ?? [])
      .filter((org) => org.your_role === "admin")
      .map((org) => org.id),
  );

  // 60s safety timer: never leave polling on indefinitely if no node ever
  // checks in (user may have run the wrong command, or the node may have
  // failed to start). Cleared early if a node appears (effect below) or on
  // unmount.
  useEffect(() => {
    if (!pollingActive) return;
    const timeoutId = window.setTimeout(() => {
      setPollingActive(false);
    }, 60_000);
    return () => window.clearTimeout(timeoutId);
  }, [pollingActive]);

  // Auto-navigate to the first freshly-registered node during the polling
  // window so the user lands on the confirmation page the moment the node
  // checks in. Filters out pre-existing nodes via `knownNodeIdsAtPollStart`.
  useEffect(() => {
    if (!pollingActive || !nodes || nodes.length === 0) return;
    const freshNode = nodes.find((n) => !knownNodeIdsAtPollStart.has(n.id));
    if (!freshNode) return;
    setPollingActive(false);
    void navigate({
      to: "/nodes/$nodeId",
      params: { nodeId: freshNode.id },
    });
  }, [pollingActive, nodes, knownNodeIdsAtPollStart, navigate]);

  function handleRegistered() {
    setKnownNodeIdsAtPollStart(new Set((nodes ?? []).map((n) => n.id)));
    setPollingActive(true);
  }

  async function handleDelete() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      toast.success(translateUi("Node \"{{value1}}\" deleted", { value1: deleteTarget.name }));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to delete node"),
      );
    } finally {
      setDeleteTarget(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={translateUi("Nodes")}
        description={translateUi("Manage credential routing and machines your agents may use.")}
        actions={
          <div className="flex items-center gap-3">
            <ViewToggle viewMode={viewMode} onViewModeChange={setViewMode} />
            <RegisterNodeDialog onRegistered={handleRegistered} />
          </div>
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={`node-skel-${String(i)}`} className="h-16 w-full" />
          ))}
        </div>
      ) : error ? (
        <ErrorBanner message={translateUi("Failed to load nodes. Please try again.")} onRetry={refetch} />
      ) : !nodes || nodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <WifiRouterIcon className="h-64 w-64 text-muted-foreground" />
          <div className="max-w-md space-y-1">
            {pollingActive ? (
              <>
                <div className="flex items-center justify-center gap-2">
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  <p className="type-row-title text-foreground">{translateUi("Waiting for your node to connect…")}</p>
                </div>
                <p className="text-14 text-muted-foreground">{translateUi("Run the registration command on the node. We'll take you to its page automatically.")}</p>
              </>
            ) : (
              <>
                <p className="type-row-title text-foreground">{translateUi("No Credential Nodes")}</p>
                <p className="text-14 text-muted-foreground">{translateUi("Create a registration token to get started.")}</p>
              </>
            )}
          </div>
        </div>
      ) : (
        <>
          {/* Mobile card view - always visible on mobile */}
          <div className="flex flex-col gap-3 md:hidden">
            {nodes.map((node) => (
              <div
                key={node.id}
                role="button"
                tabIndex={0}
                onClick={() => void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } })}
                onKeyDown={(e) => { if (e.key === "Enter") void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } }); }}
                className="relative rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-overlay cursor-pointer"
              >
                {canManageNode(node, currentUserId, adminOrgIds) && (
                  <div className="absolute right-3 top-3" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7"
                      onClick={() => setDeleteTarget({ id: node.id, name: node.name })}
                    >
                      <Trash2 className="h-3.5 w-3.5 text-destructive" />
                    </Button>
                  </div>
                )}
                <p data-machine-card-title="" className="pr-10 type-card-title text-foreground truncate">
                  {node.name}
                  {node.metadata?.agent_version && (
                    <span className="ml-2 type-secondary text-muted-foreground">
                      v{node.metadata.agent_version}
                    </span>
                  )}
                </p>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
                  <Badge variant="secondary">{nodeOwnerLabel(node.owner, currentUserId)}</Badge>
                  {node.machine ? <MachineIsolationBadge machine={node.machine} /> : null}
                </div>
                <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-13 text-muted-foreground">
                  <span>{formatRelativeTime(node.last_heartbeat_at) ?? translateUi("No heartbeat")}</span>
                  <span>{translateUi("Created ")}{formatRelativeTime(node.created_at)}</span>
                </div>
              </div>
            ))}
          </div>

          {/* Desktop grid view */}
          {viewMode === "grid" && (
            <div className="hidden md:grid gap-4 md:grid-cols-2 lg:grid-cols-3">
              {nodes.map((node) => (
                <Link
                  key={node.id}
                  to="/nodes/$nodeId"
                  params={{ nodeId: node.id }}
                  className="relative rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-overlay"
                >
                  {canManageNode(node, currentUserId, adminOrgIds) && (
                    <div className="absolute right-3 top-3" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={(e) => {
                          e.preventDefault();
                          setDeleteTarget({ id: node.id, name: node.name });
                        }}
                      >
                        <Trash2 className="h-3.5 w-3.5 text-destructive" />
                      </Button>
                    </div>
                  )}
                  <p data-machine-card-title="" className="pr-10 type-card-title text-foreground truncate">
                    {node.name}
                    {node.metadata?.agent_version && (
                      <span className="ml-2 type-secondary text-muted-foreground">
                        v{node.metadata.agent_version}
                      </span>
                    )}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
                    <Badge variant="secondary">{nodeOwnerLabel(node.owner, currentUserId)}</Badge>
                  {node.machine ? <MachineIsolationBadge machine={node.machine} /> : null}
                  </div>
                  <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-13 text-muted-foreground">
                    <span>{formatRelativeTime(node.last_heartbeat_at) ?? translateUi("No heartbeat")}</span>
                    <span>{translateUi("Created ")}{formatRelativeTime(node.created_at)}</span>
                  </div>
                </Link>
              ))}
            </div>
          )}

          {/* Desktop table view */}
          {viewMode === "table" && (
            <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{translateUi("Name")}</TableHead>
                    <TableHead>{translateUi("Owner")}</TableHead>
                    <TableHead>{translateUi("Status")}</TableHead>
                    <TableHead>{translateUi("Last Heartbeat")}</TableHead>
                    <TableHead>{translateUi("Created")}</TableHead>
                    {nodes.some((n) => canManageNode(n, currentUserId, adminOrgIds)) && (
                      <TableHead className="w-[80px]">{translateUi("Actions")}</TableHead>
                    )}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {nodes.map((node) => (
                    <TableRow
                      key={node.id}
                      className="cursor-pointer hover:bg-overlay"
                      onClick={() => void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } })}
                    >
                      <TableCell>
                        <span className="font-medium">
                          {node.name}
                        </span>
                        {node.metadata?.agent_version && (
                          <span className="ml-2 text-13 text-muted-foreground">
                            v{node.metadata.agent_version}
                          </span>
                        )}
                      </TableCell>
                      <TableCell>
                        <Badge variant="secondary">
                          {nodeOwnerLabel(node.owner, currentUserId)}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <NodeStatusBadge
                          status={node.status}
                          isConnected={node.is_connected}
                        />
                        {node.machine ? <MachineIsolationBadge machine={node.machine} /> : null}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {formatRelativeTime(node.last_heartbeat_at) ?? translateUi("Never")}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {formatRelativeTime(node.created_at)}
                      </TableCell>
                      {nodes.some((n) => canManageNode(n, currentUserId, adminOrgIds)) && (
                        <TableCell>
                          {canManageNode(node, currentUserId, adminOrgIds) && (
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-muted-foreground hover:text-destructive"
                              onClick={(e) => {
                                e.stopPropagation();
                                setDeleteTarget({ id: node.id, name: node.name });
                              }}
                            >
                              <Trash2 className="h-4 w-4 text-destructive" />
                              <span className="sr-only">{translateUi("Delete ")}{node.name}</span>
                            </Button>
                          )}
                        </TableCell>
                      )}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </>
      )}

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Delete Node")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to delete \"")}{deleteTarget?.name ?? ""}{translateUi("\"? This will disconnect the node and detach any AI Services routed through it. This action cannot be undone.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >{translateUi("Delete")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
