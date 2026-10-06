import { Link } from "@tanstack/react-router";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { ChevronRight, Clock3, History } from "lucide-react";
import { useServiceView } from "@/hooks/use-service-view";
import { useServiceCardTransition } from "@/hooks/use-service-card-transition";
import { ServiceViewToolbar } from "./service-view-toolbar";
import { ServiceConnectionTable } from "./service-connection-table";
import { ServiceAvatarStack } from "./service-avatar-stack";
import { ServiceBillingSummary } from "./service-billing-summary";
import { latestServiceEdit } from "@/lib/service-card-summary";
import {
  useServiceRoutingPools,
  type ServiceRoutingPools,
} from "@/hooks/use-service-routing-pools";
import { ServicePoolRoutingPanel } from "./service-pool-routing-panel";
import { ServicePoolSummary } from "./service-pool-summary";
import { useAuthStore } from "@/stores/auth-store";
import {
  connectionSourceLabel as sourceLabel,
  connectionSource,
  matchingConnections,
} from "@/lib/service-view";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn, formatRelativeTime } from "@/lib/utils";
import {
  groupServiceConnections,
  type ServiceConnectionGroup,
} from "@/lib/service-groups";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  callerLabel,
  latestRecordedUse,
  outcomeLabel,
} from "@/lib/service-insights";
import type { CatalogEntry, KeyInfo } from "@/types/keys";

