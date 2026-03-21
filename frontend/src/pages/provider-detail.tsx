import { useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useProvider, useDeleteProvider } from "@/hooks/use-providers";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Separator } from "@/components/ui/separator";
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
import { Pencil, Trash2, AlertCircle } from "lucide-react";
import { toast } from "sonner";

const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  oauth2: "OAuth 2.0",
  api_key: "API Key",
  device_code: "Device Code",
  telegram_widget: "Telegram Widget",
};

export function ProviderDetailPage() {
  const { providerId } = useParams({ strict: false }) as {
    providerId: string;
  };
  const navigate = useNavigate();
  const { data: provider, isLoading, error } = useProvider(providerId);
  const deleteMutation = useDeleteProvider();
  const [deleteOpen, setDeleteOpen] = useState(false);

  async function handleDelete() {
    if (!provider) return;
    try {
      await deleteMutation.mutateAsync(provider.id);
      toast.success("Provider deleted successfully");
      void navigate({ to: "/providers/manage" });
    } catch {
      toast.error("Failed to delete provider");
    } finally {
      setDeleteOpen(false);
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

  if (error || !provider) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">Provider not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The provider you are looking for does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/providers/manage" })}
        >
          Back to Providers
        </Button>
      </div>
    );
  }

  const isOAuth = provider.provider_type === "oauth2";
  const isDeviceCode = provider.provider_type === "device_code";

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Manage Providers", to: "/providers/manage" },
          { label: provider.name },
        ]}
        title={provider.name}
        description={provider.description ?? undefined}
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void navigate({
                  to: "/providers/$providerId/edit",
                  params: { providerId },
                })
              }
            >
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setDeleteOpen(true)}
            >
              <Trash2 className="mr-1 h-3 w-3" />
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="General">
        <DetailRow label="Slug" value={provider.slug} copyable />
        <DetailRow
          label="Provider Type"
          value={
            PROVIDER_TYPE_LABELS[provider.provider_type] ??
            provider.provider_type
          }
          badge
        />
        <DetailRow
          label="Credential Mode"
          value={
            provider.credential_mode === "admin"
              ? "Admin Only"
              : provider.credential_mode === "user"
                ? "User Provided"
                : provider.credential_mode === "both"
                  ? "Admin or User"
                  : provider.credential_mode
          }
          badge
        />
        <DetailRow
          label="Status"
          value={provider.is_active ? "Active" : "Inactive"}
          badge
          badgeVariant={provider.is_active ? "success" : "secondary"}
        />
        <DetailRow label="Created" value={formatDate(provider.created_at)} />
        <DetailRow label="Updated" value={formatDate(provider.updated_at)} />
      </DetailSection>

      {isOAuth && (
        <>
          <Separator />
          <DetailSection title="OAuth 2.0 Configuration">
            <DetailRow
              label="OAuth Configured"
              value={provider.has_oauth_config ? "Yes" : "No"}
              badge
              badgeVariant={provider.has_oauth_config ? "success" : "secondary"}
            />
            <DetailRow
              label="Supports PKCE"
              value={provider.supports_pkce ? "Yes" : "No"}
              badge
              badgeVariant={provider.supports_pkce ? "success" : "secondary"}
            />
            {provider.default_scopes &&
              provider.default_scopes.length > 0 && (
                <div className="flex items-start justify-between text-sm">
                  <span className="text-muted-foreground">Default Scopes</span>
                  <div className="flex flex-wrap gap-1 justify-end max-w-[60%]">
                    {provider.default_scopes.map((scope) => (
                      <Badge key={scope} variant="outline">
                        {scope}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
          </DetailSection>
        </>
      )}

      {isDeviceCode && (
        <>
          <Separator />
          <DetailSection title="Device Code Configuration (RFC 8628)">
            <DetailRow
              label="OAuth Configured"
              value={provider.has_oauth_config ? "Yes" : "No"}
              badge
              badgeVariant={provider.has_oauth_config ? "success" : "secondary"}
            />
            {provider.device_code_url && (
              <DetailRow
                label="Device Code URL"
                value={provider.device_code_url}
                copyable
              />
            )}
            {provider.device_token_url && (
              <DetailRow
                label="Device Token URL"
                value={provider.device_token_url}
                copyable
              />
            )}
            {provider.hosted_callback_url && (
              <DetailRow
                label="Hosted Callback URL (legacy)"
                value={provider.hosted_callback_url}
                copyable
              />
            )}
            {provider.default_scopes &&
              provider.default_scopes.length > 0 && (
                <div className="flex items-start justify-between text-sm">
                  <span className="text-muted-foreground">Default Scopes</span>
                  <div className="flex flex-wrap gap-1 justify-end max-w-[60%]">
                    {provider.default_scopes.map((scope) => (
                      <Badge key={scope} variant="outline">
                        {scope}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
          </DetailSection>
        </>
      )}

      {!isOAuth && !isDeviceCode && (
        <>
          <Separator />
          <DetailSection title="API Key Configuration">
            {provider.api_key_instructions && (
              <div className="text-sm">
                <span className="text-muted-foreground block mb-1">
                  Instructions
                </span>
                <p className="whitespace-pre-wrap text-sm">
                  {provider.api_key_instructions}
                </p>
              </div>
            )}
            {provider.api_key_url && (
              <DetailRow
                label="API Key URL"
                value={provider.api_key_url}
                copyable
              />
            )}
          </DetailSection>
        </>
      )}

      {(provider.icon_url || provider.documentation_url) && (
        <>
          <Separator />
          <DetailSection title="Display">
            {provider.icon_url && (
              <DetailRow
                label="Icon URL"
                value={provider.icon_url}
                copyable
              />
            )}
            {provider.documentation_url && (
              <DetailRow
                label="Documentation URL"
                value={provider.documentation_url}
                copyable
              />
            )}
          </DetailSection>
        </>
      )}

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Provider</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{provider.name}&quot;? This
              will deactivate the provider and revoke all user tokens. This
              action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
