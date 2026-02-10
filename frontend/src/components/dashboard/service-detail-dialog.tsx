import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { DownstreamService } from "@/types/api";
import {
  useOidcCredentials,
  useRegenerateOidcSecret,
} from "@/hooks/use-services";
import { getAuthTypeLabel, isOidcService } from "@/lib/constants";
import { formatDate, copyToClipboard } from "@/lib/utils";
import { ServiceEditDialog } from "./service-edit-dialog";
import { RedirectUriEditor } from "./redirect-uri-editor";
import { EndpointList } from "./endpoint-list";
import { McpConnectionInfo } from "./mcp-connection-info";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Copy,
  Eye,
  EyeOff,
  Pencil,
  RefreshCw,
  AlertTriangle,
} from "lucide-react";
import { toast } from "sonner";

interface ServiceDetailDialogProps {
  readonly service: DownstreamService | null;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

export function ServiceDetailDialog({
  service,
  open,
  onOpenChange,
}: ServiceDetailDialogProps) {
  const queryClient = useQueryClient();
  const [editOpen, setEditOpen] = useState(false);
  const [showCredentials, setShowCredentials] = useState(false);
  const [confirmRegenerate, setConfirmRegenerate] = useState(false);
  const [secretVisible, setSecretVisible] = useState(false);

  const regenerateMutation = useRegenerateOidcSecret();

  // CR-8: Compute oidc status safely (no unsafe cast to empty object)
  const oidc = service ? isOidcService(service) : false;
  const serviceId = service?.id ?? "";

  const {
    data: credentials,
    isLoading: credentialsLoading,
  } = useOidcCredentials(serviceId, showCredentials && oidc);

  if (!service) return null;

  async function handleCopy(text: string, label: string) {
    try {
      await copyToClipboard(text);
      toast.success(`${label} copied to clipboard`);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  async function handleRegenerate() {
    if (!service) return;
    try {
      const result = await regenerateMutation.mutateAsync(service.id);
      setConfirmRegenerate(false);
      setShowCredentials(false);
      setSecretVisible(false);
      toast.success(result.message);
    } catch {
      toast.error("Failed to regenerate secret");
    }
  }

  function handleClose(nextOpen: boolean) {
    if (!nextOpen) {
      setShowCredentials(false);
      setSecretVisible(false);
      setConfirmRegenerate(false);
      // SEC-9: Remove cached credentials from React Query to prevent
      // decrypted secrets from lingering in memory after dialog close
      if (service) {
        queryClient.removeQueries({
          queryKey: ["services", service.id, "oidc-credentials"],
        });
      }
    }
    onOpenChange(nextOpen);
  }

  return (
    <>
      <Dialog open={open} onOpenChange={handleClose}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <div className="flex items-center justify-between pr-8">
              <DialogTitle>{service.name}</DialogTitle>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setEditOpen(true)}
              >
                <Pencil className="mr-1 h-3 w-3" />
                Edit
              </Button>
            </div>
            <DialogDescription>
              {service.description ?? "No description provided."}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <DetailSection title="General">
              <DetailRow label="Slug" value={service.slug} />
              <DetailRow label="Base URL" value={service.base_url} copyable />
              <DetailRow
                label="Auth Type"
                value={getAuthTypeLabel(service)}
                badge
              />
              <DetailRow
                label="Status"
                value={service.is_active ? "Active" : "Inactive"}
                badge
                badgeVariant={service.is_active ? "success" : "secondary"}
              />
              <DetailRow
                label="Created"
                value={formatDate(service.created_at)}
              />
              <DetailRow
                label="Updated"
                value={formatDate(service.updated_at)}
              />
            </DetailSection>

            {oidc && (
              <>
                <Separator />
                <DetailSection title="OIDC Configuration">
                  {service.oauth_client_id && (
                    <DetailRow
                      label="Client ID"
                      value={service.oauth_client_id}
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
                          serviceId={service.id}
                          initialUris={credentials.redirect_uris}
                        />
                      </div>

                      <Separator />

                      <DiscoveryEndpoints
                        issuer={credentials.issuer}
                        authorizationEndpoint={
                          credentials.authorization_endpoint
                        }
                        tokenEndpoint={credentials.token_endpoint}
                        userinfoEndpoint={credentials.userinfo_endpoint}
                        jwksUri={credentials.jwks_uri}
                        onCopy={handleCopy}
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
                </DetailSection>
              </>
            )}

            <Separator />
            <DetailSection title="API Endpoints">
              <EndpointList
                serviceId={service.id}
                hasApiSpecUrl={service.api_spec_url !== null && service.api_spec_url !== undefined}
              />
            </DetailSection>

            <Separator />
            <DetailSection title="MCP Connection">
              <McpConnectionInfo
                serviceSlug={service.slug}
                serviceName={service.name}
              />
            </DetailSection>
          </div>
        </DialogContent>
      </Dialog>

      <ServiceEditDialog
        service={service}
        open={editOpen}
        onOpenChange={setEditOpen}
      />
    </>
  );
}

function DetailSection({
  title,
  children,
}: {
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  return (
    <div>
      <h3 className="mb-3 text-sm font-semibold">{title}</h3>
      <div className="space-y-2">{children}</div>
    </div>
  );
}

function DetailRow({
  label,
  value,
  copyable = false,
  mono = false,
  badge = false,
  badgeVariant = "secondary",
}: {
  readonly label: string;
  readonly value: string;
  readonly copyable?: boolean;
  readonly mono?: boolean;
  readonly badge?: boolean;
  readonly badgeVariant?: "default" | "secondary" | "destructive" | "outline" | "success" | "warning";
}) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-muted-foreground">{label}</span>
      <div className="flex items-center gap-1">
        {badge ? (
          <Badge variant={badgeVariant}>{value}</Badge>
        ) : (
          <span className={mono ? "font-mono text-xs" : ""}>{value}</span>
        )}
        {copyable && (
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={() =>
              void copyToClipboard(value).then(
                () => toast.success(`${label} copied`),
                () => toast.error("Failed to copy"),
              )
            }
          >
            <Copy className="h-3 w-3" />
          </Button>
        )}
      </div>
    </div>
  );
}

function DiscoveryEndpoints({
  issuer,
  authorizationEndpoint,
  tokenEndpoint,
  userinfoEndpoint,
  jwksUri,
  onCopy,
}: {
  readonly issuer: string;
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly userinfoEndpoint: string;
  readonly jwksUri: string;
  readonly onCopy: (text: string, label: string) => Promise<void>;
}) {
  const endpoints = [
    { label: "Issuer", value: issuer },
    { label: "Authorization", value: authorizationEndpoint },
    { label: "Token", value: tokenEndpoint },
    { label: "UserInfo", value: userinfoEndpoint },
    { label: "JWKS", value: jwksUri },
  ] as const;

  return (
    <div>
      <p className="mb-2 text-xs font-medium text-muted-foreground">
        Discovery Endpoints
      </p>
      <div className="space-y-1">
        {endpoints.map((ep) => (
          <div
            key={ep.label}
            className="flex items-center justify-between text-xs"
          >
            <span className="text-muted-foreground">{ep.label}</span>
            <div className="flex items-center gap-1">
              <code className="max-w-[300px] truncate rounded bg-muted px-1.5 py-0.5">
                {ep.value}
              </code>
              <Button
                variant="ghost"
                size="icon"
                className="h-5 w-5"
                onClick={() => void onCopy(ep.value, ep.label)}
              >
                <Copy className="h-2.5 w-2.5" />
              </Button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
