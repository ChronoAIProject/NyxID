import { useState } from "react";
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
import { useOrgs } from "@/hooks/use-orgs";
import { ProviderCard } from "./provider-card";
import { ApiKeyDialog } from "./api-key-dialog";
import { DeviceCodeDialog } from "./device-code-dialog";
import { TelegramLoginDialog } from "./telegram-login-dialog";
import { UserCredentialsDialog } from "./user-credentials-dialog";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  canConnectProvider,
  getProviderConnectHint,
  needsUserCredentials,
} from "@/lib/constants";
import { Building2, KeyRound } from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { hardRedirect } from "@/lib/navigation";

export function ProviderGrid() {
  const [targetOrgId, setTargetOrgId] = useState<string | null>(null);
  const { data: providers, isLoading: providersLoading } = useProviders();
  const { data: orgs, isLoading: orgsLoading } = useOrgs();
  const { data: tokens, isLoading: tokensLoading } = useMyProviderTokens({
    targetOrgId,
  });
  const { data: llmStatus } = useLlmStatus();
  const connectApiKeyMutation = useConnectApiKey();
  const initiateOAuthMutation = useInitiateOAuth();
  const disconnectMutation = useDisconnectProvider();
  const refreshMutation = useRefreshProviderToken();

  const [apiKeyDialog, setApiKeyDialog] = useState<ProviderConfig | null>(null);
  const [deviceCodeDialog, setDeviceCodeDialog] =
    useState<ProviderConfig | null>(null);
  const [telegramDialog, setTelegramDialog] =
    useState<ProviderConfig | null>(null);
  const [credentialsDialog, setCredentialsDialog] =
    useState<ProviderConfig | null>(null);
  // Track which provider is currently being acted upon for per-card disabled state
  const [activeProviderId, setActiveProviderId] = useState<string | null>(null);

  const isLoading = providersLoading || tokensLoading;

  function handleConnect(provider: ProviderConfig, hasUserCredentials = false) {
    if (!canConnectProvider(provider, hasUserCredentials)) {
      toast.error(
        getProviderConnectHint(provider, hasUserCredentials) ??
          "Provider is not ready to connect.",
      );
      return;
    }

    if (provider.provider_type === "api_key") {
      setApiKeyDialog(provider);
    } else if (provider.provider_type === "device_code") {
      setDeviceCodeDialog(provider);
    } else if (provider.provider_type === "telegram_widget") {
      setTelegramDialog(provider);
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

  async function handleApiKeySubmit(
    apiKey: string,
    label?: string,
    gatewayUrl?: string,
  ) {
    if (!apiKeyDialog) return;
    setActiveProviderId(apiKeyDialog.id);
    try {
      await connectApiKeyMutation.mutateAsync({
        providerId: apiKeyDialog.id,
        apiKey,
        label,
        gatewayUrl,
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
      await disconnectMutation.mutateAsync({ providerId, targetOrgId });
      toast.success(`Disconnected from ${provider?.name ?? "provider"}`);
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
      await refreshMutation.mutateAsync({ providerId, targetOrgId });
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
  const tokensByProviderId = new Map(
    tokens?.map((t) => [t.provider_id, t]) ?? [],
  );
  const adminOrgs = (orgs ?? []).filter((org) => org.your_role === "admin");
  const selectedOrg = adminOrgs.find((org) => org.id === targetOrgId);
  const selectedOrgName = selectedOrg?.display_name ?? selectedOrg?.id;
  const visibleProviders = targetOrgId
    ? activeProviders.filter((provider) => tokensByProviderId.has(provider.id))
    : activeProviders;
  const showScopeSelect = !orgsLoading && adminOrgs.length > 0;

  const scopeSelector = showScopeSelect ? (
    <div className="mb-4 rounded-lg border border-border bg-muted/30 px-3 py-2">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <Building2 className="h-3.5 w-3.5" />
          Owner
        </div>
        <div className="w-full sm:w-[220px]">
          <OrgScopeSelect
            value={targetOrgId}
            onChange={setTargetOrgId}
            label="Provider token owner"
            adminOnly
          />
        </div>
      </div>
      <p className="mt-1 text-[11px] text-muted-foreground">
        Select an org to review and disconnect provider tokens owned by that
        org.
      </p>
    </div>
  ) : null;

  if (visibleProviders.length === 0) {
    return (
      <>
        {scopeSelector}
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <KeyRound className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-[12px] font-medium">No Provider Tokens</p>
            <p className="text-xs text-muted-foreground">
              {targetOrgId
                ? selectedOrgName
                  ? `No provider tokens for ${selectedOrgName}.`
                  : "No org provider tokens connected."
                : "No providers available. An admin needs to configure providers first."}
            </p>
          </div>
        </div>
      </>
    );
  }

  const llmStatusBySlug = new Map(
    llmStatus?.providers.map((s) => [s.provider_slug, s]) ?? [],
  );

  const gatewayUrl = llmStatus?.gateway_url ?? "";

  return (
    <>
      {scopeSelector}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {visibleProviders.map((provider) => {
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
          onSubmit={(key, label, gatewayUrl) =>
            void handleApiKeySubmit(key, label, gatewayUrl)
          }
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

      {telegramDialog !== null && (
        <TelegramLoginDialog
          provider={telegramDialog}
          onClose={() => setTelegramDialog(null)}
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
  props: Omit<React.ComponentProps<typeof ProviderCard>, "hasUserCredentials">,
) {
  const showCreds = needsUserCredentials(props.provider);
  const { data: credentials } = useMyProviderCredentials(
    showCreds ? props.provider.id : "",
  );
  const hasUserCredentials = credentials?.has_credentials === true;

  return <ProviderCard {...props} hasUserCredentials={hasUserCredentials} />;
}
