import { useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { useMyConsents, useRevokeConsent } from "@/hooks/use-consents";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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
import { Trash2 } from "lucide-react";
import { SmartLockIcon, BiometricLockIcon } from "@/components/icons/empty-state";
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

export function ConsentsPage() {
  const searchParams = useRouterState({ select: (s) => s.location.search as Record<string, unknown> });
  const tabParam = typeof searchParams.tab === "string" ? searchParams.tab : undefined;
  const defaultTab = tabParam === "authorizations" ? "authorizations" : "apps";

  return (
    <div className="space-y-8">
      <PageHeader
        title="Access & Authorizations"
        description="Manage apps with access to your account and credentials held on your behalf."
      />

      <Tabs defaultValue={defaultTab} className="space-y-6">
        <TabsList>
          <TabsTrigger value="apps">Authorized Apps</TabsTrigger>
          <TabsTrigger value="authorizations">Authorizations</TabsTrigger>
        </TabsList>

        <TabsContent value="apps">
          <AuthorizedAppsTab />
        </TabsContent>
        <TabsContent value="authorizations">
          <AuthorizationsTab />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function AuthorizedAppsTab() {
  const { data, isLoading, error, refetch } = useMyConsents();
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

  if (isLoading) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton
            key={`consent-skel-${String(i)}`}
            className="h-12 w-full"
          />
        ))}
      </div>
    );
  }

  if (error) {
    return <ErrorBanner message="Failed to load consents. Please try again." onRetry={refetch} />;
  }

  if (consents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
        <SmartLockIcon className="h-64 w-64 text-muted-foreground/30" />
        <div className="space-y-1">
          <p className="text-[12px] font-medium text-muted-foreground/30">No Authorized Apps</p>
          <p className="text-xs text-muted-foreground/30">
            No applications have been authorized.
          </p>
        </div>
      </div>
    );
  }

  return (
    <>
      {/* Mobile card view */}
      <div className="flex flex-col gap-3 md:hidden">
        {consents.map((consent) => (
          <div
            key={consent.id}
            className="relative rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="absolute right-3 top-3">
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                onClick={() => setRevokeClientId(consent.client_id)}
              >
                <Trash2 className="h-4 w-4 text-destructive" />
              </Button>
            </div>
            <p className="pr-10 text-[13px] font-bold">
              {consent.client_name}
            </p>
            <div className="mt-2 flex flex-wrap gap-1">
              {consent.scopes.split(" ").map((scope) => (
                <Badge key={scope} variant="secondary" className="text-xs">
                  {scope}
                </Badge>
              ))}
            </div>
            <div className="mt-3 space-y-1">
              <p className="text-[11px] text-muted-foreground">
                <span className="font-medium">Granted:</span>{" "}
                {formatDate(consent.granted_at)}
              </p>
              <p className="text-[11px] text-muted-foreground">
                <span className="font-medium">Expires:</span>{" "}
                {consent.expires_at
                  ? formatDate(consent.expires_at)
                  : "Never"}
              </p>
            </div>
          </div>
        ))}
      </div>

      {/* Desktop table view */}
      <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Application</TableHead>
              <TableHead>Scopes</TableHead>
              <TableHead>Granted</TableHead>
              <TableHead>Expires</TableHead>
              <TableHead className="w-[60px]">Actions</TableHead>
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
                        variant="secondary"
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
                    <Trash2 className="h-4 w-4 text-destructive" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

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
              {revokeTarget
                ? `"${revokeTarget.client_name}"`
                : "this application"}
              ? The application will no longer be able to access your account
              with the granted scopes.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRevokeClientId(null)}>
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
    </>
  );
}

function AuthorizationsTab() {
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

  if (isLoading) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton
            key={`authorization-skel-${String(i)}`}
            className="h-12 w-full"
          />
        ))}
      </div>
    );
  }

  if (error) {
    return <ErrorBanner message="Failed to load authorizations. Please try again." onRetry={refetch} />;
  }

  if (bindings.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
        <BiometricLockIcon className="h-64 w-64 text-muted-foreground/30" />
        <div className="space-y-1">
          <p className="text-[12px] font-medium text-muted-foreground/30">No Authorizations</p>
          <p className="text-xs text-muted-foreground/30">
            No broker authorizations issued. Apps that hold server-side credentials on your behalf will appear here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <>
      {/* Mobile card view */}
      <div className="flex flex-col gap-3 md:hidden">
        {bindings.map((binding) => (
          <div
            key={binding.binding_hash}
            className="relative rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="absolute right-3 top-3">
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                onClick={() => setRevokeBindingHash(binding.binding_hash)}
              >
                <Trash2 className="h-4 w-4 text-destructive" />
              </Button>
            </div>
            <p className="pr-10 text-[13px] font-bold">
              {binding.client_name ?? binding.client_id}
            </p>
            <p
              className={`mt-1 text-[11px] ${
                binding.external_subject
                  ? "text-foreground"
                  : "text-muted-foreground"
              }`}
            >
              {formatExternalSubject(binding.external_subject)}
            </p>
            <div className="mt-2 flex flex-wrap gap-1">
              {binding.scopes.map((scope) => (
                <Badge key={scope} variant="secondary" className="text-xs">
                  {scope}
                </Badge>
              ))}
            </div>
            <div className="mt-3 space-y-1">
              <p className="text-[11px] text-muted-foreground">
                <span className="font-medium">Created:</span>{" "}
                {formatDate(binding.created_at)}
              </p>
              <p className="text-[11px] text-muted-foreground">
                <span className="font-medium">Last used:</span>{" "}
                {binding.last_used_at
                  ? formatDate(binding.last_used_at)
                  : "—"}
              </p>
            </div>
          </div>
        ))}
      </div>

      {/* Desktop table view */}
      <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
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
    </>
  );
}
