import { useState } from "react";
import { useParams } from "@tanstack/react-router";
import { useApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import { maskApiKey } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { RefreshCw, Trash2 } from "lucide-react";
import { DetailsCard } from "@/components/dashboard/api-key-detail/details-card";
import { ServiceScopeCard } from "@/components/dashboard/api-key-detail/service-scope-card";
import { NodeScopeCard } from "@/components/dashboard/api-key-detail/node-scope-card";
import { RotateKeyDialog } from "@/components/dashboard/api-key-detail/rotate-key-dialog";
import { DeleteKeyDialog } from "@/components/dashboard/api-key-detail/delete-key-dialog";
import { PlatformCard } from "@/components/dashboard/api-key-detail/platform-card";
import { CallbackUrlCard } from "@/components/dashboard/api-key-detail/callback-url-card";
import { RateLimitCard } from "@/components/dashboard/api-key-detail/rate-limit-card";
import { BindingsCard } from "@/components/dashboard/api-key-detail/bindings-card";
import { UsageStatsCard } from "@/components/dashboard/api-key-detail/usage-stats-card";

export function ApiKeyDetailPage() {
  const { keyId } = useParams({ strict: false }) as { keyId: string };
  const { data: apiKey, isLoading, error } = useApiKey(keyId);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

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
          breadcrumbs={[
            { label: "AI Services", to: "/keys" },
            { label: "Agent Keys", to: "/keys?tab=nyxid" },
            { label: "Not Found" },
          ]}
        />
        <Card>
          <CardContent className="py-8 text-center text-sm text-destructive">
            {error instanceof ApiError
              ? error.message
              : "Failed to load key details."}
          </CardContent>
        </Card>
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
        breadcrumbs={[
          { label: "AI Services", to: "/keys" },
          { label: "Agent Keys", to: "/keys?tab=nyxid" },
          { label: apiKey.name },
        ]}
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setRotateOpen(true)}
              disabled={!apiKey.is_active}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              Rotate Key
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setDeleteOpen(true)}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Revoke
            </Button>
          </div>
        }
      />

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
          keyId={apiKey.id}
          allowAllServices={apiKey.allow_all_services}
          allowedServiceIds={apiKey.allowed_service_ids}
          allowedServices={apiKey.allowed_services}
          apiKeySource={apiKey.credential_source}
        />

        <NodeScopeCard
          keyId={apiKey.id}
          allowAllNodes={apiKey.allow_all_nodes}
          allowedNodeIds={apiKey.allowed_node_ids}
          allowedNodes={apiKey.allowed_nodes}
        />

        <RateLimitCard
          keyId={apiKey.id}
          rateLimitPerSecond={apiKey.rate_limit_per_second}
          rateLimitBurst={apiKey.rate_limit_burst}
        />

        <BindingsCard
          keyId={apiKey.id}
          allowAllServices={apiKey.allow_all_services}
          apiKeySource={apiKey.credential_source}
        />
        <UsageStatsCard keyId={apiKey.id} />
      </div>

      <RotateKeyDialog
        open={rotateOpen}
        onOpenChange={setRotateOpen}
        keyId={apiKey.id}
      />

      <DeleteKeyDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        keyId={apiKey.id}
        keyName={apiKey.name}
      />
    </div>
  );
}
