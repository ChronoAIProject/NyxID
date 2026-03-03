import { useState } from "react";
import { useApprovalRequests, useDecideApproval } from "@/hooks/use-approvals";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
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
  Clock,
  Timer,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
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
      return (
        <Badge variant="success" className="gap-1">
          <CheckCircle2 className="h-3 w-3" />
          Approved
        </Badge>
      );
    case "rejected":
      return (
        <Badge variant="destructive" className="gap-1">
          <XCircle className="h-3 w-3" />
          Rejected
        </Badge>
      );
    case "expired":
      return (
        <Badge variant="secondary" className="gap-1">
          <Timer className="h-3 w-3" />
          Expired
        </Badge>
      );
    case "pending":
      return (
        <Badge variant="warning" className="gap-1">
          <Clock className="h-3 w-3" />
          Pending
        </Badge>
      );
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

export function ApprovalHistoryPage() {
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState("all");
  const [decideTarget, setDecideTarget] = useState<{
    readonly id: string;
    readonly service: string;
    readonly action: "approve" | "reject";
  } | null>(null);

  const perPage = 20;
  const filterValue = statusFilter === "all" ? undefined : statusFilter;
  const { data, isLoading, error } = useApprovalRequests(
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
            <Skeleton
              key={`req-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ClipboardList className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load approval history. Please try again.
          </p>
        </div>
      ) : requests.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ClipboardList className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No approval requests found.
          </p>
        </div>
      ) : (
        <>
          <div className="rounded-xl border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>Requester</TableHead>
                  <TableHead>Operation</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Requested</TableHead>
                  <TableHead>Decided</TableHead>
                  <TableHead className="w-[100px]" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {requests.map((request) => (
                  <TableRow key={request.id}>
                    <TableCell>
                      <div className="flex flex-col">
                        <span className="font-medium">
                          {request.service_name}
                        </span>
                        <span className="text-xs text-muted-foreground">
                          {request.service_slug}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col">
                        <span className="text-sm">
                          {request.requester_label ?? request.requester_type}
                        </span>
                        <span className="text-xs text-muted-foreground">
                          {request.requester_type}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <span className="font-mono text-xs">
                        {request.operation_summary}
                      </span>
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
                    <TableCell>
                      {request.status === "pending" && (
                        <div className="flex gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 text-xs"
                            onClick={() =>
                              setDecideTarget({
                                id: request.id,
                                service: request.service_name,
                                action: "approve",
                              })
                            }
                          >
                            <CheckCircle2 className="mr-1 h-3 w-3 text-success" />
                            Approve
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 text-xs"
                            onClick={() =>
                              setDecideTarget({
                                id: request.id,
                                service: request.service_name,
                                action: "reject",
                              })
                            }
                          >
                            <XCircle className="mr-1 h-3 w-3 text-destructive" />
                            Reject
                          </Button>
                        </div>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between">
              <p className="text-sm text-muted-foreground">
                Showing {String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))} of {String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-sm">
                  Page {String(page)} of {String(totalPages)}
                </span>
                <Button
                  variant="outline"
                  size="sm"
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
                ? `Are you sure you want to approve access to "${decideTarget?.service ?? ""}"? An approval grant will be created.`
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
