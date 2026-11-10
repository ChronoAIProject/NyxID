import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { useApprovalRequests, useDecideApproval } from "@/hooks/use-approvals";
import { ApiError } from "@/lib/api-client";
import {
  isToolApproval,
  operationIdentityLabel,
  primaryActionLabel,
  shouldShowRawIdentityLine,
} from "@/lib/approval-labels";
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
  CheckCircle2,
  XCircle,
  ChevronLeft,
  ChevronRight,
  Wrench,
} from "lucide-react";
import { SeoKeywordIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

const STATUS_OPTIONS = [
  { value: "all", get label() { return translateUi("All Statuses"); } },
  { value: "pending", get label() { return translateUi("Pending"); } },
  { value: "approved", get label() { return translateUi("Approved"); } },
  { value: "rejected", get label() { return translateUi("Rejected"); } },
  { value: "expired", get label() { return translateUi("Expired"); } },
] as const;

function getStatusBadge(status: string) {
  switch (status) {
    case "approved":
      return <Badge variant="success">{translateUi("Approved")}</Badge>;
    case "rejected":
      return <Badge variant="destructive">{translateUi("Rejected")}</Badge>;
    case "expired":
      return <Badge variant="secondary">{translateUi("Expired")}</Badge>;
    case "pending":
      return <Badge variant="warning">{translateUi("Pending")}</Badge>;
    default:
      return <Badge variant="secondary">{status}</Badge>;
  }
}

export function ApprovalHistoryPage() {
  const { t } = useUiTranslation();
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
        translateErrorMessage(err instanceof ApiError
          ? err.message
          : `Failed to ${decideTarget.action} request`),
      );
    } finally {
      setDecideTarget(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("Approval History")}
        description={t("View past and pending approval requests.")}
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
        <ErrorBanner message={t("Failed to load approval history. Please try again.")} onRetry={refetch} />
      ) : requests.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <SeoKeywordIcon className="h-64 w-64 text-muted-foreground" />
          <div className="max-w-md space-y-1">
            <p className="text-12 font-medium text-muted-foreground">{t("No Approval Requests")}</p>
            <p className="text-12 text-muted-foreground">{t("No approval requests match the current filter.")}</p>
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
                        <p className="text-13 font-semibold text-foreground truncate">{request.tool_name}</p>
                      </div>
                    ) : (
                      <p className="text-13 font-semibold text-foreground truncate">{request.service_name}</p>
                    )}
                    <p className="text-11 text-muted-foreground truncate">
                      {request.requester_label ?? request.requester_type}
                    </p>
                  </div>
                  {getStatusBadge(request.status)}
                </div>
                <p className="mt-1.5 text-11 text-muted-foreground line-clamp-2">
                  {isToolApproval(request)
                    ? request.tool_arguments ?? t("Tool execution approval")
                    : primaryActionLabel(request)}
                </p>
                {shouldShowRawIdentityLine(request) && (
                  <p className="mt-1 text-11 text-muted-foreground">
                    {operationIdentityLabel(request)}
                  </p>
                )}
                {request.is_destructive && (
                  <Badge variant="destructive" className="mt-1.5">{t("Destructive")}</Badge>
                )}
                <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-11 text-muted-foreground">
                  <span>{formatDate(request.created_at)}</span>
                  {request.decided_at && <span>{t("Decided ")}{formatDate(request.decided_at)}</span>}
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
                      <ButtonIcon><CheckCircle2 className="h-3 w-3 text-success" /></ButtonIcon>{t("Approve")}</Button>
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
                      <ButtonIcon><XCircle className="h-3 w-3 text-destructive" /></ButtonIcon>{t("Reject")}</Button>
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
                  <TableHead>{t("Service")}</TableHead>
                  <TableHead>{t("Requester")}</TableHead>
                  <TableHead>{t("Action")}</TableHead>
                  <TableHead>{t("Status")}</TableHead>
                  <TableHead>{t("Requested")}</TableHead>
                  <TableHead>{t("Decided")}</TableHead>
                  {requests.some((r) => r.status === "pending") && (
                    <TableHead className="w-[100px]">{t("Actions")}</TableHead>
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
                            <Badge variant="destructive">{t("Destructive")}</Badge>
                          )}
                        </div>
                      ) : (
                        <div className="flex flex-col gap-0.5">
                          <span className="font-medium">
                            {request.service_name}
                          </span>
                          <span className="text-11 text-muted-foreground">
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
                        <span className="text-11 text-muted-foreground">
                          {request.requester_type}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-0.5">
                        {isToolApproval(request) ? (
                          <>
                            <span>{t("Tool execution approval")}</span>
                            {request.tool_arguments && (
                              <span className="max-w-[300px] truncate text-11 text-muted-foreground">
                                {request.tool_arguments}
                              </span>
                            )}
                          </>
                        ) : (
                          <>
                            <span>
                              {primaryActionLabel(request)}
                            </span>
                            {request.action_description &&
                              request.operation_summary && (
                                <span className="text-11 text-muted-foreground">
                                  {request.operation_summary}
                                </span>
                              )}
                            {shouldShowRawIdentityLine(request) && (
                              <span className="text-11 text-muted-foreground">
                                {operationIdentityLabel(request)}
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
                              <ButtonIcon><CheckCircle2 className="h-3 w-3 text-success" /></ButtonIcon>{t("Approve")}</Button>
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
                              <ButtonIcon><XCircle className="h-3 w-3 text-destructive" /></ButtonIcon>{t("Reject")}</Button>
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
              <p className="text-11 text-text-tertiary">{t("Showing ")}{String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))}{t(" of ")}{String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="icon"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  aria-label={t("Previous page")}
                >
                  <ChevronLeft className="h-3 w-3" />
                </Button>
                <span className="text-11 text-text-tertiary">{t("Page ")}{String(page)}{t(" of ")}{String(totalPages)}
                </span>
                <Button
                  variant="outline"
                  size="icon"
                  disabled={page >= totalPages}
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                  aria-label={t("Next page")}
                >
                  <ChevronRight className="h-3 w-3" />
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
                ? t("Approve Request")
                : t("Reject Request")}
            </DialogTitle>
            <DialogDescription>
              {decideTarget?.action === "approve"
                ? decideTarget.approvalMode === "grant"
                  ? t("Are you sure you want to approve access to \"{{value}}\"? A reusable approval grant will be created using your configured expiry.", { value: decideTarget?.service ?? "" })
                  : t("Are you sure you want to approve access to \"{{value}}\"? This approval applies only to the current request.", { value: decideTarget?.service ?? "" })
                : t("Are you sure you want to reject access to \"{{value}}\"?", { value: decideTarget?.service ?? "" })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDecideTarget(null)}>{t("Cancel")}</Button>
            <Button
              variant={
                decideTarget?.action === "approve" ? "default" : "destructive"
              }
              onClick={() => void handleDecide()}
              isLoading={decideMutation.isPending}
            >
              {decideTarget?.action === "approve" ? t("Approve") : t("Reject")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
