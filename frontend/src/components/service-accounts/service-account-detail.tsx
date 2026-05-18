import { useState, useEffect } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useServiceAccount,
  useUpdateServiceAccount,
  useDeleteServiceAccount,
  useRotateSecret,
  useRevokeTokens,
} from "@/hooks/use-service-accounts";
import {
  updateServiceAccountSchema,
  type UpdateServiceAccountFormData,
} from "@/schemas/service-accounts";
import { formatDate, copyToClipboard } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { SaConnectedServices } from "@/components/dashboard/sa-connected-services";
import type { RotateSecretResponse } from "@/types/service-accounts";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import {
  Pencil,
  Trash2,
  RefreshCw,
  Ban,
  AlertCircle,
  Copy,
  AlertTriangle,
} from "lucide-react";
import { toast } from "sonner";

type ConfirmAction = "delete" | "revoke-tokens" | null;

interface ServiceAccountDetailProps {
  readonly saId: string;
  readonly backTo: { readonly to: string; readonly label: string };
  readonly showProviderSections?: boolean;
}

export function ServiceAccountDetail({
  saId,
  backTo,
  showProviderSections = true,
}: ServiceAccountDetailProps) {
  const navigate = useNavigate();

  const { data: sa, isLoading, error } = useServiceAccount(saId);

  const updateMutation = useUpdateServiceAccount();
  const deleteMutation = useDeleteServiceAccount();
  const rotateMutation = useRotateSecret();
  const revokeMutation = useRevokeTokens();

  const [editOpen, setEditOpen] = useState(false);
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [rotateResult, setRotateResult] = useState<RotateSecretResponse | null>(
    null,
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

    const newRoleIds = formData.role_ids
      ? formData.role_ids
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean)
      : [];
    const roleIdsChanged =
      JSON.stringify([...sa.role_ids].sort()) !==
      JSON.stringify([...newRoleIds].sort());

    const newRate = formData.rate_limit_override
      ? Number(formData.rate_limit_override)
      : null;

    const payload = {
      ...(formData.name !== sa.name ? { name: formData.name } : {}),
      ...((formData.description ?? "") !== (sa.description ?? "")
        ? { description: formData.description || undefined }
        : {}),
      ...(formData.allowed_scopes !== sa.allowed_scopes
        ? { allowed_scopes: formData.allowed_scopes }
        : {}),
      ...(roleIdsChanged ? { role_ids: newRoleIds } : {}),
      ...(newRate !== sa.rate_limit_override
        ? { rate_limit_override: newRate }
        : {}),
      ...(formData.is_active !== sa.is_active
        ? { is_active: formData.is_active }
        : {}),
    };

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
      toast.success(`${String(result.revoked_count)} token(s) revoked`);
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
      void navigate({ to: backTo.to });
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to delete service account",
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
        <h3 className="mb-2 text-lg font-semibold">
          Service account not found
        </h3>
        <p className="mb-4 text-[12px] text-muted-foreground">
          The service account you are looking for does not exist or has been
          deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: backTo.to })}
        >
          Back to {backTo.label}
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={sa.name}
        description={sa.description ?? undefined}
        actions={
          <>
            <Button variant="outline" onClick={openEditDialog}>
              <ButtonIcon><Pencil className="h-3 w-3" /></ButtonIcon>
              Edit
            </Button>
            <Button
              variant="destructive"
              onClick={() => setConfirmAction("delete")}
            >
              <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="Service Account Information">
        <DetailRow label="ID" value={sa.id} copyable />
        <DetailRow label="Client ID" value={sa.client_id} copyable />
        <DetailRow
          label="Secret Prefix"
          value={`${sa.secret_prefix}...`}
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
          value={sa.role_ids.length > 0 ? sa.role_ids.join(", ") : "None"}
        />
        <DetailRow
          label="Rate Limit"
          value={
            sa.rate_limit_override
              ? `${String(sa.rate_limit_override)} req/s`
              : "Default"
          }
        />
        <DetailRow label="Created By" value={sa.created_by} />
        <DetailRow label="Created" value={formatDate(sa.created_at)} />
        <DetailRow label="Updated" value={formatDate(sa.updated_at)} />
        <DetailRow
          label="Last Authenticated"
          value={formatDate(sa.last_authenticated_at)}
        />
      </DetailSection>

      <Separator />

      {showProviderSections ? (
        <SaConnectedServices saId={saId} />
      ) : (
        <DetailSection title="Provider Connections">
          <p className="text-[12px] text-muted-foreground">
            Provider connections for org-owned service accounts aren't yet
            available here. Use the{" "}
            <code className="rounded bg-muted px-1 font-mono text-xs">
              nyxid
            </code>{" "}
            CLI or API for now.
          </p>
        </DetailSection>
      )}

      <Separator />

      <DetailSection title="Actions">
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={openRotateDialog}>
            <ButtonIcon><RefreshCw className="h-3 w-3" /></ButtonIcon>
            Rotate Secret
          </Button>
          <Button
            variant="outline"
            onClick={() => setConfirmAction("revoke-tokens")}
          >
            <ButtonIcon><Ban className="h-3 w-3" /></ButtonIcon>
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
                <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
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
                      <FormLabel className="text-[12px] font-medium">
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
                <Button variant="primary" type="submit" isLoading={updateMutation.isPending} disabled={!form.formState.isDirty || updateMutation.isPending}>
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
              <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
                <div className="flex items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 text-amber-600" />
                  <p className="text-[12px] text-amber-700 dark:text-amber-400">
                    Save this secret now. It cannot be retrieved later.
                  </p>
                </div>
              </div>

              <div>
                <p className="mb-1 text-xs font-medium text-muted-foreground">
                  New Client Secret
                </p>
                <div className="flex items-center gap-2">
                  <code className="flex-1 rounded bg-muted px-2 py-1 text-[12px] font-mono break-all">
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
                  variant="primary"
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
              <Button variant="outline" onClick={() => setRotateOpen(false)}>
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
