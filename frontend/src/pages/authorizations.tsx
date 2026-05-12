import { useState } from "react";
import {
  type BrokerBindingExternalSubject,
  useMyBrokerBindings,
  useRevokeBrokerBinding,
} from "@/hooks/use-broker-bindings";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
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
import { Link2, Trash2 } from "lucide-react";
import { toast } from "sonner";

function formatExternalSubject(
  subject: BrokerBindingExternalSubject | null,
): string {
  if (!subject) return "—";
  const parts = [subject.platform];
  if (subject.tenant) parts.push(subject.tenant);
  parts.push(subject.external_user_id);
  return parts.join(" · ");
}

export function AuthorizationsPage() {
  const { data, isLoading, error, refetch } = useMyBrokerBindings();
  const revokeMutation = useRevokeBrokerBinding();
  const [revokeBindingHash, setRevokeBindingHash] = useState<string | null>(
    null,
  );

  const bindings = data?.bindings ?? [];

  const revokeTarget = bindings.find(
    (binding) => binding.binding_hash === revokeBindingHash,
  );

  async function handleRevoke() {
    if (!revokeBindingHash) return;
    try {
      await revokeMutation.mutateAsync(revokeBindingHash);
      toast.success("Authorization revoked");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to revoke authorization",
      );
    } finally {
      setRevokeBindingHash(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Authorizations"
        description="Apps and external accounts that hold server-side credentials on your behalf."
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton
              key={`authorization-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <ErrorBanner message="Failed to load authorizations. Please try again." onRetry={refetch} />
      ) : bindings.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-4 py-16">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
              <Link2 className="h-6 w-6 text-muted-foreground" />
            </div>
            <div className="max-w-md space-y-2 text-center">
              <p className="text-[12px] font-medium">No Authorizations</p>
              <p className="text-xs text-muted-foreground">
                No broker authorizations issued. Apps that hold server-side credentials on your behalf will appear here.
              </p>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Application</TableHead>
                <TableHead>External account</TableHead>
                <TableHead>Scopes</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>Last used</TableHead>
                <TableHead className="w-[60px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {bindings.map((binding) => (
                <TableRow key={binding.binding_hash}>
                  <TableCell className="font-medium">
                    {binding.client_name ?? binding.client_id}
                  </TableCell>
                  <TableCell
                    className={
                      binding.external_subject
                        ? undefined
                        : "text-muted-foreground"
                    }
                  >
                    {formatExternalSubject(binding.external_subject)}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {binding.scopes.map((scope) => (
                        <Badge
                          key={scope}
                          variant="secondary"
                          className="text-xs"
                        >
                          {scope}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(binding.created_at)}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {binding.last_used_at
                      ? formatDate(binding.last_used_at)
                      : "—"}
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8"
                      onClick={() => setRevokeBindingHash(binding.binding_hash)}
                    >
                      <Trash2 className="h-4 w-4 text-destructive" />
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
        open={revokeBindingHash !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeBindingHash(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke Authorization</DialogTitle>
            <DialogDescription>
              {`Are you sure you want to revoke the authorization for "${
                revokeTarget?.client_name ?? "this app"
              }"? The app will need to ask for your consent again before it can act on your behalf.`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setRevokeBindingHash(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevoke()}
              isLoading={revokeMutation.isPending}
            >
              Revoke
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
