import { useState, useEffect } from "react";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useServiceAccount,
  useUpdateServiceAccount,
  useDeleteServiceAccount,
  useRotateSecret,
  useRevokeTokens,
  useSaProviders,
  useConnectApiKeyForSa,
  useDisconnectSaProvider,
  useInitiateOAuthForSa,
} from "@/hooks/use-service-accounts";
import { useProviders } from "@/hooks/use-providers";
import {
  updateServiceAccountSchema,
  type UpdateServiceAccountFormData,
} from "@/schemas/service-accounts";
import { formatDate, copyToClipboard } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { ApiKeyDialog } from "@/components/dashboard/api-key-dialog";
import { SaDeviceCodeDialog } from "@/components/dashboard/sa-device-code-dialog";
import type { ProviderConfig } from "@/types/api";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Separator } from "@/components/ui/separator";
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
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
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
  Pencil,
  Trash2,
  RefreshCw,
  Ban,
  AlertCircle,
  Copy,
  AlertTriangle,
  Plug,
  Unlink,
  KeyRound,
  Globe,
  Smartphone,
} from "lucide-react";
import { toast } from "sonner";
import type { RotateSecretResponse } from "@/types/service-accounts";

type ConfirmAction = "delete" | "revoke-tokens" | null;

export function AdminServiceAccountDetailPage() {
  const { saId } = useParams({
    from: "/dashboard/admin/service-accounts/$saId",
  });
  const navigate = useNavigate();

  const { data: sa, isLoading, error } = useServiceAccount(saId);

  const updateMutation = useUpdateServiceAccount();
  const deleteMutation = useDeleteServiceAccount();
  const rotateMutation = useRotateSecret();
  const revokeMutation = useRevokeTokens();

  const { data: saProviders, isLoading: providersLoading } = useSaProviders(saId);
  const { data: allProviders } = useProviders();
  const connectApiKeyMutation = useConnectApiKeyForSa();
  const disconnectMutation = useDisconnectSaProvider();

  const [editOpen, setEditOpen] = useState(false);
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [rotateResult, setRotateResult] = useState<RotateSecretResponse | null>(
    null,
  );
  const [connectDialogProvider, setConnectDialogProvider] = useState<ProviderConfig | null>(null);
  const [deviceCodeDialogProvider, setDeviceCodeDialogProvider] = useState<ProviderConfig | null>(null);

  const initiateOAuthMutation = useInitiateOAuthForSa();

  const connectedProviderIds = new Set(saProviders?.map((t) => t.provider_id) ?? []);
  const availableProviders = (allProviders ?? []).filter(
    (p) => p.is_active && !connectedProviderIds.has(p.id),
  );

  // Handle OAuth callback redirect (provider_status query param)
  const search = useSearch({ strict: false }) as {
    readonly provider_status?: string;
    readonly message?: string;
  };

  useEffect(() => {
    if (search.provider_status === "success") {
      toast.success("Provider connected successfully");
      void navigate({ to: ".", search: {}, replace: true });
    } else if (search.provider_status === "error") {
      toast.error(search.message ?? "Failed to connect provider");
      void navigate({ to: ".", search: {}, replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.provider_status]);

  async function handleConnectApiKey(apiKey: string, label?: string) {
    if (!connectDialogProvider) return;
    try {
      await connectApiKeyMutation.mutateAsync({
        saId,
        providerId: connectDialogProvider.id,
        apiKey,
        label,
      });
      toast.success(`Connected ${connectDialogProvider.name}`);
      setConnectDialogProvider(null);
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to connect provider");
      }
    }
  }

  function handleConnect(provider: ProviderConfig) {
    if (provider.provider_type === "api_key") {
      setConnectDialogProvider(provider);
    } else if (provider.provider_type === "device_code") {
      setDeviceCodeDialogProvider(provider);
    } else {
      void handleOAuthConnect(provider);
    }
  }

  async function handleOAuthConnect(provider: ProviderConfig) {
    try {
      const response = await initiateOAuthMutation.mutateAsync({
        saId,
        providerId: provider.id,
      });
      window.location.href = response.authorization_url;
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to initiate OAuth connection");
      }
    }
  }

  async function handleDisconnectSaProvider(providerId: string) {
    try {
      await disconnectMutation.mutateAsync({ saId, providerId });
      toast.success("Provider disconnected");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to disconnect provider");
      }
    }
  }

  const form = useForm<UpdateServiceAccountFormData>({
    resolver: zodResolver(updateServiceAccountSchema),
    defaultValues: {
      name: "",
      description: "",
      allowed_scopes: "",
      role_ids: "",
      rate_limit_override: "",
      is_active: true,
    },
  });

  function openEditDialog() {
    if (!sa) return;
    form.reset({
      name: sa.name,
      description: sa.description ?? "",
      allowed_scopes: sa.allowed_scopes,
      role_ids: sa.role_ids.join(", "),
      rate_limit_override: sa.rate_limit_override
        ? String(sa.rate_limit_override)
        : "",
      is_active: sa.is_active,
    });
    setEditOpen(true);
  }

  async function handleEdit(formData: UpdateServiceAccountFormData) {
    if (!sa) return;
    const payload: Partial<{
      name: string;
      description: string;
      allowed_scopes: string;
      role_ids: string[];
      rate_limit_override: number | null;
      is_active: boolean;
    }> = {};

    if (formData.name !== sa.name) {
      payload.name = formData.name;
    }
    if ((formData.description ?? "") !== (sa.description ?? "")) {
      payload.description = formData.description || undefined;
    }
    if (formData.allowed_scopes !== sa.allowed_scopes) {
      payload.allowed_scopes = formData.allowed_scopes;
    }

    const newRoleIds = formData.role_ids
      ? formData.role_ids
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean)
      : [];
    const currentRoleIds = [...sa.role_ids].sort();
    const sortedNewRoleIds = [...newRoleIds].sort();
    if (JSON.stringify(currentRoleIds) !== JSON.stringify(sortedNewRoleIds)) {
      payload.role_ids = newRoleIds;
    }

    const newRate = formData.rate_limit_override
      ? Number(formData.rate_limit_override)
      : null;
    if (newRate !== sa.rate_limit_override) {
      payload.rate_limit_override = newRate;
    }

    if (formData.is_active !== sa.is_active) {
      payload.is_active = formData.is_active;
    }

    if (Object.keys(payload).length === 0) {
      setEditOpen(false);
      return;
    }

    try {
      await updateMutation.mutateAsync({ saId, data: payload });
      toast.success("Service account updated");
      setEditOpen(false);
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update service account");
      }
    }
  }

  async function handleRotateSecret() {
    try {
      const result = await rotateMutation.mutateAsync(saId);
      setRotateResult(result);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to rotate secret",
      );
    }
  }

  function openRotateDialog() {
    setRotateResult(null);
    setRotateOpen(true);
  }

  async function handleRevokeTokens() {
    try {
      const result = await revokeMutation.mutateAsync(saId);
      toast.success(
        `${String(result.revoked_count)} token(s) revoked`,
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke tokens",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(saId);
      toast.success("Service account deleted");
      void navigate({ to: "/admin/service-accounts" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete service account",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !sa) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 text-lg font-semibold">Service account not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The service account you are looking for does not exist or has been
          deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/admin/service-accounts" })}
        >
          Back to Service Accounts
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        breadcrumbs={[
          { label: "Service Accounts", to: "/admin/service-accounts" },
          { label: sa.name },
        ]}
        title={sa.name}
        description={sa.description ?? undefined}
        actions={
          <>
            <Button variant="outline" size="sm" onClick={openEditDialog}>
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setConfirmAction("delete")}
            >
              <Trash2 className="mr-1 h-3 w-3" />
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="Service Account Information">
        <DetailRow label="ID" value={sa.id} copyable mono />
        <DetailRow label="Client ID" value={sa.client_id} copyable mono />
        <DetailRow
          label="Secret Prefix"
          value={`${sa.secret_prefix}...`}
          mono
        />
        <DetailRow
          label="Status"
          value={sa.is_active ? "Active" : "Inactive"}
          badge
          badgeVariant={sa.is_active ? "success" : "destructive"}
        />
        <DetailRow label="Allowed Scopes" value={sa.allowed_scopes} />
        <DetailRow
          label="Role IDs"
          value={
            sa.role_ids.length > 0 ? sa.role_ids.join(", ") : "None"
          }
        />
        <DetailRow
          label="Rate Limit"
          value={
            sa.rate_limit_override
              ? `${String(sa.rate_limit_override)} req/s`
              : "Default"
          }
        />
        <DetailRow label="Created By" value={sa.created_by} mono />
        <DetailRow label="Created" value={formatDate(sa.created_at)} />
        <DetailRow label="Updated" value={formatDate(sa.updated_at)} />
        <DetailRow
          label="Last Authenticated"
          value={formatDate(sa.last_authenticated_at)}
        />
      </DetailSection>

      <Separator />

      <DetailSection title="Connected Providers">
        {providersLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : saProviders && saProviders.length > 0 ? (
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Provider</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Label</TableHead>
                  <TableHead>Connected</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {saProviders.map((token) => (
                  <TableRow key={token.provider_id}>
                    <TableCell className="font-medium">{token.provider_name}</TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {providerTypeLabel(token.provider_type)}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={token.status === "active" ? "success" : "secondary"}>
                        {token.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {token.label ?? "-"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(token.connected_at)}
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => void handleDisconnectSaProvider(token.provider_id)}
                        disabled={disconnectMutation.isPending}
                      >
                        <Unlink className="mr-1 h-3 w-3" />
                        Disconnect
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No providers connected to this service account.
          </p>
        )}

        {availableProviders.length > 0 && (
          <div className="mt-3">
            <ConnectProviderDropdown
              providers={availableProviders}
              onSelect={handleConnect}
            />
          </div>
        )}
      </DetailSection>

      {connectDialogProvider !== null && (
        <ApiKeyDialog
          provider={connectDialogProvider}
          onSubmit={(apiKey, label) => void handleConnectApiKey(apiKey, label)}
          onCancel={() => setConnectDialogProvider(null)}
          isPending={connectApiKeyMutation.isPending}
        />
      )}

      {deviceCodeDialogProvider !== null && (
        <SaDeviceCodeDialog
          saId={saId}
          provider={deviceCodeDialogProvider}
          onClose={() => setDeviceCodeDialogProvider(null)}
        />
      )}

      <Separator />

      <DetailSection title="Actions">
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={openRotateDialog}>
            <RefreshCw className="mr-1 h-3 w-3" />
            Rotate Secret
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirmAction("revoke-tokens")}
          >
            <Ban className="mr-1 h-3 w-3" />
            Revoke Tokens
          </Button>
        </div>
      </DetailSection>

      {/* Edit Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Service Account</DialogTitle>
            <DialogDescription>
              Update configuration for {sa.name}.
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form
              onSubmit={form.handleSubmit((data) => void handleEdit(data))}
              className="space-y-4"
            >
              {form.formState.errors.root && (
                <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                  {form.formState.errors.root.message}
                </div>
              )}
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input placeholder="Service account name" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Description</FormLabel>
                    <FormControl>
                      <Input placeholder="Optional description" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="allowed_scopes"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Allowed Scopes</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="e.g. openid proxy:* llm:proxy"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="role_ids"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Role IDs (comma-separated)</FormLabel>
                    <FormControl>
                      <Input placeholder="Optional" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="rate_limit_override"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Rate Limit Override</FormLabel>
                    <FormControl>
                      <Input
                        type="number"
                        placeholder="Requests per second (empty for default)"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="is_active"
                render={({ field }) => (
                  <FormItem className="flex items-center justify-between rounded-lg border p-3">
                    <div className="space-y-0.5">
                      <FormLabel className="text-sm font-medium">
                        Active
                      </FormLabel>
                      <p className="text-xs text-muted-foreground">
                        Inactive accounts cannot authenticate.
                      </p>
                    </div>
                    <FormControl>
                      <Switch
                        checked={field.value}
                        onCheckedChange={field.onChange}
                      />
                    </FormControl>
                  </FormItem>
                )}
              />
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setEditOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" isLoading={updateMutation.isPending}>
                  Save Changes
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Rotate Secret Dialog */}
      <Dialog
        open={rotateOpen}
        onOpenChange={(open) => {
          if (!open) {
            setRotateOpen(false);
            setRotateResult(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rotate Client Secret</DialogTitle>
            <DialogDescription>
              {rotateResult
                ? "The secret has been rotated. All existing tokens have been revoked."
                : "This will generate a new client secret and revoke all existing tokens. The old secret will stop working immediately."}
            </DialogDescription>
          </DialogHeader>

          {rotateResult ? (
            <div className="space-y-4">
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3">
                <div className="flex items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 text-amber-600" />
                  <p className="text-sm text-amber-700 dark:text-amber-400">
                    Save this secret now. It cannot be retrieved later.
                  </p>
                </div>
              </div>

              <div>
                <p className="mb-1 text-xs font-medium text-muted-foreground">
                  New Client Secret
                </p>
                <div className="flex items-center gap-2">
                  <code className="flex-1 rounded bg-muted px-2 py-1 text-sm font-mono break-all">
                    {rotateResult.client_secret}
                  </code>
                  <Button
                    variant="outline"
                    size="icon"
                    className="h-8 w-8"
                    onClick={() =>
                      void copyToClipboard(rotateResult.client_secret).then(
                        () => toast.success("Secret copied"),
                        () => toast.error("Failed to copy"),
                      )
                    }
                  >
                    <Copy className="h-3 w-3" />
                  </Button>
                </div>
              </div>

              <DialogFooter>
                <Button
                  onClick={() => {
                    setRotateOpen(false);
                    setRotateResult(null);
                  }}
                >
                  Done
                </Button>
              </DialogFooter>
            </div>
          ) : (
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => setRotateOpen(false)}
              >
                Cancel
              </Button>
              <Button
                variant="destructive"
                isLoading={rotateMutation.isPending}
                onClick={() => void handleRotateSecret()}
              >
                Rotate Secret
              </Button>
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>

      {/* Confirm Delete Dialog */}
      <ConfirmDialog
        open={confirmAction === "delete"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Delete Service Account"
        description={`Are you sure you want to permanently delete "${sa.name}"? All tokens will be revoked and the client credentials will stop working immediately. This cannot be undone.`}
        confirmLabel="Delete Service Account"
        variant="destructive"
        isPending={deleteMutation.isPending}
        onConfirm={() => void handleDelete()}
      />

      {/* Confirm Revoke Tokens Dialog */}
      <ConfirmDialog
        open={confirmAction === "revoke-tokens"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Revoke All Tokens"
        description={`Are you sure you want to revoke all active tokens for "${sa.name}"? The service account will need to re-authenticate.`}
        confirmLabel="Revoke Tokens"
        variant="destructive"
        isPending={revokeMutation.isPending}
        onConfirm={() => void handleRevokeTokens()}
      />
    </div>
  );
}

function ProviderTypeIcon({ type }: { readonly type: string }) {
  if (type === "api_key") return <KeyRound className="mr-2 h-4 w-4" />;
  if (type === "oauth2") return <Globe className="mr-2 h-4 w-4" />;
  return <Smartphone className="mr-2 h-4 w-4" />;
}

function providerTypeLabel(type: string): string {
  if (type === "api_key") return "API Key";
  if (type === "oauth2") return "OAuth";
  return "Device Code";
}

function ConnectProviderDropdown({
  providers,
  onSelect,
}: {
  readonly providers: readonly ProviderConfig[];
  readonly onSelect: (provider: ProviderConfig) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <Plug className="mr-1 h-3 w-3" />
          Connect Provider
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        {providers.map((p) => (
          <DropdownMenuItem key={p.id} onClick={() => onSelect(p)}>
            <ProviderTypeIcon type={p.provider_type} />
            <span>{p.name}</span>
            <Badge variant="outline" className="ml-auto text-xs">
              {providerTypeLabel(p.provider_type)}
            </Badge>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// Generic confirmation dialog. Consider extracting to
// components/shared/confirm-dialog.tsx if reuse is needed elsewhere.
interface ConfirmDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
  readonly variant: "default" | "destructive";
  readonly isPending: boolean;
  readonly onConfirm: () => void;
}

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  variant,
  isPending,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant={variant} onClick={onConfirm} isLoading={isPending}>
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
