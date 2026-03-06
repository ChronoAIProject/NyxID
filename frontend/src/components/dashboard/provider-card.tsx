import type {
  ProviderConfig,
  UserProviderToken,
  LlmProviderStatus,
} from "@/types/api";
import {
  canConnectProvider,
  getProviderConnectHint,
  getProviderConnectLabel,
  needsUserCredentials,
} from "@/lib/constants";
import { ProviderStatusBadge } from "./provider-status-badge";
import { LlmReadyBadge } from "./llm-ready-badge";
import { getProviderBrand, hasKnownBrand } from "@/lib/provider-branding";
import { formatDate } from "@/lib/utils";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Plug,
  Unlink,
  RefreshCw,
  KeyRound,
  ExternalLink,
  Settings2,
} from "lucide-react";

interface ProviderCardProps {
  readonly provider: ProviderConfig;
  readonly token: UserProviderToken | undefined;
  readonly llmStatus: LlmProviderStatus | undefined;
  readonly gatewayUrl: string;
  readonly hasUserCredentials: boolean;
  readonly onConnect: (provider: ProviderConfig) => void;
  readonly onDisconnect: (providerId: string) => void;
  readonly onRefresh: (providerId: string) => void;
  readonly onSetupCredentials: (provider: ProviderConfig) => void;
  readonly isConnecting: boolean;
  readonly isDisconnecting: boolean;
  readonly isRefreshing: boolean;
}

export function ProviderCard({
  provider,
  token,
  llmStatus,
  gatewayUrl,
  hasUserCredentials,
  onConnect,
  onDisconnect,
  onRefresh,
  onSetupCredentials,
  isConnecting,
  isDisconnecting,
  isRefreshing,
}: ProviderCardProps) {
  const isConnected = token !== undefined;
  const isExpired = token?.status === "expired";
  const needsAttention =
    token?.status === "expired" || token?.status === "refresh_failed";
  const brand = getProviderBrand(provider.slug);
  const hasBrand = hasKnownBrand(provider.slug);
  const canConnect = canConnectProvider(provider, hasUserCredentials);
  const connectHint = getProviderConnectHint(provider, hasUserCredentials);
  const connectLabel = getProviderConnectLabel(provider, hasUserCredentials);
  const showCredentialsSetup = needsUserCredentials(provider);

  return (
    <Card
      className={
        isConnected && !needsAttention
          ? "border-primary/30 bg-primary/5"
          : needsAttention
            ? "border-warning/30 bg-warning/5"
            : "transition-colors hover:border-border/80"
      }
    >
      <CardHeader className="pb-3">
        <div className="flex min-w-0 items-start gap-3">
          <div
            className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-lg ${
              hasBrand
                ? brand.bgClass
                : isConnected && !needsAttention
                  ? "bg-primary/20"
                  : needsAttention
                    ? "bg-warning/20"
                    : "bg-muted"
            }`}
          >
            {provider.icon_url ? (
              <img
                src={provider.icon_url}
                alt={provider.name}
                className="h-6 w-6 rounded"
              />
            ) : hasBrand ? (
              <span className={`text-sm font-bold ${brand.textClass}`}>
                {brand.initial}
              </span>
            ) : (
              <KeyRound
                className={`h-5 w-5 ${
                  isConnected && !needsAttention
                    ? "text-primary"
                    : needsAttention
                      ? "text-warning"
                      : "text-muted-foreground"
                }`}
              />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <CardTitle className="truncate text-base">{provider.name}</CardTitle>
            {provider.description && (
              <CardDescription className="truncate text-xs">
                {provider.description}
              </CardDescription>
            )}
            <div className="mt-2 flex flex-wrap items-center gap-1">
              {isConnected ? (
                <ProviderStatusBadge status={token.status} />
              ) : (
                <Badge variant="secondary">Not Connected</Badge>
              )}
              {llmStatus?.status === "ready" && (
                <LlmReadyBadge llmStatus={llmStatus} gatewayUrl={gatewayUrl} />
              )}
              <Badge variant="outline" className="text-[10px]">
                {provider.provider_type === "api_key"
                  ? "API Key"
                  : provider.provider_type === "device_code"
                    ? "Device Code"
                    : "OAuth"}
              </Badge>
            </div>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        {isConnected && token ? (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-0.5">
              <span className="text-xs text-muted-foreground">
                Connected {formatDate(token.connected_at)}
              </span>
              {token.label && (
                <span className="text-xs text-muted-foreground/70">
                  {token.label}
                </span>
              )}
              {token.expires_at && (
                <span className="text-xs text-muted-foreground/70">
                  Expires {formatDate(token.expires_at)}
                </span>
              )}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {isExpired &&
                (provider.provider_type === "oauth2" ||
                  provider.provider_type === "device_code") && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => onRefresh(provider.id)}
                  disabled={isRefreshing}
                  isLoading={isRefreshing}
                >
                  <RefreshCw className="mr-1.5 h-3 w-3" />
                  Refresh
                </Button>
              )}
              <Button
                variant="outline"
                size="sm"
                onClick={() => onDisconnect(provider.id)}
                disabled={isDisconnecting}
                isLoading={isDisconnecting}
              >
                <Unlink className="mr-1.5 h-3 w-3" />
                Disconnect
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1">
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">
                  Not connected
                </span>
                {provider.documentation_url && (
                  <a
                    href={provider.documentation_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-0.5 text-xs text-primary hover:underline"
                  >
                    Docs
                    <ExternalLink className="h-2.5 w-2.5" />
                  </a>
                )}
              </div>
              {connectHint && (
                <span className="text-xs text-muted-foreground/70">
                  {connectHint}
                </span>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {showCredentialsSetup && !hasUserCredentials && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => onSetupCredentials(provider)}
                >
                  <Settings2 className="mr-1.5 h-3 w-3" />
                  Setup OAuth App
                </Button>
              )}
              {showCredentialsSetup && hasUserCredentials && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onSetupCredentials(provider)}
                >
                  <Settings2 className="mr-1.5 h-3 w-3" />
                  Manage App
                </Button>
              )}
              <Button
                size="sm"
                onClick={() => onConnect(provider)}
                disabled={isConnecting || !canConnect}
                isLoading={isConnecting}
              >
                <Plug className="mr-1.5 h-3 w-3" />
                {connectLabel}
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
