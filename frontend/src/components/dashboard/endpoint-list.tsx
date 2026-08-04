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

export function EndpointList({ serviceId, hasApiSpecUrl }: EndpointListProps) {
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

  async function handleDelete(endpointId: string) {
    setDeletingId(endpointId);
    try {
      await deleteMutation.mutateAsync({ serviceId, endpointId });
      toast.success("Endpoint deleted");
    } catch {
      toast.error("Failed to delete endpoint");
    } finally {
      setDeletingId(null);
    }
  }

  async function handleFormSubmit(data: CreateEndpointFormData) {
    if (editingEndpoint) {
      await updateMutation.mutateAsync({
        serviceId,
        endpointId: editingEndpoint.id,
        data,
      });
      toast.success("Endpoint updated");
    } else {
      await createMutation.mutateAsync({ serviceId, data });
      toast.success("Endpoint created");
    }
  }

  async function handleDiscover() {
    try {
      const result = await discoverMutation.mutateAsync(serviceId);
      toast.success(result.message);
    } catch {
      toast.error("Failed to discover endpoints");
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <AddCtaButton label="Add Endpoint" onClick={handleAdd} />
        {hasApiSpecUrl && (
          <Button
            variant="outline"
            onClick={() => void handleDiscover()}
            isLoading={discoverMutation.isPending}
          >
            <ButtonIcon><Wand2 className="h-3 w-3" /></ButtonIcon>
            Auto-discover from OpenAPI
          </Button>
        )}
      </div>

      {!endpoints || endpoints.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-8">
          <PowerBoltIcon className="h-48 w-48 text-muted-foreground/30" />
          <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground/30">
            No endpoints configured.{" "}
            {hasApiSpecUrl
              ? "Use auto-discover or add one manually."
              : "Add one manually or set an OpenAPI spec URL to auto-discover."}
          </div>
        </div>
      ) : (
        <div className="rounded-lg border">
          <table className="w-full text-[12px]">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">
                  Name
                </th>
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">
                  Method
                </th>
                <th className="px-3 py-2 text-left font-medium text-muted-foreground">
                  Path
                </th>
                <th className="w-[80px] px-3 py-2 text-right font-medium text-muted-foreground">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {endpoints.map((ep) => (
                <tr key={ep.id} className="border-b last:border-0">
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-2">
                      <span className="text-xs">{ep.name}</span>
                      {!ep.is_active && (
                        <Badge variant="secondary" className="text-[10px]">
                          Inactive
                        </Badge>
                      )}
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <Badge
                      variant="secondary"
                      className={`text-[10px] ${getMethodColor(ep.method)}`}
                    >
                      {ep.method}
                    </Badge>
                  </td>
                  <td className="px-3 py-2">
                    <code className="text-xs text-muted-foreground">
                      {ep.path}
                    </code>
                  </td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => handleEdit(ep)}
                      >
                        <Pencil className="h-3 w-3" />
                        <span className="sr-only">Edit endpoint</span>
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground hover:text-destructive"
                        onClick={() => void handleDelete(ep.id)}
                        disabled={deletingId === ep.id}
                      >
                        <Trash2 className="h-3 w-3 text-destructive" />
                        <span className="sr-only">Delete endpoint</span>
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <EndpointFormDialog
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
