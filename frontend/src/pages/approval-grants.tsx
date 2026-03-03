import { useState } from "react";
import { useApprovalGrants, useRevokeGrant } from "@/hooks/use-approvals";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
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
import {
  ShieldCheck,
  Trash2,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import { toast } from "sonner";

export function ApprovalGrantsPage() {
  const [page, setPage] = useState(1);
  const [revokeGrantId, setRevokeGrantId] = useState<string | null>(null);

  const perPage = 20;
  const { data, isLoading, error } = useApprovalGrants(page, perPage);
  const revokeMutation = useRevokeGrant();

  const grants = data?.grants ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  const revokeTarget = grants.find((g) => g.id === revokeGrantId);

  async function handleRevoke() {
    if (!revokeGrantId) return;
    try {
      await revokeMutation.mutateAsync(revokeGrantId);
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
            <Skeleton
              key={`grant-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ShieldCheck className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load grants. Please try again.
          </p>
        </div>
      ) : grants.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ShieldCheck className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No active approval grants.
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
                  <TableHead>Granted</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead className="w-[60px]" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {grants.map((grant) => (
                  <TableRow key={grant.id}>
                    <TableCell className="font-medium">
                      {grant.service_name}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col">
                        <span className="text-sm">
                          {grant.requester_label ?? grant.requester_type}
                        </span>
                        <span className="text-xs text-muted-foreground">
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
                        <Trash2 className="h-4 w-4 text-muted-foreground" />
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
              {revokeTarget
                ? `"${revokeTarget.service_name}"`
                : "this service"}
              ? The requester will need to request approval again.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setRevokeGrantId(null)}
            >
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
