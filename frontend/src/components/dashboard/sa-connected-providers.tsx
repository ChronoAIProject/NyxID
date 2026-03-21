import { useState } from "react";
import {
  useSaProviders,
  useConnectApiKeyForSa,
  useDisconnectSaProvider,
  useInitiateOAuthForSa,
} from "@/hooks/use-service-accounts";
import { useProviders } from "@/hooks/use-providers";
import { formatDate } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { hardRedirect } from "@/lib/navigation";
import { ApiKeyDialog } from "@/components/dashboard/api-key-dialog";
import { SaDeviceCodeDialog } from "@/components/dashboard/sa-device-code-dialog";
import type { ProviderConfig } from "@/types/api";
import { DetailSection } from "@/components/shared/detail-section";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
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
import { Plug, Unlink, KeyRound, Globe, Smartphone, MessageCircle } from "lucide-react";
import { toast } from "sonner";

interface SaConnectedProvidersProps {
  readonly saId: string;
}

export function SaConnectedProviders({ saId }: SaConnectedProvidersProps) {
  const { data: saProviders, isLoading: providersLoading } = useSaProviders(saId);
  const { data: allProviders } = useProviders();
  const connectApiKeyMutation = useConnectApiKeyForSa();
  const disconnectMutation = useDisconnectSaProvider();
  const initiateOAuthMutation = useInitiateOAuthForSa();

  const [connectDialogProvider, setConnectDialogProvider] = useState<ProviderConfig | null>(null);
  const [deviceCodeDialogProvider, setDeviceCodeDialogProvider] = useState<ProviderConfig | null>(null);

  const connectedProviderIds = new Set(saProviders?.map((t) => t.provider_id) ?? []);
  const availableProviders = (allProviders ?? []).filter(
    (p) => p.is_active && !connectedProviderIds.has(p.id),
  );

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
      hardRedirect(response.authorization_url);
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

  return (
    <>
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
    </>
  );
}

function ProviderTypeIcon({ type }: { readonly type: string }) {
  if (type === "api_key") return <KeyRound className="mr-2 h-4 w-4" />;
  if (type === "oauth2") return <Globe className="mr-2 h-4 w-4" />;
  if (type === "telegram_widget") return <MessageCircle className="mr-2 h-4 w-4" />;
  return <Smartphone className="mr-2 h-4 w-4" />;
}

function providerTypeLabel(type: string): string {
  if (type === "api_key") return "API Key";
  if (type === "oauth2") return "OAuth";
  if (type === "telegram_widget") return "Telegram";
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
