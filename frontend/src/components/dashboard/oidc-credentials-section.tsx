import { useState, useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useOidcCredentials,
  useRegenerateOidcSecret,
} from "@/hooks/use-services";
import { copyToClipboard } from "@/lib/utils";
import { DetailRow } from "@/components/shared/detail-row";
import { DiscoveryEndpoints } from "./discovery-endpoints";
import { RedirectUriEditor } from "./redirect-uri-editor";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Copy,
  Eye,
  EyeOff,
  RefreshCw,
  AlertTriangle,
} from "lucide-react";
import { toast } from "sonner";

interface OidcCredentialsSectionProps {
  readonly serviceId: string;
  readonly oauthClientId: string | null;
}

export function OidcCredentialsSection({
  serviceId,
  oauthClientId,
}: OidcCredentialsSectionProps) {
  const queryClient = useQueryClient();
  const [showCredentials, setShowCredentials] = useState(false);
  const [secretVisible, setSecretVisible] = useState(false);
  const [confirmRegenerate, setConfirmRegenerate] = useState(false);

  const regenerateMutation = useRegenerateOidcSecret();

  const {
    data: credentials,
    isLoading: credentialsLoading,
  } = useOidcCredentials(serviceId, showCredentials);

  // SEC-9: Clean up cached credentials on unmount to prevent
  // decrypted secrets from lingering in memory
  useEffect(() => {
    return () => {
      queryClient.removeQueries({
        queryKey: ["services", serviceId, "oidc-credentials"],
      });
    };
  }, [queryClient, serviceId]);

  async function handleCopy(text: string, label: string) {
    try {
      await copyToClipboard(text);
      toast.success(`${label} copied to clipboard`);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  async function handleRegenerate() {
    try {
      const result = await regenerateMutation.mutateAsync(serviceId);
      setConfirmRegenerate(false);
      setShowCredentials(false);
      setSecretVisible(false);
      toast.success(result.message);
    } catch {
      toast.error("Failed to regenerate secret");
    }
  }

  return (
    <div className="space-y-3">
      {oauthClientId && (
        <DetailRow
          label="Client ID"
          value={oauthClientId}
          copyable
          mono
        />
      )}

      {!showCredentials ? (
        <div className="pt-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowCredentials(true)}
          >
            <Eye className="mr-1 h-3 w-3" />
            Reveal credentials
          </Button>
          <p className="mt-1 text-xs text-muted-foreground">
            Credentials should be stored securely and never shared
            publicly.
          </p>
        </div>
      ) : credentialsLoading ? (
        <p className="text-sm text-muted-foreground">
          Loading credentials...
        </p>
      ) : credentials ? (
        <div className="space-y-3">
          <div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-amber-400">
              <AlertTriangle className="h-4 w-4" />
              Store this secret securely
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              The client secret provides full access to this OIDC
              client. Never expose it in client-side code or version
              control.
            </p>
          </div>

          <div>
            <p className="mb-1 text-xs font-medium text-muted-foreground">
              Client Secret
            </p>
            <div className="flex items-center gap-2">
              <code className="flex-1 truncate rounded bg-muted px-2 py-1 text-xs">
                {secretVisible
                  ? credentials.client_secret
                  : "***".padEnd(32, "*")}
              </code>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 shrink-0"
                onClick={() => setSecretVisible(!secretVisible)}
              >
                {secretVisible ? (
                  <EyeOff className="h-3 w-3" />
                ) : (
                  <Eye className="h-3 w-3" />
                )}
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 shrink-0"
                onClick={() =>
                  void handleCopy(
                    credentials.client_secret,
                    "Client secret",
                  )
                }
              >
                <Copy className="h-3 w-3" />
              </Button>
            </div>
          </div>

          <Separator />

          <div>
            <p className="mb-2 text-xs font-medium text-muted-foreground">
              Redirect URIs
            </p>
            <RedirectUriEditor
              serviceId={serviceId}
              initialUris={credentials.redirect_uris}
            />
          </div>

          <Separator />

          <DiscoveryEndpoints
            issuer={credentials.issuer}
            authorizationEndpoint={credentials.authorization_endpoint}
            tokenEndpoint={credentials.token_endpoint}
            userinfoEndpoint={credentials.userinfo_endpoint}
            jwksUri={credentials.jwks_uri}
          />

          <Separator />

          <div>
            {!confirmRegenerate ? (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setConfirmRegenerate(true)}
              >
                <RefreshCw className="mr-1 h-3 w-3" />
                Regenerate secret
              </Button>
            ) : (
              <div className="space-y-2 rounded-md border border-destructive/30 bg-destructive/5 p-3">
                <p className="text-sm font-medium text-destructive">
                  This will invalidate the current secret
                  immediately.
                </p>
                <p className="text-xs text-muted-foreground">
                  All existing integrations using the current
                  secret will stop working until updated with the
                  new secret.
                </p>
                <div className="flex gap-2">
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => void handleRegenerate()}
                    isLoading={regenerateMutation.isPending}
                  >
                    Confirm regeneration
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setConfirmRegenerate(false)}
                  >
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
