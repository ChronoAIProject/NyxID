import { useState } from "react";
import { useServices, useDeleteService } from "@/hooks/use-services";
import { CreateServiceDialog } from "@/components/services/create-service-dialog";
import { ServiceCard } from "@/components/dashboard/service-card";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { BrickWallIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

export function ServiceListPage() {
  const { data: services, isLoading } = useServices();
  const deleteMutation = useDeleteService();
  const [createOpen, setCreateOpen] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  async function handleDelete(id: string) {
    setDeletingId(id);
    try {
      await deleteMutation.mutateAsync(id);
      toast.success("Service deleted successfully");
    } catch {
      toast.error("Failed to delete service");
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Services"
        description="Manage downstream services and their authentication."
        actions={
          <AddCtaButton label="Create Service" onClick={() => setCreateOpen(true)} />
        }
      />

      <CreateServiceDialog open={createOpen} onOpenChange={setCreateOpen} />

      {isLoading ? (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={`svc-skel-${String(i)}`} className="h-36 w-full" />
          ))}
        </div>
      ) : !services || services.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <BrickWallIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">No services yet</p>
            <p className="text-xs text-muted-foreground">
              Add a service to get started.
            </p>
          </div>
        </div>
      ) : (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {services.map((service) => (
            <ServiceCard
              key={service.id}
              service={service}
              onDelete={handleDelete}
              isDeleting={deletingId === service.id}
            />
          ))}
        </div>
      )}
    </div>
  );
}
