import { useNavigate, useParams } from "@tanstack/react-router";
import { useService, useDeleteService } from "@/hooks/use-services";
import {
  isOidcService,
  isConnectable,
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
} from "@/lib/constants";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { CopyableField } from "@/components/shared/copyable-field";
import { OidcCredentialsSection } from "@/components/dashboard/oidc-credentials-section";
import { EndpointList } from "@/components/dashboard/endpoint-list";
import { McpConnectionInfo } from "@/components/dashboard/mcp-connection-info";
import { SshServiceInstructions } from "@/components/dashboard/ssh-service-instructions";
import { ServiceRequirementsView } from "@/components/dashboard/service-requirements-editor";
import { useMyProviderTokens } from "@/hooks/use-providers";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Pencil, Trash2, AlertCircle, Terminal } from "lucide-react";
import { toast } from "sonner";

const PROPAGATION_MODE_LABELS: Readonly<Record<string, string>> = {
  none: "None",
  headers: "Headers (X-NyxID-*)",
  jwt: "Signed JWT",
  both: "Headers + JWT",
};

export function ServiceDetailPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const navigate = useNavigate();
  const { data: service, isLoading, error } = useService(serviceId);
  const deleteMutation = useDeleteService();
  const { data: tokens } = useMyProviderTokens();

  async function handleDelete() {
    if (!service) return;
    try {
      await deleteMutation.mutateAsync(service.id);
      toast.success("Service deleted successfully");
      void navigate({ to: "/services" });
    } catch {
      toast.error("Failed to delete service");
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

  if (error || !service) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">Service not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The service you are looking for does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/services" })}
        >
          Back to Services
        </Button>
      </div>
    );
  }

  const oidc = isOidcService(service);
  const isSshService = service.service_type === "ssh";

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Services", to: "/services" },
          { label: service.name },
        ]}
        title={service.name}
        description={service.description ?? undefined}
        actions={
          <>
            {isSshService && service.ssh_config?.certificate_auth_enabled && (
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  void navigate({
                    to: "/ssh/$serviceId/terminal",
                    params: { serviceId },
                    search: { principal: service.ssh_config?.allowed_principals[0] },
                  })
                }
              >
                <Terminal className="mr-1 h-3 w-3" />
                Terminal
              </Button>
            )}
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void navigate({
                  to: "/services/$serviceId/edit",
                  params: { serviceId },
                })
              }
            >
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              <Trash2 className="mr-1 h-3 w-3" />
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="General">
        <DetailRow label="Slug" value={service.slug} />
        <DetailRow
          label="Service Type"
          value={SERVICE_TYPE_LABELS[service.service_type] ?? service.service_type}
          badge
        />
        <DetailRow
          label="Category"
          value={SERVICE_CATEGORY_LABELS[service.service_category] ?? service.service_category}
          badge
        />
        <DetailRow
          label="Visibility"
          value={service.visibility === "private" ? "Private (only visible to you)" : "Public"}
          badge
          badgeVariant={service.visibility === "private" ? "outline" : "secondary"}
        />
        {!isSshService && (
          <>
            <DetailRow label="Base URL" value={service.base_url} copyable />
            <DetailRow
              label="Auth Type"
              value={getAuthTypeLabel(service)}
              badge
            />
          </>
        )}
        <DetailRow
          label="Status"
          value={service.is_active ? "Active" : "Inactive"}
          badge
          badgeVariant={service.is_active ? "success" : "secondary"}
        />
        <DetailRow label="Created" value={formatDate(service.created_at)} />
        <DetailRow label="Updated" value={formatDate(service.updated_at)} />
      </DetailSection>

      {isSshService ? (
        <>
          <Separator />
          <DetailSection title="SSH Configuration">
            {service.ssh_config ? (
              <>
                <DetailRow
                  label="Target"
                  value={`${service.ssh_config.host}:${String(service.ssh_config.port)}`}
                  copyable
                  mono
                />
                <DetailRow
                  label="Certificate Auth"
                  value={
                    service.ssh_config.certificate_auth_enabled
                      ? "Enabled"
                      : "Transport only"
                  }
                  badge
                  badgeVariant={
                    service.ssh_config.certificate_auth_enabled
                      ? "success"
                      : "secondary"
                  }
                />
                {service.ssh_config.certificate_auth_enabled && (
                  <>
                    <DetailRow
                      label="Certificate TTL"
                      value={`${String(service.ssh_config.certificate_ttl_minutes)} minutes`}
                    />
                    <DetailRow
                      label="Allowed Principals"
                      value={service.ssh_config.allowed_principals.join(", ")}
                      copyable
                    />
                  </>
                )}
                {service.ssh_config.ca_public_key && (
                  <CopyableField
                    label="SSH CA Public Key"
                    value={service.ssh_config.ca_public_key}
                    size="sm"
                  />
                )}
              </>
            ) : (
              <p className="text-sm text-muted-foreground">
                SSH configuration is missing for this service.
              </p>
            )}
          </DetailSection>

          {service.ssh_config && (
            <>
              <Separator />
              <DetailSection title="Connection Instructions">
                <SshServiceInstructions
                  serviceId={service.id}
                  serviceSlug={service.slug}
                  sshConfig={service.ssh_config}
                />
              </DetailSection>
            </>
          )}
        </>
      ) : (
        <>
          {oidc && (
            <>
              <Separator />
              <DetailSection title="OIDC Configuration">
                <OidcCredentialsSection
                  serviceId={service.id}
                  oauthClientId={service.oauth_client_id}
                />
              </DetailSection>
            </>
          )}

          {isConnectable(service) && !oidc && (
            <>
              <Separator />
              <DetailSection title="API Endpoints">
                <EndpointList
                  serviceId={service.id}
                  hasApiSpecUrl={
                    (service.openapi_spec_url ?? service.api_spec_url) !== null &&
                    (service.openapi_spec_url ?? service.api_spec_url) !== undefined
                  }
                />
              </DetailSection>

              <Separator />
              <DetailSection title="MCP Connection">
                <McpConnectionInfo />
              </DetailSection>
            </>
          )}

          {service.identity_propagation_mode &&
            service.identity_propagation_mode !== "none" && (
              <>
                <Separator />
                <DetailSection title="Identity Propagation">
                  <DetailRow
                    label="Mode"
                    value={
                      PROPAGATION_MODE_LABELS[service.identity_propagation_mode] ??
                      service.identity_propagation_mode
                    }
                    badge
                  />
                  {service.identity_include_user_id && (
                    <DetailRow label="User ID" value="Included" badge badgeVariant="success" />
                  )}
                  {service.identity_include_email && (
                    <DetailRow label="Email" value="Included" badge badgeVariant="success" />
                  )}
                  {service.identity_include_name && (
                    <DetailRow label="Display Name" value="Included" badge badgeVariant="success" />
                  )}
                  {service.identity_jwt_audience && (
                    <DetailRow label="JWT Audience" value={service.identity_jwt_audience} />
                  )}
                </DetailSection>
              </>
            )}

          {service.inject_delegation_token && (
            <>
              <Separator />
              <DetailSection title="Delegation Token Injection">
                <DetailRow
                  label="Status"
                  value="Enabled"
                  badge
                  badgeVariant="success"
                />
                <DetailRow
                  label="Token Scope"
                  value={service.delegation_token_scope || "llm:proxy"}
                  mono
                />
              </DetailSection>
            </>
          )}

          <Separator />
          <DetailSection title="Provider Requirements">
            <ServiceRequirementsView
              serviceId={service.id}
              userTokenProviderIds={
                tokens
                  ? new Set(tokens.map((t) => t.provider_id))
                  : undefined
              }
            />
          </DetailSection>
        </>
      )}
    </div>
  );
}
