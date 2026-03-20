import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import type { ProviderConfig } from "@/types/api";
import {
  useProviders,
  useMyProviderTokens,
  useConnectApiKey,
  useInitiateOAuth,
  useDisconnectProvider,
  useRefreshProviderToken,
  useMyProviderCredentials,
} from "@/hooks/use-providers";
import { useLlmStatus } from "@/hooks/use-llm-gateway";
import { ProviderCard } from "./provider-card";
import { ApiKeyDialog } from "./api-key-dialog";
import { DeviceCodeDialog } from "./device-code-dialog";
import { UserCredentialsDialog } from "./user-credentials-dialog";
import { Skeleton } from "@/components/ui/skeleton";
import {
  canConnectProvider,
  getProviderConnectHint,
  needsUserCredentials,
} from "@/lib/constants";
import { KeyRound } from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { hardRedirect } from "@/lib/navigation";

export function ProviderGrid() {
  const navigate = useNavigate();
  const { data: providers, isLoading: providersLoading } = useProviders();
  const { data: tokens, isLoading: tokensLoading } = useMyProviderTokens();
  const { data: llmStatus } = useLlmStatus();
  const connectApiKeyMutation = useConnectApiKey();
  const initiateOAuthMutation = useInitiateOAuth();
  const disconnectMutation = useDisconnectProvider();
  const refreshMutation = useRefreshProviderToken();

  const [apiKeyDialog, setApiKeyDialog] = useState<ProviderConfig | null>(
    null,
  );
  const [deviceCodeDialog, setDeviceCodeDialog] =
    useState<ProviderConfig | null>(null);
  const [credentialsDialog, setCredentialsDialog] =
    useState<ProviderConfig | null>(null);
  // Track which provider is currently being acted upon for per-card disabled state
  const [activeProviderId, setActiveProviderId] = useState<string | null>(null);

  const isLoading = providersLoading || tokensLoading;

  function handleConnect(provider: ProviderConfig, hasUserCredentials = false) {
    if (!canConnectProvider(provider, hasUserCredentials)) {
      toast.error(
        getProviderConnectHint(provider, hasUserCredentials) ?? "Provider is not ready to connect.",
      );
      return;
    }

    if (provider.provider_type === "api_key") {
      setApiKeyDialog(provider);
    } else if (provider.provider_type === "device_code") {
      setDeviceCodeDialog(provider);
    } else if (provider.provider_type === "telegram_widget") {
      void navigate({
        to: "/providers/$providerId/connect/telegram",
        params: { providerId: provider.id },
      });
    } else {
      void handleOAuthConnect(provider.id);
    }
  }

  async function handleOAuthConnect(providerId: string) {
    setActiveProviderId(providerId);
    try {
      const response = await initiateOAuthMutation.mutateAsync(providerId);
      hardRedirect(response.authorization_url);
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to initiate OAuth connection");
      }
    } finally {
      setActiveProviderId(null);
    }
  }

  async function handleApiKeySubmit(apiKey: string, label?: string) {
    if (!apiKeyDialog) return;
    setActiveProviderId(apiKeyDialog.id);
    try {
      await connectApiKeyMutation.mutateAsync({
        providerId: apiKeyDialog.id,
        apiKey,
        label,
      });
      toast.success(`Connected to ${apiKeyDialog.name}`);
      setApiKeyDialog(null);
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to connect API key");
      }
    } finally {
      setActiveProviderId(null);
    }
  }

  async function handleDisconnect(providerId: string) {
    const provider = providers?.find((p) => p.id === providerId);
    setActiveProviderId(providerId);
    try {
      await disconnectMutation.mutateAsync(providerId);
      toast.success(
        `Disconnected from ${provider?.name ?? "provider"}`,
      );
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to disconnect provider");
      }
    } finally {
      setActiveProviderId(null);
    }
  }

  async function handleRefresh(providerId: string) {
    setActiveProviderId(providerId);
    try {
      await refreshMutation.mutateAsync(providerId);
      toast.success("Token refreshed");
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to refresh token");
      }
    } finally {
      setActiveProviderId(null);
    }
  }

  if (isLoading) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {Array.from({ length: 6 }).map((_, i) => (
          <Skeleton key={`prov-skel-${String(i)}`} className="h-40 w-full" />
        ))}
      </div>
    );
  }

  const activeProviders = providers?.filter((p) => p.is_active) ?? [];

  if (activeProviders.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <KeyRound className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">
          No providers available. An admin needs to configure providers first.
        </p>
      </div>
    );
  }

  const tokensByProviderId = new Map(
    tokens?.map((t) => [t.provider_id, t]) ?? [],
  );

  const llmStatusBySlug = new Map(
    llmStatus?.providers.map((s) => [s.provider_slug, s]) ?? [],
  );

  const gatewayUrl = llmStatus?.gateway_url ?? "";

  return (
    <>
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {activeProviders.map((provider) => {
          const isActive = activeProviderId === provider.id;
          return (
            <ProviderCardWithCredentials
              key={provider.id}
              provider={provider}
              token={tokensByProviderId.get(provider.id)}
              llmStatus={llmStatusBySlug.get(provider.slug)}
              gatewayUrl={gatewayUrl}
              onConnect={handleConnect}
              onDisconnect={(id) => void handleDisconnect(id)}
              onRefresh={(id) => void handleRefresh(id)}
              onSetupCredentials={setCredentialsDialog}
              isConnecting={
                isActive &&
                (connectApiKeyMutation.isPending ||
                  initiateOAuthMutation.isPending)
              }
              isDisconnecting={isActive && disconnectMutation.isPending}
              isRefreshing={isActive && refreshMutation.isPending}
            />
          );
        })}
      </div>

      {apiKeyDialog !== null && (
        <ApiKeyDialog
          provider={apiKeyDialog}
          onSubmit={(key, label) => void handleApiKeySubmit(key, label)}
          onCancel={() => setApiKeyDialog(null)}
          isPending={connectApiKeyMutation.isPending}
        />
      )}

      {deviceCodeDialog !== null && (
        <DeviceCodeDialog
          provider={deviceCodeDialog}
          onClose={() => setDeviceCodeDialog(null)}
        />
      )}

      {credentialsDialog !== null && (
        <UserCredentialsDialog
          provider={credentialsDialog}
          onClose={() => setCredentialsDialog(null)}
        />
      )}
    </>
  );
}

/**
 * Wrapper that fetches per-user credential status for a provider,
 * then renders the presentational ProviderCard with credential data.
 */
function ProviderCardWithCredentials(
  props: Omit<
    React.ComponentProps<typeof ProviderCard>,
    "hasUserCredentials"
  >,
) {
  const showCreds = needsUserCredentials(props.provider);
  const { data: credentials } = useMyProviderCredentials(
    showCreds ? props.provider.id : "",
  );
  const hasUserCredentials = credentials?.has_credentials === true;

  return (
    <ProviderCard
      {...props}
      hasUserCredentials={hasUserCredentials}
    />
  );
}
