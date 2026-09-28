import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { useKeys, useCatalog } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { PageHeader } from "@/components/shared/page-header";
import { ServiceIcon } from "@/components/service-icon";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ServiceConnectionTable } from "@/components/dashboard/service-connection-table";
import { ServiceHistory } from "@/components/dashboard/service-history";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { groupServiceConnections } from "@/lib/service-groups";
import { connectionSourceLabel } from "@/lib/service-view";

export function ServiceOverviewPage() {
  const { groupId } = useParams({ strict: false }) as { groupId: string };
  const keys = useKeys();
  const catalog = useCatalog();
  const services = useUserServices();
  const [historyId, setHistoryId] = useState<string | null>(null);
  const [tab, setTab] = useState("connections");
  const connections = (keys.data ?? []).map((key) => ({
    ...key,
    credential_source:
      key.credential_source ??
      services.data?.find((service) => service.id === key.id)
        ?.credential_source,
  }));
  const group = groupServiceConnections(connections, catalog.data).find(
    (group) => group.id === groupId,
  );
  useBreadcrumbLabel(group?.name);
  const entry = catalog.data?.find((entry) => entry.slug === group?.slug);
  const historyConnection =
    group?.connections.find((connection) => connection.id === historyId) ??
    group?.connections[0];
  const back = (
    <Link
      to="/keys"
      search={
        import.meta.env.DEV && import.meta.env.VITE_ROUTING_PREVIEW === "1"
          ? { view: "routing" }
          : {}
      }
      className="inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
    >
      <ArrowLeft className="size-3" />
      All services
    </Link>
  );

  if (keys.isLoading) return <Skeleton className="h-96 w-full" />;
  if (keys.error)
    return (
      <div className="space-y-4">
        {back}
        <ErrorBanner
          message="Service information could not be loaded."
          onRetry={keys.refetch}
        />
      </div>
    );
  if (!group)
    return (
      <div className="space-y-4">
        {back}
        <PageHeader
          title="Service unavailable"
          description="This service is no longer in your accessible connections."
        />
      </div>
    );

  return (
    <div className="space-y-5">
      {back}
      <PageHeader
        title={group.name}
        leading={
          <div
            aria-hidden="true"
            className="flex size-12 items-center justify-center rounded-xl border border-border bg-card"
          >
            <ServiceIcon
              slug={group.iconSlug}
              iconUrl={group.iconUrl}
              size="lg"
            />
          </div>
        }
        description={
          group.description ??
          "Connections, configuration and history for this service."
        }
      />
      <div className="flex flex-wrap gap-4 text-xs text-muted-foreground">
        <span>
          {group.connections.length}{" "}
          {group.connections.length === 1 ? "connection" : "connections"}
        </span>
        <span>
          {group.connections.filter((key) => key.is_active).length} enabled
        </span>
        <span>
          {[...new Set(group.connections.map(connectionSourceLabel))].join(
            " · ",
          )}
        </span>
        {entry?.documentation_url &&
          /^https?:\/\//i.test(entry.documentation_url) && (
            <a
              href={entry.documentation_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary hover:underline"
            >
              Service documentation
            </a>
          )}
      </div>
      {catalog.error && (
        <ErrorBanner
          message="Catalog metadata could not be loaded."
          onRetry={catalog.refetch}
        />
      )}
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="connections">Connections</TabsTrigger>
          <TabsTrigger value="history">History</TabsTrigger>
        </TabsList>
        <TabsContent
          value="connections"
          className="mt-4 overflow-hidden rounded-xl border border-border/50 bg-card"
        >
          <ServiceConnectionTable
            connections={group.connections}
            serviceName={group.name}
            onViewHistory={(connection) => {
              setHistoryId(connection.id);
              setTab("history");
            }}
          />
        </TabsContent>
        <TabsContent value="history" className="mt-4 space-y-4">
          <Select value={historyConnection?.id} onValueChange={setHistoryId}>
            <SelectTrigger
              aria-label="Connection history"
              className="w-full sm:w-80"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {group.connections.map((key) => (
                <SelectItem key={key.id} value={key.id}>
                  {key.label} · {connectionSourceLabel(key)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {tab === "history" && historyConnection && (
            <ServiceHistory
              key={historyConnection.id}
              serviceId={historyConnection.id}
            />
          )}
        </TabsContent>
      </Tabs>
    </div>
  );
}
