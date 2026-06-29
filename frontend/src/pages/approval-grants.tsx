import { useState } from "react";
import { useApprovalGrants, useRevokeGrant } from "@/hooks/use-approvals";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
import { Trash2, ChevronLeft, ChevronRight } from "lucide-react";
import { SmartLockIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

export function ApprovalGrantsPage() {
  const [page, setPage] = useState(1);
  const [revokeGrantId, setRevokeGrantId] = useState<string | null>(null);

  const perPage = 20;
  const { data, isLoading, error, refetch } = useApprovalGrants(page, perPage);
  const revokeMutation = useRevokeGrant();

  const grants = data?.grants ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  const revokeTarget = grants.find((g) => g.id === revokeGrantId);

  async function handleRevoke() {
    if (!revokeGrantId) return;
    try {
      await revokeMutation.mutateAsync({ grantId: revokeGrantId });
      toast.success("Grant revoked");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke grant",
      );
    } finally {
      setRevokeGrantId(null);
    }
  }

  function isExpiringSoon(expiresAt: string): boolean {
    const expiry = new Date(expiresAt).getTime();
    const threeDaysMs = 3 * 24 * 60 * 60 * 1000;
    return expiry - Date.now() < threeDaysMs;
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Active Grants"
        description="Manage active approval grants. Revoking a grant will require re-approval on the next request."
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`grant-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <ErrorBanner message="Failed to load grants. Please try again." onRetry={refetch} />
      ) : grants.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <SmartLockIcon className="h-64 w-64 text-muted-foreground" />
          <div className="max-w-md space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">No Active Grants</p>
            <p className="text-[12px] text-muted-foreground">
              Services using per-request approval do not create grants. Only
              services set to time-based grant mode will appear here.
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {grants.map((grant) => (
              <div key={grant.id} className="relative rounded-xl border border-border/50 bg-card p-4">
                <div className="absolute right-3 top-3">
                  <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setRevokeGrantId(grant.id)}>
                    <Trash2 className="h-3.5 w-3.5 text-destructive" />
                  </Button>
                </div>
                <p className="pr-10 text-[13px] font-semibold text-foreground truncate">{grant.service_name}</p>
                <p className="text-[11px] text-muted-foreground">{grant.requester_label ?? grant.requester_type}</p>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {isExpiringSoon(grant.expires_at) && (
                    <Badge variant="warning">Expiring soon</Badge>
                  )}
                </div>
                <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                  <span>Granted {formatDate(grant.granted_at)}</span>
                  <span>Expires {formatDate(grant.expires_at)}</span>
                </div>
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
                  <TableHead>Granted</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead className="w-[60px]">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {grants.map((grant) => (
                  <TableRow key={grant.id}>
                    <TableCell className="font-medium">
                      {grant.service_name}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-0.5">
                        <span>
                          {grant.requester_label ?? grant.requester_type}
                        </span>
                        <span className="text-[11px] text-muted-foreground">
                          {grant.requester_type}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(grant.granted_at)}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <span className="text-muted-foreground">
                          {formatDate(grant.expires_at)}
                        </span>
                        {isExpiringSoon(grant.expires_at) && (
                          <Badge variant="warning" className="text-xs">
                            Expiring soon
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        onClick={() => setRevokeGrantId(grant.id)}
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between">
              <p className="text-[11px] text-text-tertiary">
                Showing {String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))} of {String(total)}
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
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                  aria-label="Next page"
                >
                  <ChevronRight className="h-3 w-3" />
                </Button>
              </div>
            </div>
          )}
        </>
      )}

      {/* Revoke Confirmation */}
      <Dialog
        open={revokeGrantId !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeGrantId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke Grant</DialogTitle>
            <DialogDescription>
              Are you sure you want to revoke the grant for{" "}
              {revokeTarget ? `"${revokeTarget.service_name}"` : "this service"}
              ? The requester will need to request approval again.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRevokeGrantId(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevoke()}
              isLoading={revokeMutation.isPending}
            >
              Revoke Grant
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