function GroupCard({
  group,
  expanded,
  onToggle,
  insights,
  connections,
  search,
  renderConnectionActions,
  filtersRef,
  routing,
  allConnections,
  catalog,
}: {
  readonly catalog?: CatalogEntry;
  readonly routing: ServiceRoutingPools;
  readonly allConnections: readonly KeyInfo[];
  readonly group: ServiceConnectionGroup;
  readonly expanded: boolean;
  readonly onToggle: (card: HTMLElement | null) => void;
  readonly insights: ServiceInsightsState;
  readonly connections: readonly KeyInfo[];
  readonly search: string;
  readonly renderConnectionActions?: (key: KeyInfo) => ReactNode;
  readonly filtersRef: RefObject<HTMLDivElement | null>;
}) {
  const identity = useAuthStore((state) => state.user?.id);
  const [routingOpen, setRoutingOpen] = useState(false);
  const [requestedPanel, setRequestedPanel] = useState<{
    id: string;
    view: "billing" | "requests" | "access" | "history";
    version: number;
  } | null>(null);
  const [routeId, setRouteId] = useState<string | null>(null);
  const pools = routing.pools.filter((pool) =>
    pool.members.some((member) =>
      connections.some((key) => key.id === member.user_service_id),
    ),
  );
  const selectedPool = pools.find((pool) => pool.id === routeId) ?? pools[0];
  const contentId = useId();
  const headingId = useId();
  const cardRef = useRef<HTMLElement>(null);
  const headerRef = useRef<HTMLDivElement>(null);
  const headerOffset = useRef(0);
  const connectionIds = connections
    .map((connection) => connection.id)
    .join(",");
  const [headerStuck, setHeaderStuck] = useState(false);
  useLayoutEffect(() => {
    if (!expanded) return;
    const card = cardRef.current;
    const header = headerRef.current;
    const scroller = card?.closest("main");
    if (!card || !header || !scroller) return;
    const rows = card.querySelectorAll<HTMLElement>(
      "[data-service-connection-row]",
    );
    const finalRowsStart = rows[Math.max(0, rows.length - 3)];
    const update = () => {
      const bounds = header.getBoundingClientRect();
      const nativeTop = bounds.top - headerOffset.current;
      const offset = finalRowsStart
        ? Math.min(
            0,
            finalRowsStart.getBoundingClientRect().top -
              nativeTop -
              bounds.height,
          )
        : 0;
      headerOffset.current = offset;
      header.style.translate = offset ? `0 ${offset}px` : "";
      setHeaderStuck(
        nativeTop > card.getBoundingClientRect().top + card.clientTop + 1,
      );
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(card);
    observer.observe(header);
    observer.observe(scroller);
    if (finalRowsStart) observer.observe(finalRowsStart);
    const filterSurface = filtersRef.current?.firstElementChild;
    if (filterSurface) observer.observe(filterSurface);
    scroller.addEventListener("scroll", update, { passive: true });
    return () => {
      observer.disconnect();
      scroller.removeEventListener("scroll", update);
      headerOffset.current = 0;
      header.style.translate = "";
    };
  }, [
    expanded,
    filtersRef,
    connectionIds,
    routingOpen,
    selectedPool?.id,
    requestedPanel?.version,
  ]);
  const count = group.connections.length;
  const matchingCount = connections.length;
  const sources = [
    ...new Map(
      connections.map((key) => {
        const type = connectionSource(key);
        const org =
          key.credential_source?.type === "org" ? key.credential_source : null;
        return [
          type === "org" ? org!.org_id : type,
          {
            id: type === "org" ? org!.org_id : type,
            type,
            name: sourceLabel(key),
            avatarUrl: org?.avatar_url,
            description:
              type === "personal"
                ? "Connections you own. Billing is shown separately."
                : type === "org"
                  ? "Connections owned by this organization. Billing is shown separately."
                  : "Connections using NyxID credentials. Billing is shown separately.",
          },
        ] as const;
      }),
    ).values(),
  ];
  const lastEdit = latestServiceEdit(connections);
  const disabled = connections.filter((key) => !key.is_active).length;
  const connectionInsights = connections.map((key) =>
    insights.connections.get(key.id),
  );
  const access = connectionInsights.map((item) => item?.usage?.access);
  const configuredKeys = [
    ...new Map(
      access.flatMap((item) =>
        (item?.keys ?? []).map((key) => [key.id, key.name] as const),
      ),
    ).values(),
  ];
  const accessIncomplete = access.some(
    (item) =>
      !item ||
      item.incomplete ||
      item.truncated ||
      item.visibility === "unavailable",
  );
  const latestUse = connectionInsights
    .map((item) => latestRecordedUse(item?.usage))
    .filter((request) => request !== undefined)
    .sort((a, b) => b.occurred_at.localeCompare(a.occurred_at))[0];
  const useTracked = connectionInsights.every((item) => {
    const activity = item?.usage?.activity;
    return (
      activity &&
      activity.tracking !== "unavailable" &&
      activity.visibility !== "unavailable"
    );
  });
  const ownUseOnly = connectionInsights.every(
    (item) => item?.usage?.activity.visibility === "own_requests",
  );
  const keysText = configuredKeys.length
    ? `${configuredKeys.length}${accessIncomplete ? "+" : ""} agent ${configuredKeys.length === 1 ? "key" : "keys"}`
    : accessIncomplete
      ? "Agent keys unverified"
      : "0 agent keys";
  const useText = latestUse
    ? `${ownUseOnly ? "Your last use" : "Last use"} ${formatRelativeTime(latestUse.occurred_at)}`
    : useTracked
      ? "No use recorded · 30d"
      : "Last use not reported";
  const agents =
    insights.status === "loading"
      ? { text: "Loading…", title: "Loading agent keys and use" }
      : insights.status === "restricted"
        ? { text: "Restricted", title: "Agent key access is restricted" }
        : insights.status !== "ready"
          ? {
              text: "—",
              title:
                insights.status === "unavailable"
                  ? "Agent keys and use are not reported by this server"
                  : "Agent keys and use couldn't load",
            }
          : {
              text: keysText,
              title: [
                configuredKeys.length
                  ? `Keys with access: ${configuredKeys.join(", ")}`
                  : accessIncomplete
                    ? "Key access incomplete"
                    : "No agent keys with access",
                latestUse
                  ? `${ownUseOnly ? "Your last use" : "Last use"}: ${callerLabel(latestUse.caller)}${latestUse.caller.app_name ? ` · ${latestUse.caller.app_name}` : ""} · ${outcomeLabel(latestUse.outcome)} · ${latestUse.occurred_at}`
                  : useTracked
                    ? "No recorded use with exact connection attribution in the last 30 days"
                    : "Use is not reported by this server",
              ].join("\n"),
            };
  const openSummary = (
    view: "billing" | "requests" | "access" | "history",
    connectionId?: string,
  ) => {
    const lastConnection =
      view === "requests"
        ? connections
            .map((connection) => ({
              id: connection.id,
              request: latestRecordedUse(
                insights.connections.get(connection.id)?.usage,
              ),
            }))
            .filter((item) => item.request)
            .sort((a, b) =>
              b.request!.occurred_at.localeCompare(a.request!.occurred_at),
            )[0]?.id
        : undefined;
    const id = connectionId ?? lastConnection ?? connections[0]?.id;
    if (!id) return;
    setRequestedPanel((current) => ({
      id,
      view,
      version: (current?.version ?? 0) + 1,
    }));
    setRoutingOpen(false);
    if (!expanded) onToggle(cardRef.current);
  };

  return (
    <section
      ref={cardRef}
      aria-labelledby={headingId}
      style={{
        viewTransitionName: `service-card-${headingId.replace(/[^a-zA-Z0-9-]/g, "")}`,
      }}
      className={cn(
        "min-w-0 scroll-mt-[calc(var(--service-filters-height,0px)+24px)] sm:scroll-mt-[calc(var(--service-filters-height,0px)+20px)] rounded-xl border border-border bg-card shadow-sm",
        expanded
          ? "sm:col-span-2 xl:col-span-3"
          : "relative focus-within:z-10 hover:z-10",
      )}
    >
      <div
        ref={headerRef}
        data-stuck={expanded && headerStuck}
        className={cn(
          "service-card-header",
          expanded
            ? "sticky top-[calc(var(--service-filters-height,0px)+24px)] z-10 sm:top-[calc(var(--service-filters-height,0px)+20px)]"
            : undefined,
        )}
      >
        <div
          className={cn(
            "relative flex flex-col bg-card",
            expanded ? "rounded-t-xl shadow-sm" : "h-72 rounded-xl",
          )}
        >
          <div className="flex min-h-0 flex-1 flex-col gap-2 p-4">
            <div className="flex items-start gap-3">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-border bg-background/50">
                <ServiceIcon
                  slug={group.iconSlug}
                  iconUrl={group.iconUrl}
                  size="md"
                />
              </div>
              <div className="min-w-0 flex-1">
                <h3
                  id={headingId}
                  className="text-[15px] font-semibold tracking-tight"
                >
                  <button
                    type="button"
                    onClick={() => onToggle(cardRef.current)}
                    aria-expanded={expanded}
                    aria-controls={contentId}
                    className="max-w-full cursor-pointer truncate text-left hover:text-primary focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring"
                  >
                    {group.name}
                  </button>
                </h3>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  {matchingCount < count
                    ? `${matchingCount} of ${count}`
                    : count}{" "}
                  {count === 1 ? "connection" : "connections"}
                </p>
              </div>
              {disabled > 0 && (
                <Badge variant="secondary">{disabled} disabled</Badge>
              )}
            </div>
            {!expanded && (
              <p className="line-clamp-2 h-8 shrink-0 text-xs leading-4 text-muted-foreground">
                {search.trim()
                  ? `Matches: ${connections.map((key) => key.label).join(" · ")}`
                  : group.description}
              </p>
            )}
            <div
              className={cn(
                "mt-auto text-xs",
                expanded
                  ? "grid items-center gap-x-6 gap-y-2 md:grid-cols-2"
                  : "space-y-0.5",
              )}
            >
              <ServiceBillingSummary
                connections={connections}
                insights={insights}
                catalog={catalog}
                serviceName={group.name}
                onOpen={(id) => openSummary("billing", id)}
              />
              <ServicePoolSummary
                pools={pools}
                loading={routing.loading}
                incomplete={routing.incomplete}
                serviceName={group.name}
                expanded={expanded && routingOpen}
                contentId={contentId}
                onOpen={() => {
                  setRoutingOpen(true);
                  if (!expanded) onToggle(cardRef.current);
                }}
              />
              <TooltipProvider delayDuration={100} disableHoverableContent>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      onClick={() => {
                        openSummary(latestUse ? "requests" : "access");
                      }}
                      aria-expanded={expanded}
                      aria-controls={contentId}
                      aria-label={`Show agent keys and use for ${group.name}`}
                      className="flex h-6 w-full min-w-0 items-center gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring"
                    >
                      <Clock3
                        className="size-3.5 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                      <span className="truncate">{agents.text}</span>
                      <span
                        className="shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      >
                        ·
                      </span>
                      <span className="shrink-0 whitespace-nowrap text-muted-foreground">
                        {insights.status === "ready"
                          ? useText
                          : "Last use unavailable"}
                      </span>
                    </button>
                  </TooltipTrigger>
                  <TooltipContent
                    collisionPadding={12}
                    className="max-w-[min(22rem,calc(100vw-2rem))] whitespace-pre-line break-words leading-relaxed [overflow-wrap:anywhere]"
                  >
                    {agents.title}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
              <div className="flex h-6 min-w-0 items-center justify-between gap-2">
                <button
                  type="button"
                  aria-label={`Show last edit for ${group.name}`}
                  aria-controls={contentId}
                  onClick={() =>
                    openSummary("history", lastEdit?.connection.id)
                  }
                  title={
                    lastEdit
                      ? `${lastEdit.edit.action_label ?? "Last edited"} · ${lastEdit.edit.at} · ${lastEdit.edit.actor.name} · ${lastEdit.connection.label}`
                      : "No edit attribution was reported for these connections"
                  }
                  className="flex min-w-0 items-center gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring"
                >
                  <History
                    className="size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <span className="truncate text-muted-foreground">
                    {lastEdit
                      ? `Last edit ${formatRelativeTime(lastEdit.edit.at)} · ${lastEdit.edit.actor.name}`
                      : "Last edit not recorded"}
                  </span>
                </button>
                <ServiceAvatarStack
                  items={sources}
                  label={`Show sources for ${group.name}`}
                />
              </div>
            </div>
          </div>
          <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-t border-border/70 px-4">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                if (expanded && routingOpen) setRoutingOpen(false);
                else {
                  setRoutingOpen(false);
                  onToggle(cardRef.current);
                }
              }}
              aria-expanded={expanded && !routingOpen}
              aria-controls={contentId}
              aria-label={`${expanded && !routingOpen ? "Collapse" : "Expand"} ${group.name} connections`}
            >
              <ChevronRight
                className={cn(
                  "size-3.5 transition-transform motion-reduce:transition-none",
                  expanded && "rotate-90",
                )}
              />
              {expanded && !routingOpen
                ? "Hide connections"
                : `View ${matchingCount} ${matchingCount === 1 ? "connection" : "connections"}`}
            </Button>
            <div className="flex items-center gap-2 pr-2">
              {matchingCount < count && (
                <span className="text-[11px] text-muted-foreground">
                  {matchingCount} of {count} match
                </span>
              )}
              <Link
                to="/keys/services/$groupId"
                params={{ groupId: group.id }}
                aria-label={`View all ${group.name} service details`}
                className="text-xs text-primary hover:underline"
              >
                Service details
              </Link>
            </div>
          </div>
        </div>
      </div>
      <div
        id={contentId}
        hidden={!expanded}
        className="overflow-hidden rounded-b-xl"
      >
        {expanded && (
          <div className="border-t border-border bg-background/30">
            {routingOpen ? (
              <>
                <div className="flex flex-wrap items-center gap-2 border-b px-4 py-3">
                  {pools.map((pool) => (
                    <Button
                      key={pool.id}
                      size="sm"
                      variant={
                        selectedPool?.id === pool.id ? "secondary" : "ghost"
                      }
                      onClick={() => setRouteId(pool.id)}
                    >
                      {pool.name}
                    </Button>
                  ))}
                  <Link
                    to="/keys"
                    search={{
                      tab: "pools",
                      view: "routing",
                      pool: selectedPool?.id,
                      org:
                        selectedPool && selectedPool.user_id !== identity
                          ? selectedPool.user_id
                          : undefined,
                    }}
                    className="ml-auto text-xs text-primary hover:underline"
                  >
                    Manage in Service Pools
                  </Link>
                </div>
                {selectedPool ? (
                  <ServicePoolRoutingPanel
                    key={selectedPool.id}
                    pool={selectedPool}
                    connections={allConnections}
                    insights={insights}
                  />
                ) : (
                  <p className="p-4 text-xs text-muted-foreground">
                    {routing.loading
                      ? "Loading saved pools…"
                      : routing.incomplete
                        ? "Some pools could not be inspected. Organization pool settings require admin access."
                        : "These connections use their individual slugs. Create a pool to give compatible connections one route with rotation or priority failover."}
                  </p>
                )}
                {selectedPool && routing.incomplete && (
                  <p className="px-4 pb-4 text-xs text-muted-foreground">
                    Additional organization pools may require admin access.
                  </p>
                )}
              </>
            ) : (
              <ServiceConnectionTable
                key={requestedPanel?.version ?? 0}
                initialPanel={requestedPanel}
                connections={connections}
                insights={insights}
                serviceName={group.name}
                renderActions={renderConnectionActions}
                catalog={catalog}
                pools={pools}
                onViewPool={(id) => {
                  setRouteId(id);
                  setRoutingOpen(true);
                }}
              />
            )}
          </div>
        )}
      </div>
    </section>
  );
}

