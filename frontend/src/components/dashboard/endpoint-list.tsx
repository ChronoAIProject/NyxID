import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useAuthStore } from "@/stores/auth-store";
import { PublicationOperations } from "@/components/services/tool-publication";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import type { CreateEndpointPayload } from "@/lib/endpoint-changes";
import { useState } from "react";
import type { ServiceEndpoint } from "@/types/api";
import type { CreateEndpointFormData } from "@/schemas/endpoints";
import {
  useEndpoints,
  useCreateEndpoint,
  useUpdateEndpoint,
  useDeleteEndpoint,
  useDiscoverEndpoints,
} from "@/hooks/use-endpoints";
import { EndpointFormDialog } from "./endpoint-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { PowerBoltIcon } from "@/components/icons/empty-state";
import { Pencil, Trash2, Wand2 } from "lucide-react";
import { toast } from "sonner";

interface EndpointListProps {
  readonly serviceId: string;
  readonly service?: import("@/types/api").DownstreamService;
  readonly canWrite?: boolean;
  readonly hasApiSpecUrl: boolean;
}

const METHOD_COLORS: Readonly<Record<string, string>> = {
  GET: "bg-info/15 text-info border-info/30",
  POST: "bg-success/15 text-success border-success/30",
  PUT: "bg-warning/15 text-warning border-warning/30",
  PATCH: "bg-orange-500/15 text-orange-400 border-orange-500/30",
  DELETE: "bg-destructive/15 text-destructive border-destructive/30",
};

function getMethodColor(method: string): string {
  return (
    METHOD_COLORS[method.toUpperCase()] ?? "bg-muted text-muted-foreground"
  );
}

export function EndpointList(props: EndpointListProps) {
  useUiTranslation();
  return <EndpointListEditor key={props.serviceId} {...props} />;
}

