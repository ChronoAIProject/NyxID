import { ServiceScopeCard } from "@/components/dashboard/api-key-detail/service-scope-card";
import { useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { useApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import { maskApiKey } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { RefreshCw, Trash2 } from "lucide-react";
import { DetailsCard } from "@/components/dashboard/api-key-detail/details-card";
import { NodeScopeCard } from "@/components/dashboard/api-key-detail/node-scope-card";
import { RotateKeyDialog } from "@/components/dashboard/api-key-detail/rotate-key-dialog";
import { DeleteKeyDialog } from "@/components/dashboard/api-key-detail/delete-key-dialog";
import { PlatformCard } from "@/components/dashboard/api-key-detail/platform-card";
import { CallbackUrlCard } from "@/components/dashboard/api-key-detail/callback-url-card";
import { RateLimitCard } from "@/components/dashboard/api-key-detail/rate-limit-card";
import { BindingsCard } from "@/components/dashboard/api-key-detail/bindings-card";
import { UsageStatsCard } from "@/components/dashboard/api-key-detail/usage-stats-card";
import { VerifyKeyCard } from "@/components/dashboard/api-key-detail/verify-key-card";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { LoginCredentialsSection } from "@/components/dashboard/api-key-detail/login-credentials-section";

export function ApiKeyDetailPage() {
  const { keyId } = useParams({ strict: false }) as { keyId: string };
  const { data: apiKey, isLoading, error, refetch } = useApiKey(keyId);
  useBreadcrumbLabel(apiKey?.name);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const canWrite = apiKey != null &&
    (apiKey.credential_source?.type !== "org" ||
      apiKey.credential_source.role === "admin");

  if (isLoading) {
    return (
      <div className="space-y-8">
        <Skeleton className="h-20 w-full" />
        <div className="grid gap-4 md:grid-cols-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-48" />
          ))}
        </div>
      </div>
    );
  }

  if (error || !apiKey) {
    return (
      <div className="space-y-8">
        <PageHeader
          title="Key Not Found"
        />
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : "Failed to load key details."
          }
          onRetry={refetch}
        />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={apiKey.name}
        description={
          apiKey.description ?? `API key ${maskApiKey(apiKey.key_prefix)}`
        }
        actions={
          canWrite &&
          <div className="flex gap-2">
            <Button
              variant="outline"
              className="text-text-tertiary hover:text-muted-foreground"
              onClick={() => setRotateOpen(true)}
              disabled={!apiKey.is_active}
            >
              <ButtonIcon><RefreshCw className="h-3 w-3" /></ButtonIcon>
              Rotate Key
            </Button>
            <Button
              variant="destructive"
              onClick={() => setDeleteOpen(true)}
            >
              <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>
              Revoke
            </Button>
          </div>
        }
      />

      {apiKey.assistant_conversation_id ? (
        <p className="text-[12px] text-muted-foreground">
          Used by assistant chat · {" "}
          <Link
            to="/assistant"
            search={{ c: apiKey.assistant_conversation_id }}
            className="text-primary hover:underline"
          >
            Open chat
          </Link>
        </p>
      ) : null}
      <div className="grid gap-4 md:grid-cols-2">
        <DetailsCard
          name={apiKey.name}
          description={apiKey.description}
          keyPrefix={apiKey.key_prefix}
          scopes={apiKey.scopes}
          isActive={apiKey.is_active}
          createdAt={apiKey.created_at}
          lastUsedAt={apiKey.last_used_at}
          expiresAt={apiKey.expires_at}
          keyId={apiKey.id}
        />

        <PlatformCard keyId={apiKey.id} platform={apiKey.platform} />
        <CallbackUrlCard keyId={apiKey.id} callbackUrl={apiKey.callback_url} />

        <ServiceScopeCard
          canWrite={canWrite}
          keyId={apiKey.id}
          allowAllServices={apiKey.allow_all_services}
          allowAutoConnectedServices={apiKey.allow_auto_connected_services}
          allowedServiceIds={apiKey.allowed_service_ids}
          allowedServices={apiKey.allowed_services}
          apiKeySource={apiKey.credential_source}
        />

        <NodeScopeCard
          keyId={apiKey.id}
          allowAllNodes={apiKey.allow_all_nodes}
          allowedNodeIds={apiKey.allowed_node_ids}
          allowedNodes={apiKey.allowed_nodes}
          apiKeySource={apiKey.credential_source}
        />

        <RateLimitCard
          keyId={apiKey.id}
          rateLimitPerSecond={apiKey.rate_limit_per_second}
          rateLimitBurst={apiKey.rate_limit_burst}
        />

        <BindingsCard
          keyId={apiKey.id}
          apiKeySource={apiKey.credential_source}
        />
        <VerifyKeyCard apiKey={apiKey} />
        <UsageStatsCard keyId={apiKey.id} />
      </div>

      <LoginCredentialsSection keyId={apiKey.id} canWrite={canWrite} />

      <RotateKeyDialog
        open={canWrite && rotateOpen}
        onOpenChange={setRotateOpen}
        keyId={apiKey.id}
      />

      <DeleteKeyDialog
        open={canWrite && deleteOpen}
        onOpenChange={setDeleteOpen}
        keyId={apiKey.id}
        keyName={apiKey.name}
      />
    </div>
  );
}
