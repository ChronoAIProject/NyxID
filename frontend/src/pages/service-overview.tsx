import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { useEffect, useId, useRef, useState } from "react";
import { useServiceGroupOrder } from "@/hooks/use-service-group-order";
import { ServiceAgentOrderPanel } from "@/components/dashboard/service-agent-order-panel";
import { ServiceOrderActions } from "@/components/dashboard/service-order-actions";
import { Button } from "@/components/ui/button";
import { Link, useParams, useBlocker } from "@tanstack/react-router";
import { ArrowLeft, ListOrdered } from "lucide-react";
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
import { useServiceRoutingPools } from "@/hooks/use-service-routing-pools";

export function ServiceOverviewPage() {
  const { groupId } = useParams({ strict: false }) as { groupId: string };
  const keys = useKeys();
  const catalog = useCatalog({ includeAll: true });
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
  const agentOrder = useServiceGroupOrder(connections);
  const orderFormId = useId();
  const routing = useServiceRoutingPools(agentOrder.inventory);
  useBlocker({
    shouldBlockFn: () => !agentOrder.guard(),
    enableBeforeUnload: agentOrder.dirty,
  });
  const orderButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (agentOrder.focusGroup !== groupId || agentOrder.reason) return;
    const frame = requestAnimationFrame(() => {
      if (orderButton.current && !orderButton.current.disabled) {
        orderButton.current.focus();
        agentOrder.clearFocus();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [agentOrder, groupId]);
  const group =
    groupServiceConnections(agentOrder.inventory, catalog.data).find(
      (group) => group.id === groupId,
    ) ??
    (agentOrder.editingGroup?.id === groupId
      ? agentOrder.editingGroup
      : undefined);
  const pools = routing.pools.filter((pool) =>
    pool.members.some((member) =>
      group?.connections.some((key) => key.id === member.user_service_id),
    ),
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
  if (keys.error && !agentOrder.groupId)
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
              className="text-primary-text hover:underline"
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
      <Tabs
        className="sm:pt-1"
        value={tab}
        activationMode={agentOrder.dirty ? "manual" : "automatic"}
        onValueChange={(value) => {
          if (agentOrder.guard()) setTab(value);
        }}
      >
        <div
          data-service-order-actions
          className="sticky top-0 z-10 bg-background before:absolute before:inset-x-0 before:-top-4 before:h-4 before:bg-background sm:before:-top-6 sm:before:h-6"
        >
          <div className="flex min-h-12 flex-wrap items-center gap-4 py-2">
            <TabsList>
              <TabsTrigger value="connections">Connections</TabsTrigger>
              <TabsTrigger value="history">History</TabsTrigger>
            </TabsList>
          </div>
          {tab === "connections" && (
            <ServiceAgentOrderPanel
              group={group}
              order={agentOrder}
              className="bg-background px-0"
              actions={
                agentOrder.groupId === group.id ? (
                  <ServiceOrderActions order={agentOrder} formId={orderFormId} />
                ) : (
                  <Button
                    ref={orderButton}
                    type="button"
                    variant="primary"
                    size="sm"
                    disabled={Boolean(agentOrder.reason)}
                    title={
                      agentOrder.reason ??
                      "Reorder discovery within this service"
                    }
                    onClick={() => agentOrder.start(group)}
                  >
                    <ListOrdered className="size-4" aria-hidden="true" />
                    Reorder discovery
                  </Button>
                )
              }
            />
          )}
        </div>
        <TabsContent
          value="connections"
          className="mt-4 overflow-hidden rounded-xl border border-border/50 bg-card"
        >
          {keys.error && (
            <ErrorBanner
              message="Failed to refresh connections. Your edits are kept."
              onRetry={keys.refetch}
            />
          )}
          <ServiceConnectionTable
            catalog={entry}
            pools={pools}
            connections={
              agentOrder.groupId === group.id
                ? agentOrder.connections
                : group.connections
            }
            ordering={agentOrder.groupId === group.id ? agentOrder : undefined}
            orderFormId={orderFormId}
            externalOrderActions
            savedOrder={agentOrder.savedOrder(group.id)}
            serviceName={group.name}
            onViewHistory={(connection) => {
              if (!agentOrder.guard()) return;
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
