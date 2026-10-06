import { useState, type ReactNode } from "react";
import { useKeys, useCatalog } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { GroupedServiceCards } from "./grouped-service-cards";
import type { KeyInfo } from "@/types/keys";
import { classifyConnection } from "@/lib/service-routing-preview";

export default function ServiceRoutingPreview({
  renderConnectionActions,
  actions,
}: {
  readonly renderConnectionActions?: (key: KeyInfo) => ReactNode;
  /** Page-level primary action (Connect Service) shown in the sticky toolbar. */
  readonly actions?: ReactNode | ((compact: boolean) => ReactNode);
}) {
  const keys = useKeys();
  const catalog = useCatalog({ includeAll: true });
  const services = useUserServices();
  const [mountedAt] = useState(Date.now);
  const candidates = (keys.data ?? []).map((key) =>
    classifyConnection(
      key,
      services.data ?? [],
      keys.dataUpdatedAt || mountedAt,
    ),
  );

  if (keys.isLoading) return <Skeleton className="h-64 w-full" />;
  if (keys.error)
    return (
      <div className="rounded-xl border p-6 text-sm">
        Connections could not be loaded.{" "}
        <Button variant="outline" onClick={() => void keys.refetch()}>
          Retry
        </Button>
      </div>
    );

  return (
    <GroupedServiceCards
      keys={candidates.map((candidate) => ({
        ...candidate.key,
        credential_source: candidate.source,
      }))}
      catalog={catalog.data}
      renderConnectionActions={renderConnectionActions}
      actions={actions}
    />
  );
}
