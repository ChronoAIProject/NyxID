import { Link } from "@tanstack/react-router";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { ChevronRight, UsersRound } from "lucide-react";
import { useServiceView } from "@/hooks/use-service-view";
import { useServiceCardTransition } from "@/hooks/use-service-card-transition";
import { ServiceViewToolbar } from "./service-view-toolbar";
import { ServiceConnectionTable } from "./service-connection-table";
import { ServiceAvatarStack } from "./service-avatar-stack";
import { ServiceBillingSummary } from "./service-billing-summary";
import {
  connectionSourceLabel as sourceLabel,
  connectionSource,
  matchingConnections,
} from "@/lib/service-view";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
}: {
  readonly group: ServiceConnectionGroup;
  readonly expanded: boolean;
  readonly onToggle: (card: HTMLElement | null) => void;
  readonly insights: ServiceInsightsState;
  readonly connections: readonly KeyInfo[];
  readonly search: string;
  readonly renderConnectionActions?: (key: KeyInfo) => ReactNode;
  readonly filtersRef: RefObject<HTMLDivElement | null>;
}) {
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
  }, [expanded, filtersRef, connectionIds]);
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
          },
        ] as const;
      }),
    ).values(),
  ];
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
    ? `${configuredKeys.length} ${configuredKeys.length === 1 ? "key" : "keys"}`
    : accessIncomplete
      ? "keys —"
      : "no keys";
  const useText = latestUse
    ? `${callerLabel(latestUse.caller)} · ${formatRelativeTime(latestUse.occurred_at)}`
    : "use not recorded";
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
              text: `${keysText} · ${useText}`,
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
    view: "billing" | "requests" | "access",
    connectionId?: string,
  ) => {
    if (!expanded) {
      onToggle(cardRef.current);
      return;
    }
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
    const buttons = [
      ...(cardRef.current?.querySelectorAll<HTMLButtonElement>(
        `[data-insight-view="${view}"]`,
      ) ?? []),
    ];
    const button =
      buttons.find(
        (item) =>
          item.dataset.connectionId === (connectionId ?? lastConnection),
      ) ?? buttons[0];
    button?.click();
    button?.focus({ preventScroll: true });
  };

  return (
    <section
      ref={cardRef}
      aria-labelledby={headingId}
      style={{
        viewTransitionName: `service-card-${headingId.replace(/[^a-zA-Z0-9-]/g, "")}`,
      }}
      className={cn(
        "min-w-0 scroll-mt-[calc(var(--service-filters-height,0px)+32px)] rounded-xl border border-border bg-card shadow-sm",
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
            ? "sticky top-[calc(var(--service-filters-height,0px)+32px)] z-10"
            : undefined,
        )}
      >
        <div
          className={cn(
            "relative flex flex-col rounded-t-xl bg-card",
            expanded ? "shadow-sm" : "h-64",
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
                  ? "grid gap-x-6 gap-y-2 md:grid-cols-2"
                  : "space-y-1.5",
              )}
            >
              {!expanded && (
                <div className="flex h-6 min-w-0 items-center gap-2">
                  <span className="w-16 shrink-0 text-muted-foreground">
                    Sources
                  </span>
                  <ServiceAvatarStack
                    items={sources}
                    label={`Show sources for ${group.name}`}
                  />
                </div>
              )}
              <button
                type="button"
                onClick={() => {
                  openSummary(latestUse ? "requests" : "access");
                }}
                aria-expanded={expanded}
                aria-controls={contentId}
                aria-label={`Show agent keys and use for ${group.name}`}
                className="flex h-6 w-full min-w-0 items-center gap-2 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-ring"
              >
                <span className="w-16 shrink-0 text-muted-foreground">
                  Agents
                </span>
                <UsersRound
                  className="size-3.5 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
                <span className="truncate" title={agents.title}>
                  {agents.text}
                </span>
              </button>
              <ServiceBillingSummary
                connections={connections}
                insights={insights}
                serviceName={group.name}
                onOpen={(id) => openSummary("billing", id)}
              />
            </div>
          </div>
          <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-t border-border/70 px-4">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onToggle(cardRef.current)}
              aria-expanded={expanded}
              aria-controls={contentId}
              aria-label={`${expanded ? "Collapse" : "Expand"} ${group.name} connections`}
            >
              <ChevronRight
                className={cn(
                  "size-3.5 transition-transform motion-reduce:transition-none",
                  expanded && "rotate-90",
                )}
              />
              {expanded
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
            <ServiceConnectionTable
              connections={connections}
              insights={insights}
              serviceName={group.name}
              renderActions={renderConnectionActions}
            />
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