function EndpointListEditor({
  serviceId,
  hasApiSpecUrl,
  service,
  canWrite = true,
}: EndpointListProps) {
  useUiTranslation();
  const admin = useAuthStore((state) => state.user?.is_admin) ?? false;
  const { data: endpoints, isLoading } = useEndpoints(serviceId);
  const createMutation = useCreateEndpoint();
  const updateMutation = useUpdateEndpoint();
  const deleteMutation = useDeleteEndpoint();
  const discoverMutation = useDiscoverEndpoints();

  const [formOpen, setFormOpen] = useState(false);
  const [editingEndpoint, setEditingEndpoint] =
    useState<ServiceEndpoint | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  function handleAdd() {
    setEditingEndpoint(null);
    setFormOpen(true);
  }

  function handleEdit(endpoint: ServiceEndpoint) {
    setEditingEndpoint(endpoint);
    setFormOpen(true);
  }

  async function handleDelete({
    serviceId: targetServiceId,
    endpointId,
  }: {
    serviceId: string;
    endpointId: string;
  }) {
    setDeletingId(endpointId);
    try {
      await deleteMutation.mutateAsync({
        serviceId: targetServiceId,
        endpointId,
      });
      toast.success(translateUi("Endpoint deleted"));
    } catch {
      toast.error(translateErrorMessage("Failed to delete endpoint"));
      throw new Error("Failed to delete endpoint");
    } finally {
      setDeletingId(null);
    }
  }

  async function handleFormSubmit(
    data: CreateEndpointFormData,
    patch?: Partial<CreateEndpointPayload>,
  ) {
    if (editingEndpoint) {
      await updateMutation.mutateAsync({
        serviceId,
        endpointId: editingEndpoint.id,
        before: editingEndpoint,
        patch,
        data,
      });
      toast.success(translateUi("Endpoint updated"));
    } else {
      await createMutation.mutateAsync({ serviceId, data });
      toast.success(translateUi("Endpoint created"));
    }
  }

  async function handleDiscover() {
    try {
      const result = await discoverMutation.mutateAsync(serviceId);
      toast.success(translateUi(result.message));
    } catch {
      toast.error(translateErrorMessage("Failed to discover endpoints"));
    }
  }

  const deletion = useChangeReview(handleDelete, false, serviceId);

  if (isLoading && !endpoints) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {deletion.dialog}
      <div className="flex items-center gap-2">
        {canWrite && <AddCtaButton label={translateUi("Add Endpoint")} onClick={handleAdd} />}
        {canWrite && hasApiSpecUrl && (
          <Button
            variant="outline"
            onClick={() => void handleDiscover()}
            isLoading={discoverMutation.isPending}
          >
            <ButtonIcon>
              <Wand2 className="h-3 w-3" />
            </ButtonIcon>{translateUi("Auto-discover from OpenAPI")}</Button>
        )}
      </div>

      {!endpoints || endpoints.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-8">
          <PowerBoltIcon className="h-48 w-48 text-muted-foreground/30" />
          <div className="rounded-lg border border-dashed border-border px-4 py-3 text-14 text-muted-foreground">{translateUi("No endpoints configured.")}{" "}
            {hasApiSpecUrl
              ? translateUi("Use auto-discover or add one manually.")
              : translateUi("Add one manually or set an OpenAPI spec URL to auto-discover.")}
          </div>
        </div>
      ) : (
        <div className="rounded-lg border">
          <table className="w-full text-14">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">{translateUi("Name")}</th>
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">{translateUi("Method")}</th>
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">{translateUi("Path")}</th>
                <th className="w-[80px] px-3 py-2 text-right font-medium text-muted-foreground">{translateUi("Actions")}</th>
              </tr>
            </thead>
            <tbody>
              {endpoints.map((ep) => (
                <tr key={ep.id} className="border-b last:border-0">
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-2">
                      <span className="text-13">{ep.name}</span>
                      <Badge
                        variant={
                          (ep.publication ?? "published") === "published"
                            ? "success"
                            : "secondary"
                        }
                      >
                        {ep.publication ?? translateUi("published")}
                      </Badge>
                      {!ep.is_active && (
                        <Badge variant="secondary" className="text-12">{translateUi("Inactive")}</Badge>
                      )}
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <Badge
                      variant="secondary"
                      className={`text-12 ${getMethodColor(ep.method)}`}
                    >
                      {ep.method}
                    </Badge>
                  </td>
                  <td className="px-3 py-2">
                    <code className="text-14 text-muted-foreground">
                      {ep.path}
                    </code>
                  </td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        disabled={!canWrite}
                        onClick={() => handleEdit(ep)}
                      >
                        <Pencil className="h-3 w-3" />
                        <span className="sr-only">{translateUi("Edit endpoint")}</span>
                      </Button>
                      {canWrite &&
                        (service?.offering_kind !== "tool" || admin) && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground hover:text-destructive"
                        onClick={() =>
                          deletion.review({ serviceId, endpointId: ep.id }, [
                            {
                              field: "Delete endpoint",
                              before: `${ep.method} ${ep.path} (${ep.name})`,
                              after: "Deleted",
                            },
                          ])
                        }
                        disabled={deletingId === ep.id}
                      >
                        <Trash2 className="h-3 w-3 text-destructive" />
                        <span className="sr-only">{translateUi("Delete endpoint")}</span>
                      </Button>
                        )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {service?.offering_kind === "tool" && (
        <PublicationOperations tool={service} disabled={!canWrite} />
      )}
      <EndpointFormDialog
        serviceId={serviceId}
        currentEndpoint={
          endpoints
            ? (endpoints.find((item) => item.id === editingEndpoint?.id) ??
              null)
            : undefined
        }
        open={formOpen}
        onOpenChange={setFormOpen}
        endpoint={editingEndpoint}
        onSubmit={handleFormSubmit}
        isPending={
          editingEndpoint ? updateMutation.isPending : createMutation.isPending
        }
      />
    </div>
  );
}
