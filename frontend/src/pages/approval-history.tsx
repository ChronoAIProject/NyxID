import { useState } from "react";
import { useApprovalRequests, useDecideApproval } from "@/hooks/use-approvals";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
  ClipboardList,
  CheckCircle2,
  XCircle,
  ChevronLeft,
  ChevronRight,
  Wrench,
} from "lucide-react";
import type { ApprovalRequestItem } from "@/types/approvals";
import { toast } from "sonner";

const STATUS_OPTIONS = [
  { value: "all", label: "All Statuses" },
  { value: "pending", label: "Pending" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
  { value: "expired", label: "Expired" },
] as const;

function getStatusBadge(status: string) {
  switch (status) {
    case "approved":
      return <Badge variant="success">Approved</Badge>;
    case "rejected":
      return <Badge variant="destructive">Rejected</Badge>;
    case "expired":
      return <Badge variant="secondary">Expired</Badge>;
    case "pending":
      return <Badge variant="warning">Pending</Badge>;
    default:
      return <Badge variant="secondary">{status}</Badge>;
  }
}

function isToolApproval(request: ApprovalRequestItem): boolean {
  return request.tool_name != null;
}

export function ApprovalHistoryPage() {
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState("all");
  const [decideTarget, setDecideTarget] = useState<{
    readonly id: string;
    readonly service: string;
    readonly approvalMode: "per_request" | "grant";
    readonly action: "approve" | "reject";
  } | null>(null);

  const perPage = 20;
  const filterValue = statusFilter === "all" ? undefined : statusFilter;
  const { data, isLoading, error, refetch } = useApprovalRequests(
    page,
    perPage,
    filterValue,
  );
  const decideMutation = useDecideApproval();

  const requests = data?.requests ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  async function handleDecide() {
    if (!decideTarget) return;
    try {
      await decideMutation.mutateAsync({
        requestId: decideTarget.id,
        approved: decideTarget.action === "approve",
      });
      toast.success(
        decideTarget.action === "approve"
          ? "Request approved"
          : "Request rejected",
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : `Failed to ${decideTarget.action} request`,
      );
    } finally {
      setDecideTarget(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Approval History"
        description="View past and pending approval requests."
        actions={
          <Select
            value={statusFilter}
            onValueChange={(value) => {
              setStatusFilter(value);
              setPage(1);
            }}
          >
            <SelectTrigger className="w-[160px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {STATUS_OPTIONS.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`req-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <ErrorBanner message="Failed to load approval history. Please try again." onRetry={refetch} />
      ) : requests.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <ClipboardList className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="max-w-md space-y-1">
            <p className="text-[12px] font-medium">No Approval Requests</p>
            <p className="text-[12px] text-muted-foreground">
              No approval requests match the current filter.
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {requests.map((request) => (
              <div key={request.id} className="relative rounded-xl border border-border/50 bg-card p-4">
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    {isToolApproval(request) ? (
                      <div className="flex items-center gap-1.5">
                        <Wrench className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <p className="text-[13px] font-semibold text-foreground truncate">{request.tool_name}</p>
                      </div>
                    ) : (
                      <p className="text-[13px] font-semibold text-foreground truncate">{request.service_name}</p>
                    )}
                    <p className="text-[11px] text-muted-foreground truncate">
                      {request.requester_label ?? request.requester_type}
                    </p>
                  </div>
                  {getStatusBadge(request.status)}
                </div>
                <p className="mt-1.5 text-[11px] text-muted-foreground line-clamp-2">
                  {isToolApproval(request)
                    ? request.tool_arguments ?? "Tool execution approval"
                    : request.action_description ?? request.operation_summary}
                </p>
                {request.is_destructive && (
                  <Badge variant="destructive" className="mt-1.5">Destructive</Badge>
                )}
                <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                  <span>{formatDate(request.created_at)}</span>
                  {request.decided_at && <span>Decided {formatDate(request.decided_at)}</span>}
                </div>
                {request.status === "pending" && (
                  <div className="mt-3 flex gap-2 border-t border-border/40 pt-3">
                    <Button
                      variant="outline"
                      size="sm"
                      className="flex-1 h-8 text-xs"
                      onClick={() =>
                        setDecideTarget({
                          id: request.id,
                          service: request.service_name,
                          approvalMode: request.approval_mode,
                          action: "approve",
                        })
                      }
                    >
                      <ButtonIcon><CheckCircle2 className="h-3 w-3 text-success" /></ButtonIcon>
                      Approve
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="flex-1 h-8 text-xs"
                      onClick={() =>
                        setDecideTarget({
                          id: request.id,
                          service: request.service_name,
                          approvalMode: request.approval_mode,
                          action: "reject",
                        })
                      }
                    >
                      <ButtonIcon><XCircle className="h-3 w-3 text-destructive" /></ButtonIcon>
                      Reject
                    </Button>
                  </div>
                )}
              </div>
            ))}
          </div>

          {/* Desktop table view */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>Requester</TableHead>
                  <TableHead>Action</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Requested</TableHead>
                  <TableHead>Decided</TableHead>
                  {requests.some((r) => r.status === "pending") && (
                    <TableHead className="w-[100px]">Actions</TableHead>
                  )}
                </TableRow>
              </TableHeader>
              <TableBody>
                {requests.map((request) => (
                  <TableRow key={request.id}>
                    <TableCell>
                      {isToolApproval(request) ? (
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center gap-1.5">
                            <Wrench className="h-3.5 w-3.5 text-muted-foreground" />
                            <span className="font-medium">
                              {request.tool_name}
                            </span>
                          </div>
                          {request.is_destructive && (
                            <Badge variant="destructive">Destructive</Badge>
                          )}
                        </div>
                      ) : (
                        <div className="flex flex-col gap-0.5">
                          <span className="font-medium">
                            {request.service_name}
                          </span>
                          <span className="text-[11px] text-muted-foreground">
                            {request.service_slug}
                          </span>
                        </div>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-0.5">
                        <span>
                          {request.requester_label ?? request.requester_type}
                        </span>
                        <span className="text-[11px] text-muted-foreground">
                          {request.requester_type}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-0.5">
                        {isToolApproval(request) ? (
                          <>
                            <span>
                              Tool execution approval
                            </span>
                            {request.tool_arguments && (
                              <span className="max-w-[300px] truncate text-[11px] text-muted-foreground">
                                {request.tool_arguments}
                              </span>
                            )}
                          </>
                        ) : (
                          <>
                            <span>
                              {request.action_description ??
                                request.operation_summary}
                            </span>
                            {request.action_description && (
                              <span className="text-[11px] text-muted-foreground">
                                {request.operation_summary}
                              </span>
                            )}
                          </>
                        )}
                      </div>
                    </TableCell>
                    <TableCell>{getStatusBadge(request.status)}</TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(request.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {request.decided_at
                        ? formatDate(request.decided_at)
                        : "-"}
                    </TableCell>
                    {requests.some((r) => r.status === "pending") && (
                      <TableCell>
                        {request.status === "pending" && (
                          <div className="flex justify-end gap-1">
                            <Button
                              variant="ghost"
                              className="h-7 text-xs"
                              onClick={() =>
                                setDecideTarget({
                                  id: request.id,
                                  service: request.service_name,
                                  approvalMode: request.approval_mode,
                                  action: "approve",
                                })
                              }
                            >
                              <ButtonIcon><CheckCircle2 className="h-3 w-3 text-success" /></ButtonIcon>
                              Approve
                            </Button>
                            <Button
                              variant="ghost"
                              className="h-7 text-xs"
                              onClick={() =>
                                setDecideTarget({
                                  id: request.id,
                                  service: request.service_name,
                                  approvalMode: request.approval_mode,
                                  action: "reject",
                                })
                              }
                            >
                              <ButtonIcon><XCircle className="h-3 w-3 text-destructive" /></ButtonIcon>
                              Reject
                            </Button>
                          </div>
                        )}
                      </TableCell>
                    )}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between">
              <p className="text-[12px] text-muted-foreground">
                Showing {String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))} of {String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-[12px]">
                  Page {String(page)} of {String(totalPages)}
                </span>
                <Button
                  variant="outline"
                  disabled={page >= totalPages}
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          )}
        </>
      )}

      {/* Decide Confirmation Dialog */}
      <Dialog
        open={decideTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDecideTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {decideTarget?.action === "approve"
                ? "Approve Request"
                : "Reject Request"}
            </DialogTitle>
            <DialogDescription>
              {decideTarget?.action === "approve"
                ? decideTarget.approvalMode === "grant"
                  ? `Are you sure you want to approve access to "${decideTarget?.service ?? ""}"? A reusable approval grant will be created using your configured expiry.`
                  : `Are you sure you want to approve access to "${decideTarget?.service ?? ""}"? This approval applies only to the current request.`
                : `Are you sure you want to reject access to "${decideTarget?.service ?? ""}"?`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDecideTarget(null)}>
              Cancel
            </Button>
            <Button
              variant={
                decideTarget?.action === "approve" ? "default" : "destructive"
              }
              onClick={() => void handleDecide()}
              isLoading={decideMutation.isPending}
            >
              {decideTarget?.action === "approve" ? "Approve" : "Reject"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