export function GroupedServiceCards({
  keys,
  catalog,
  renderConnectionActions,
  actions,
  renderTable,
}: {
  readonly keys: readonly KeyInfo[];
  readonly catalog?: readonly CatalogEntry[];
  readonly renderConnectionActions?: (key: KeyInfo) => ReactNode;
  readonly actions?: ReactNode | ((compact: boolean) => ReactNode);
  readonly renderTable?: (keys: readonly KeyInfo[]) => ReactNode;
}) {
  const view = useServiceView();
  const animateCards = useServiceCardTransition();
  const containerRef = useRef<HTMLDivElement>(null);
  const filtersRef = useRef<HTMLDivElement>(null);
  const [filtersStuck, setFiltersStuck] = useState(false);
  useLayoutEffect(() => {
    const container = containerRef.current;
    const toolbar = filtersRef.current;
    if (!container || !toolbar) return;
    const surface = toolbar.firstElementChild as HTMLElement;
    const scroller = toolbar.closest("main");
    let pinned = false;
    let expandedHeight = surface.getBoundingClientRect().height;
    const updateShadow = () => {
      const toolbarTop = toolbar.getBoundingClientRect().top;
      const next = container.getBoundingClientRect().top < toolbarTop - 1;
      if (!pinned)
        expandedHeight = Math.max(
          surface.getBoundingClientRect().height,
          Number.parseFloat(toolbar.style.minHeight) || 0,
        );
      if (next !== pinned) {
        // Keep the original flow height: shrinking scrollHeight can clamp
        // scrollTop back across the pin threshold and repeatedly unpin it.
        toolbar.style.minHeight = next ? `${expandedHeight}px` : "";
        pinned = next;
      }
      setFiltersStuck(next);
    };
    const measure = () => {
      const bounds = surface.getBoundingClientRect();
      container.style.setProperty(
        "--service-filters-height",
        `${bounds.height}px`,
      );
      if (scroller) {
        const viewportLeft =
          scroller.getBoundingClientRect().left + scroller.clientLeft;
        container.style.setProperty(
          "--service-filter-gutter-left",
          `${Math.max(0, bounds.left - viewportLeft)}px`,
        );
        container.style.setProperty(
          "--service-filter-gutter-right",
          `${Math.max(0, viewportLeft + scroller.clientWidth - bounds.right)}px`,
        );
      }
      updateShadow();
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(surface);
    if (scroller) observer.observe(scroller);
    scroller?.addEventListener("scroll", updateShadow, { passive: true });
    return () => {
      observer.disconnect();
      scroller?.removeEventListener("scroll", updateShadow);
    };
  }, [view.accountId]);
  const { filters, expanded } = view;
  const groups = groupServiceConnections(keys, catalog);
  const visible = groups
    .map((group) => ({ group, matches: matchingConnections(group, filters) }))
    .filter(({ matches }) => matches.length > 0);
  const matchingKeys = visible.flatMap(({ matches }) => matches);
  const insights = useServiceInsights(renderTable ? [] : keys);
  const routing = useServiceRoutingPools(keys, !renderTable);

  return (
    <div ref={containerRef} className="space-y-6 [overflow-anchor:none]">
      <ServiceViewToolbar
        ref={filtersRef}
        key={view.accountId}
        view={view}
        keys={keys}
        groups={groups}
        stuck={filtersStuck}
        actions={actions}
      >
        <span className="text-xs text-muted-foreground" aria-live="polite">
          {visible.length} {visible.length === 1 ? "service" : "services"} ·{" "}
          {matchingKeys.length} matching{" "}
          {matchingKeys.length === 1 ? "connection" : "connections"}
        </span>
        {expanded.length > 0 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => animateCards(() => view.setExpanded([]))}
          >
            Collapse
          </Button>
        )}
      </ServiceViewToolbar>
      {visible.length ? (
        renderTable ? (
          renderTable(matchingKeys)
        ) : (
          <div className="grid items-start gap-6 sm:grid-cols-2 xl:grid-cols-3">
            {visible.map(({ group, matches }) => (
              <GroupCard
                key={group.id}
                group={group}
                expanded={expanded.includes(group.id)}
                insights={insights}
                routing={routing}
                allConnections={keys}
                catalog={catalog?.find((entry) => entry.slug === group.slug)}
                connections={matches}
                search={filters.search}
                onToggle={(card) =>
                  animateCards(
                    () =>
                      view.setExpanded(
                        expanded.includes(group.id) ? [] : [group.id],
                      ),
                    expanded.includes(group.id) ? undefined : card,
                    filtersRef.current,
                  )
                }
                renderConnectionActions={renderConnectionActions}
                filtersRef={filtersRef}
              />
            ))}
          </div>
        )
      ) : (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {keys.length
            ? filters.source === "personal"
              ? "No services with a personal connection match this view. Choose All services to include services available only through an organization or the platform."
              : "No services match these filters. Clear filters to see all services."
            : "No connected services."}
        </p>
      )}
    </div>
  );
}
