import { useState } from "react";
import { useMyConsents, useRevokeConsent } from "@/hooks/use-consents";
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
import { KeyRound, Trash2 } from "lucide-react";
import { toast } from "sonner";

export function ConsentsPage() {
  const { data, isLoading, error } = useMyConsents();
  const revokeMutation = useRevokeConsent();
  const [revokeClientId, setRevokeClientId] = useState<string | null>(null);

  const consents = data?.consents ?? [];

  const revokeTarget = consents.find((c) => c.client_id === revokeClientId);

  async function handleRevoke() {
    if (!revokeClientId) return;
    try {
      await revokeMutation.mutateAsync(revokeClientId);
      toast.success("Consent revoked");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke consent",
      );
    } finally {
      setRevokeClientId(null);
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Authorized Applications"
        description="Manage applications that have access to your account."
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton
              key={`consent-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <KeyRound className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load consents. Please try again.
          </p>
        </div>
      ) : consents.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <KeyRound className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No applications have been authorized.
          </p>
        </div>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Application</TableHead>
                <TableHead>Scopes</TableHead>
                <TableHead>Granted</TableHead>
                <TableHead>Expires</TableHead>
                <TableHead className="w-[60px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {consents.map((consent) => (
                <TableRow key={consent.id}>
                  <TableCell className="font-medium">
                    {consent.client_name}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {consent.scopes.split(" ").map((scope) => (
                        <Badge
                          key={scope}
                          variant="outline"
                          className="text-xs"
                        >
                          {scope}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(consent.granted_at)}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {consent.expires_at
                      ? formatDate(consent.expires_at)
                      : "Never"}
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8"
                      onClick={() => setRevokeClientId(consent.client_id)}
                    >
                      <Trash2 className="h-4 w-4 text-muted-foreground" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Revoke Confirmation */}
      <Dialog
        open={revokeClientId !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeClientId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke Access</DialogTitle>
            <DialogDescription>
              Are you sure you want to revoke access for{" "}
              {revokeTarget ? `"${revokeTarget.client_name}"` : "this application"}
              ? The application will no longer be able to access your account
              with the granted scopes.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setRevokeClientId(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevoke()}
              isLoading={revokeMutation.isPending}
            >
              Revoke Access
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
