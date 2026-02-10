import { useNavigate, useParams } from "@tanstack/react-router";
import { useService, useDeleteService } from "@/hooks/use-services";
import {
  isOidcService,
  isConnectable,
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
} from "@/lib/constants";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { OidcCredentialsSection } from "@/components/dashboard/oidc-credentials-section";
import { EndpointList } from "@/components/dashboard/endpoint-list";
import { McpConnectionInfo } from "@/components/dashboard/mcp-connection-info";
import { ServiceRequirementsView } from "@/components/dashboard/service-requirements-editor";
import { useMyProviderTokens } from "@/hooks/use-providers";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Pencil, Trash2, AlertCircle } from "lucide-react";
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
        <h3 className="mb-2 text-lg font-semibold">Service not found</h3>
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

  return (
    <div className="space-y-6">
      <PageHeader
        breadcrumbs={[
          { label: "Services", to: "/services" },
          { label: service.name },
        ]}
        title={service.name}
        description={service.description ?? undefined}
        actions={
          <>
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
        <DetailRow label="Base URL" value={service.base_url} copyable />
        <DetailRow
          label="Auth Type"
          value={getAuthTypeLabel(service)}
          badge
        />
        <DetailRow
          label="Category"
          value={SERVICE_CATEGORY_LABELS[service.service_category] ?? service.service_category}
          badge
        />
        <DetailRow
          label="Status"
          value={service.is_active ? "Active" : "Inactive"}
          badge
          badgeVariant={service.is_active ? "success" : "secondary"}
        />
        <DetailRow label="Created" value={formatDate(service.created_at)} />
        <DetailRow label="Updated" value={formatDate(service.updated_at)} />
      </DetailSection>

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
                service.api_spec_url !== null &&
                service.api_spec_url !== undefined
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
    </div>
  );
}
