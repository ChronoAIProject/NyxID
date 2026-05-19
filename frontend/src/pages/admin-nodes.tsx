import { useState } from "react";
import {
  useAdminNodes,
  useAdminDisconnectNode,
  useAdminDeleteNode,
} from "@/hooks/use-admin-nodes";
import { ApiError } from "@/lib/api-client";
import { formatRelativeTime } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Search,
  ChevronLeft,
  ChevronRight,
  Unplug,
  Trash2,
  MoreVertical,
} from "lucide-react";
import { MagicBoxIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";
import type { AdminNodeInfo } from "@/types/nodes";

const PER_PAGE = 50;

export function AdminNodesPage() {
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("");
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [actionTarget, setActionTarget] = useState<{
    readonly node: AdminNodeInfo;
    readonly action: "disconnect" | "delete";
  } | null>(null);

  const { data, isLoading, error } = useAdminNodes(
    page,
    PER_PAGE,
    statusFilter || undefined,
    search || undefined,
  );
  const disconnectMutation = useAdminDisconnectNode();
  const deleteMutation = useAdminDeleteNode();

  const nodes = data?.nodes ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PER_PAGE));

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    setSearch(searchInput);
    setPage(1);
  }

  async function handleAction() {
    if (!actionTarget) return;
    const { node, action } = actionTarget;
    try {
      if (action === "disconnect") {
        await disconnectMutation.mutateAsync(node.id);
        toast.success(`Node "${node.name}" disconnected`);
      } else {
        await deleteMutation.mutateAsync(node.id);
        toast.success(`Node "${node.name}" deleted`);
      }
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : `Failed to ${action} node`,
      );
    } finally {
      setActionTarget(null);
    }
  }

  function formatSuccessRate(metrics: AdminNodeInfo["metrics"]): string {
    if (!metrics || metrics.total_requests === 0) return "--";
    return `${(metrics.success_rate * 100).toFixed(1)}%`;
  }

  function formatLatency(metrics: AdminNodeInfo["metrics"]): string {
    if (!metrics || metrics.total_requests === 0) return "--";
    return `${metrics.avg_latency_ms.toFixed(0)}ms`;
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Node Registry"
        description="View and manage all credential nodes across all users."
      />

      <div className="flex items-center gap-2">
        <form onSubmit={handleSearch} className="flex items-center gap-2">
          <div className="relative max-w-sm flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Filter by user ID..."
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              className="pl-9"
            />
          </div>
          <Button type="submit" variant="outline" size="sm">
            Search
          </Button>
          {search && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => {
                setSearchInput("");
                setSearch("");
                setPage(1);
              }}
            >
              Clear
            </Button>
          )}
        </form>
        <Select
          value={statusFilter}
          onValueChange={(value) => {
            setStatusFilter(value === "all" ? "" : value);
            setPage(1);
          }}
        >
          <SelectTrigger className="w-[140px]">
            <SelectValue placeholder="All statuses" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All statuses</SelectItem>
            <SelectItem value="online">Online</SelectItem>
            <SelectItem value="offline">Offline</SelectItem>
            <SelectItem value="draining">Draining</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton
              key={`admin-node-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MagicBoxIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">Failed to load nodes</p>
            <p className="text-xs text-muted-foreground/30">Please try again later.</p>
          </div>
        </div>
      ) : nodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MagicBoxIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">No nodes found</p>
            <p className="text-xs text-muted-foreground/30">
              {search || statusFilter
                ? "No nodes match your filters."
                : "No credential nodes registered."}
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
                className="rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-white/[0.03]"
              >
                <div className="relative">
                  {canWrite && (
                    <div className="absolute right-0 top-0">
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7 text-muted-foreground hover:text-foreground"
                          >
                            <MoreVertical className="h-3 w-3" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          {node.is_connected && (
                            <DropdownMenuItem
                              onClick={() =>
                                setActionTarget({
                                  node,
                                  action: "disconnect",
                                })
                              }
                            >
                              <Unplug className="h-3 w-3" />
                              Disconnect
                            </DropdownMenuItem>
                          )}
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onClick={() =>
                              setActionTarget({ node, action: "delete" })
                            }
                          >
                            <Trash2 className="h-3 w-3" />
                            Delete
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  )}

                  <div className="pr-8">
                    <p className="text-[13px] font-semibold text-foreground truncate">
                      {node.name}
                      {node.metadata?.agent_version && (
                        <span className="ml-2 text-[11px] font-normal text-muted-foreground">
                          v{node.metadata.agent_version}
                        </span>
                      )}
                    </p>
                    <p className="text-[11px] font-mono text-muted-foreground truncate">
                      {node.id}
                    </p>
                  </div>

                  <div className="mt-2 flex flex-wrap items-center gap-1.5">
                    <NodeStatusBadge
                      status={node.status}
                      isConnected={node.is_connected}
                    />
                    <Badge variant="secondary">
                      {node.user_email ?? node.user_id}
                    </Badge>
                  </div>

                  <div className="mt-3 text-[11px] text-muted-foreground">
                    Last seen:{" "}
                    {formatRelativeTime(node.last_heartbeat_at) ?? "Never"}
                  </div>
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
                  <TableHead>User</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Requests</TableHead>
                  <TableHead>Success Rate</TableHead>
                  <TableHead>Avg Latency</TableHead>
                  <TableHead>Last Heartbeat</TableHead>
                  <TableHead className="w-[100px]">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {nodes.map((node) => (
                  <TableRow key={node.id}>
                    <TableCell>
                      <div>
                        <span className="font-medium text-foreground">
                          {node.name}
                        </span>
                        {node.metadata?.agent_version && (
                          <span className="ml-2 text-xs text-muted-foreground">
                            v{node.metadata.agent_version}
                          </span>
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      <span className="text-[11px] text-text-tertiary">
                        {node.user_email ?? node.user_id}
                      </span>
                    </TableCell>
                    <TableCell>
                      <NodeStatusBadge
                        status={node.status}
                        isConnected={node.is_connected}
                      />
                    </TableCell>
                    <TableCell>
                      <span className="text-sm tabular-nums text-muted-foreground">
                        {node.metrics
                          ? String(node.metrics.total_requests)
                          : "0"}
                      </span>
                    </TableCell>
                    <TableCell>
                      <span className="text-sm tabular-nums text-muted-foreground">
                        {formatSuccessRate(node.metrics)}
                      </span>
                    </TableCell>
                    <TableCell>
                      <span className="text-sm tabular-nums text-muted-foreground">
                        {formatLatency(node.metrics)}
                      </span>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatRelativeTime(node.last_heartbeat_at) ?? "Never"}
                    </TableCell>
                    <TableCell>
                      {canWrite && (
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-muted-foreground hover:text-foreground"
                            >
                              <MoreVertical className="h-3 w-3" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            {node.is_connected && (
                              <DropdownMenuItem
                                onClick={() =>
                                  setActionTarget({
                                    node,
                                    action: "disconnect",
                                  })
                                }
                              >
                                <Unplug className="h-3 w-3" />
                                Disconnect
                              </DropdownMenuItem>
                            )}
                            <DropdownMenuItem
                              className="text-destructive focus:text-destructive"
                              onClick={() =>
                                setActionTarget({ node, action: "delete" })
                              }
                            >
                              <Trash2 className="h-3 w-3" />
                              Delete
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          <div className="flex items-center justify-between">
            <p className="text-[11px] text-text-tertiary">
              Showing {String((page - 1) * PER_PAGE + 1)}-
              {String(Math.min(page * PER_PAGE, total))} of {String(total)}{" "}
              nodes
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="icon"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                aria-label="Previous page"
              >
                <ChevronLeft className="h-3 w-3" />
              </Button>
              <span className="text-[11px] text-text-tertiary">
                Page {String(page)} of {String(totalPages)}
              </span>
              <Button
                variant="outline"
                size="icon"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
                aria-label="Next page"
              >
                <ChevronRight className="h-3 w-3" />
              </Button>
            </div>
          </div>
        </>
      )}

      {/* Action Confirmation Dialog */}
      <Dialog
        open={actionTarget !== null}
        onOpenChange={(open) => {
          if (!open) setActionTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {actionTarget?.action === "disconnect"
                ? "Disconnect Node"
                : "Delete Node"}
            </DialogTitle>
            <DialogDescription>
              {actionTarget?.action === "disconnect"
                ? `Are you sure you want to force-disconnect "${actionTarget.node.name}"? The node will need to reconnect.`
                : `Are you sure you want to delete "${actionTarget?.node.name ?? ""}"? This will remove the node and detach any AI Services routed through it. This action cannot be undone.`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setActionTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleAction()}
              isLoading={
                disconnectMutation.isPending || deleteMutation.isPending
              }
            >
              {actionTarget?.action === "disconnect" ? "Disconnect" : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
