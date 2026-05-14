import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
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
import { Plus, Trash2 } from "lucide-react";
import { WifiRouterIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import type { NodeInfo } from "@/types/nodes";

function nodeOwnerLabel(
  owner: NodeInfo["owner"],
  currentUserId: string | null,
): string {
  if (owner.kind === "user" && owner.id === currentUserId) {
    return "You";
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

function RegisterNodeDialog() {
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

  const form = useForm<CreateRegistrationTokenFormData>({
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
      toast.success("Registration token created");
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create registration token");
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
        <button
          type="button"
          className="flex h-10 items-center gap-2.5 rounded-xl border border-white/[0.08] px-3 text-[13px] text-text-tertiary transition-all duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
        >
          <span className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] border border-white/[0.08] bg-white/[0.04]">
            <Plus className="h-3 w-3" />
          </span>
          Register Node
        </button>
      </DialogTrigger>
      <DialogContent>
        {createdToken ? (
          <>
            <DialogHeader>
              <DialogTitle>Registration Token Created</DialogTitle>
              <DialogDescription>
                Copy the token and use it to register your node. This token will
                not be shown again and expires at{" "}
                {new Date(createdToken.expires_at).toLocaleString()}.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <CopyableField
                label="Registration Token"
                value={createdToken.token}
              />
              <div className="rounded-lg bg-muted p-3 space-y-2">
                <p className="text-xs font-medium text-text-tertiary">
                  Run on your node
                </p>
                <div>
                  <p className="text-[10px] text-muted-foreground mb-0.5">
                    File-based storage (default, works on servers)
                  </p>
                  <code className="text-xs text-foreground break-all">
                    nyxid node register --token {createdToken.token}
                    {urlFlag}
                  </code>
                </div>
                <div>
                  <p className="text-[10px] text-muted-foreground mb-0.5">
                    OS keychain storage (macOS Keychain, Windows Credential
                    Manager)
                  </p>
                  <code className="text-xs text-foreground break-all">
                    nyxid node register --token {createdToken.token}
                    {urlFlag} --keychain
                  </code>
                </div>
              </div>
            </div>
            <DialogFooter>
              <Button variant="primary" onClick={handleClose}>Done</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Register a New Node</DialogTitle>
              <DialogDescription>
                Create a registration token to connect a new credential node.
              </DialogDescription>
            </DialogHeader>
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                    {form.formState.errors.root.message}
                  </div>
                )}
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Node Name</FormLabel>
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
                      <FormLabel>Owner</FormLabel>
                      <FormControl>
                        <OrgScopeSelect
                          value={field.value ?? null}
                          onChange={field.onChange}
                          label="Node owner"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={handleClose}>
                    Cancel
                  </Button>
                  <Button variant="primary" type="submit" isLoading={createMutation.isPending} disabled={!form.formState.isValid || createMutation.isPending}>
                    Create Token
                  </Button>
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
  const navigate = useNavigate();
  const { data: nodes, isLoading, error, refetch } = useNodes();
  const { data: orgs } = useOrgs();
  const currentUserId = useAuthStore((state) => state.user?.id ?? null);
  const deleteMutation = useDeleteNode();
  const [deleteTarget, setDeleteTarget] = useState<{
    readonly id: string;
    readonly name: string;
  } | null>(null);
  const adminOrgIds = new Set(
    (orgs ?? [])
      .filter((org) => org.your_role === "admin")
      .map((org) => org.id),
  );

  async function handleDelete() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      toast.success(`Node "${deleteTarget.name}" deleted`);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete node",
      );
    } finally {
      setDeleteTarget(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Credential Nodes"
        description="Manage your credential nodes for self-hosted proxy routing."
        actions={<RegisterNodeDialog />}
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={`node-skel-${String(i)}`} className="h-16 w-full" />
          ))}
        </div>
      ) : error ? (
        <ErrorBanner message="Failed to load nodes. Please try again." onRetry={refetch} />
      ) : !nodes || nodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <WifiRouterIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="max-w-md space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">No Credential Nodes</p>
            <p className="text-[12px] text-muted-foreground/30">
              Create a registration token to get started.
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {nodes.map((node) => (
              <div
                key={node.id}
                role="button"
                tabIndex={0}
                onClick={() => void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } })}
                onKeyDown={(e) => { if (e.key === "Enter") void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } }); }}
                className="relative rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-white/[0.03] cursor-pointer"
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
                <p className="pr-10 text-[13px] font-semibold text-foreground truncate">
                  {node.name}
                  {node.metadata?.agent_version && (
                    <span className="ml-2 text-[11px] font-normal text-muted-foreground">
                      v{node.metadata.agent_version}
                    </span>
                  )}
                </p>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
                  <Badge variant="secondary">{nodeOwnerLabel(node.owner, currentUserId)}</Badge>
                </div>
                <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                  <span>{formatRelativeTime(node.last_heartbeat_at) ?? "No heartbeat"}</span>
                  <span>Created {formatRelativeTime(node.created_at)}</span>
                </div>
              </div>
            ))}
          </div>

          {/* Desktop table view */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Last Heartbeat</TableHead>
                  <TableHead>Created</TableHead>
                  {nodes.some((n) => canManageNode(n, currentUserId, adminOrgIds)) && (
                    <TableHead className="w-[80px]">Actions</TableHead>
                  )}
                </TableRow>
              </TableHeader>
              <TableBody>
                {nodes.map((node) => (
                  <TableRow
                    key={node.id}
                    className="cursor-pointer hover:bg-white/[0.03]"
                    onClick={() => void navigate({ to: "/nodes/$nodeId", params: { nodeId: node.id } })}
                  >
                    <TableCell>
                      <span className="font-medium">
                        {node.name}
                      </span>
                      {node.metadata?.agent_version && (
                        <span className="ml-2 text-xs text-muted-foreground">
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
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatRelativeTime(node.last_heartbeat_at) ?? "Never"}
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
                            <span className="sr-only">Delete {node.name}</span>
                          </Button>
                        )}
                      </TableCell>
                    )}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
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
            <DialogTitle>Delete Node</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{deleteTarget?.name ?? ""}
              &quot;? This will disconnect the node and detach any AI Services
              routed through it. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
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
    </div>
  );
}
