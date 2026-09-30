import { canEditConnection } from "@/lib/connection-access";
import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { useKeys, useCatalog } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { GroupedServiceCards } from "./grouped-service-cards";
import type { KeyInfo } from "@/types/keys";
import {
  classifyConnection,
  type RoutingCandidate,
} from "@/lib/service-routing-preview";

export interface RoutingPreviewProps {
  readonly renderConnection: (candidate: RoutingCandidate) => ReactNode;
}

export function ConnectionCard({
  candidate,
  renderConnection,
}: RoutingPreviewProps & { readonly candidate: RoutingCandidate }) {
  const key = candidate.key;
  return (
    <div className="flex min-w-0 flex-col gap-2 rounded-xl border border-border/70 bg-card p-3">
      {renderConnection(candidate)}
      <div className="flex items-center justify-between gap-3 px-1 text-xs text-muted-foreground">
        <span className="truncate">{candidate.owner}</span>
        <span
          className="shrink-0"
          title="Routing readiness; the credential status is shown on the card."
        >
          {candidate.reason}
        </span>
      </div>
      <details className="px-1 text-xs text-muted-foreground">
        <summary className="w-fit cursor-pointer py-1 hover:text-foreground">
          Details
        </summary>
        <div className="space-y-2 py-2">
          {key.description && <p>{key.description}</p>}
          {canEditConnection(key) && <p>Auth: {key.auth_method}</p>}
          {key.last_used_at && (
            <p>
              Credential last prepared:{" "}
              {new Date(key.last_used_at).toLocaleDateString()}
            </p>
          )}
          {key.expires_at && (
            <p>Expires: {new Date(key.expires_at).toLocaleDateString()}</p>
          )}
          {canEditConnection(key) && !!key.granted_scopes?.length && (
            <p className="break-words">
              Permissions: {key.granted_scopes.join(", ")}
            </p>
          )}
          <Link
            to="/keys/$keyId"
            params={{ keyId: key.id }}
            className="inline-flex items-center gap-1 text-primary hover:underline"
          >
            Open full service details
            <ChevronRight className="size-3" />
          </Link>
        </div>
      </details>
    </div>
  );
}

export default function ServiceRoutingPreview({
  renderConnectionActions,
  actions,
}: {
  readonly renderConnectionActions?: (key: KeyInfo) => ReactNode;
  /** Page-level primary action (Connect Service) shown in the sticky toolbar. */
  readonly actions?: ReactNode | ((compact: boolean) => ReactNode);
}) {
  const keys = useKeys();
  const catalog = useCatalog();
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
