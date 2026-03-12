import { useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import {
  useNode,
  useNodeBindings,
  useDeleteNode,
  useRotateNodeToken,
  useCreateBinding,
  useUpdateBinding,
  useDeleteBinding,
} from "@/hooks/use-nodes";
import { useServices } from "@/hooks/use-services";
import { ApiError } from "@/lib/api-client";
import { formatDate, formatRelativeTime } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { CopyableField } from "@/components/shared/copyable-field";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Activity,
  ArrowDown,
  ArrowUp,
  HardDrive,
  KeyRound,
  Link2,
  Plus,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";

export function NodeDetailPage() {
  const { nodeId } = useParams({ strict: false }) as { nodeId: string };
  const navigate = useNavigate();

  const { data: node, isLoading, error } = useNode(nodeId);
  const { data: bindings, isLoading: bindingsLoading } =
    useNodeBindings(nodeId);
  const { data: services } = useServices();

  const deleteMutation = useDeleteNode();
  const rotateMutation = useRotateNodeToken();
  const createBindingMutation = useCreateBinding();
  const updateBindingMutation = useUpdateBinding();
  const deleteBindingMutation = useDeleteBinding();

  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showRotateDialog, setShowRotateDialog] = useState(false);
  const [rotatedCredentials, setRotatedCredentials] = useState<{
    readonly auth_token: string;
    readonly signing_secret: string;
  } | null>(null);
  const [showBindDialog, setShowBindDialog] = useState(false);
  const [selectedServiceId, setSelectedServiceId] = useState("");
  const [unbindTarget, setUnbindTarget] = useState<{
    readonly id: string;
    readonly name: string;
  } | null>(null);

  // Filter out services that already have bindings
  const boundServiceIds = new Set(
    (bindings ?? []).map((b) => b.service_id),
  );
  const availableServices = (services ?? []).filter(
    (s) => s.is_active && !boundServiceIds.has(s.id),
  );

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

  async function handleCreateBinding() {
    if (!selectedServiceId) return;
    try {
      const result = await createBindingMutation.mutateAsync({
        nodeId,
        serviceId: selectedServiceId,
      });
      toast.success(`Bound to ${result.service_name}`);
      setShowBindDialog(false);
      setSelectedServiceId("");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to create binding",
      );
    }
  }

  async function handleDeleteBinding() {
    if (!unbindTarget) return;
    try {
      await deleteBindingMutation.mutateAsync({
        nodeId,
        bindingId: unbindTarget.id,
      });
      toast.success(`Unbound from ${unbindTarget.name}`);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to remove binding",
      );
    } finally {
      setUnbindTarget(null);
    }
  }

  async function handlePriorityChange(bindingId: string, newPriority: number) {
    try {
      await updateBindingMutation.mutateAsync({
        nodeId,
        bindingId,
        priority: newPriority,
      });
      toast.success("Priority updated");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update priority",
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
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <HardDrive className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">
          Node not found or failed to load.
        </p>
        <Button
          variant="outline"
          className="mt-4"
          onClick={() => void navigate({ to: "/nodes" })}
        >
          Back to Nodes
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Nodes", to: "/nodes" },
          { label: node.name },
        ]}
        title={node.name}
        description="Manage node settings and service bindings."
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setShowRotateDialog(true)}
            >
              <KeyRound className="mr-2 h-4 w-4" />
              Rotate Credentials
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setShowDeleteDialog(true)}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </Button>
          </div>
        }
      />

      {/* Node Info */}
      <DetailSection title="Node Information">
        <div className="flex items-center justify-between border-b border-border py-2 text-sm last:border-b-0">
          <span className="text-text-tertiary">Status</span>
          <div className="flex items-center gap-1">
            <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
          </div>
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

      {/* Metrics */}
      {node.metrics && node.metrics.total_requests > 0 && (
        <DetailSection title="Metrics">
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <div className="rounded-lg border border-border p-3 text-center">
              <p className="text-2xl font-semibold text-foreground">
                {String(node.metrics.total_requests)}
              </p>
              <p className="text-xs text-muted-foreground">Total Requests</p>
            </div>
            <div className="rounded-lg border border-border p-3 text-center">
              <p className="text-2xl font-semibold text-foreground">
                {(node.metrics.success_rate * 100).toFixed(1)}%
              </p>
              <p className="text-xs text-muted-foreground">Success Rate</p>
            </div>
            <div className="rounded-lg border border-border p-3 text-center">
              <p className="text-2xl font-semibold text-foreground">
                {node.metrics.avg_latency_ms.toFixed(0)}ms
              </p>
              <p className="text-xs text-muted-foreground">Avg Latency</p>
            </div>
            <div className="rounded-lg border border-border p-3 text-center">
              <p className="text-2xl font-semibold text-foreground">
                {String(node.metrics.error_count)}
              </p>
              <p className="text-xs text-muted-foreground">Errors</p>
            </div>
          </div>
          {node.metrics.last_error && (
            <div className="mt-3 rounded-md bg-destructive/10 p-3">
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
              value={formatRelativeTime(node.metrics.last_success_at) ?? "Never"}
            />
          )}
        </DetailSection>
      )}

      {/* Service Bindings */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-medium">Service Bindings</h3>
            <p className="text-sm text-muted-foreground">
              Services routed through this node for credential injection.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowBindDialog(true)}
            disabled={availableServices.length === 0}
          >
            <Plus className="mr-2 h-4 w-4" />
            Bind Service
          </Button>
        </div>

        {bindingsLoading ? (
          <div className="space-y-2">
            {Array.from({ length: 2 }).map((_, i) => (
              <Skeleton
                key={`bind-skel-${String(i)}`}
                className="h-12 w-full"
              />
            ))}
          </div>
        ) : !bindings || bindings.length === 0 ? (
          <div className="flex flex-col items-center justify-center rounded-xl border border-border py-8 text-center">
            <Link2 className="mb-3 h-8 w-8 text-muted-foreground/50" />
            <p className="text-sm text-muted-foreground">
              No services bound to this node. Bind a service to route proxy
              requests through it.
            </p>
          </div>
        ) : (
          <div className="rounded-xl border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>Slug</TableHead>
                  <TableHead>Priority</TableHead>
                  <TableHead>Bound</TableHead>
                  <TableHead className="w-[80px]" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {bindings.map((binding) => (
                  <TableRow key={binding.id}>
                    <TableCell className="font-medium">
                      {binding.service_name}
                    </TableCell>
                    <TableCell>
                      <code className="text-xs text-muted-foreground">
                        {binding.service_slug}
                      </code>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1">
                        <span className="text-sm tabular-nums">
                          {String(binding.priority)}
                        </span>
                        <div className="flex flex-col">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-5 w-5 text-muted-foreground"
                            onClick={() =>
                              void handlePriorityChange(
                                binding.id,
                                binding.priority - 1,
                              )
                            }
                          >
                            <ArrowUp className="h-3 w-3" />
                            <span className="sr-only">Increase priority</span>
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-5 w-5 text-muted-foreground"
                            onClick={() =>
                              void handlePriorityChange(
                                binding.id,
                                binding.priority + 1,
                              )
                            }
                          >
                            <ArrowDown className="h-3 w-3" />
                            <span className="sr-only">Decrease priority</span>
                          </Button>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatRelativeTime(binding.created_at)}
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 text-muted-foreground hover:text-destructive"
                        onClick={() =>
                          setUnbindTarget({
                            id: binding.id,
                            name: binding.service_name,
                          })
                        }
                      >
                        <Trash2 className="h-4 w-4" />
                        <span className="sr-only">
                          Unbind {binding.service_name}
                        </span>
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      {/* Delete Confirmation */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Node</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{node.name}&quot;? This will
              disconnect the node and remove all service bindings. This action
              cannot be undone.
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
                  Copy both values now. The old credentials have been invalidated
                  immediately, and these secrets will not be shown again.
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
                <div className="rounded-md bg-muted p-3">
                  <p className="mb-1 text-xs font-medium text-text-tertiary">
                    Run on your node
                  </p>
                  <code className="text-xs text-foreground break-all">
                    nyxid-node rekey --auth-token {rotatedCredentials.auth_token}{" "}
                    --signing-secret {rotatedCredentials.signing_secret}
                  </code>
                </div>
              </div>
              <DialogFooter>
                <Button
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

      {/* Bind Service Dialog */}
      <Dialog
        open={showBindDialog}
        onOpenChange={(open) => {
          if (!open) {
            setShowBindDialog(false);
            setSelectedServiceId("");
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Bind Service</DialogTitle>
            <DialogDescription>
              Select a service to route through this node. Proxy requests for the
              bound service will be forwarded to this node for credential
              injection.
            </DialogDescription>
          </DialogHeader>
          <Select value={selectedServiceId} onValueChange={setSelectedServiceId}>
            <SelectTrigger>
              <SelectValue placeholder="Select a service..." />
            </SelectTrigger>
            <SelectContent>
              {availableServices.map((service) => (
                <SelectItem key={service.id} value={service.id}>
                  {service.name} ({service.slug})
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setShowBindDialog(false);
                setSelectedServiceId("");
              }}
            >
              Cancel
            </Button>
            <Button
              onClick={() => void handleCreateBinding()}
              disabled={!selectedServiceId}
              isLoading={createBindingMutation.isPending}
            >
              Bind
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Unbind Confirmation */}
      <Dialog
        open={unbindTarget !== null}
        onOpenChange={(open) => {
          if (!open) setUnbindTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove Binding</DialogTitle>
            <DialogDescription>
              Are you sure you want to unbind &quot;{unbindTarget?.name ?? ""}
              &quot; from this node? Proxy requests for this service will fall
              back to NyxID-stored credentials.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setUnbindTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDeleteBinding()}
              isLoading={deleteBindingMutation.isPending}
            >
              Unbind
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
