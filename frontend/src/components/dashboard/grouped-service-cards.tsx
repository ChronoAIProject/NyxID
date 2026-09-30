import { Link } from "@tanstack/react-router";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { ChevronRight, CreditCard, UsersRound } from "lucide-react";
import { useServiceView } from "@/hooks/use-service-view";
import { useServiceCardTransition } from "@/hooks/use-service-card-transition";
import { ServiceViewToolbar } from "./service-view-toolbar";
import { ServiceConnectionTable } from "./service-connection-table";
import { ServiceOwnerAvatar } from "./service-owner-avatar";
import {
  connectionSourceLabel as sourceLabel,
  connectionSource,
  matchingConnections,
} from "@/lib/service-view";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  groupServiceConnections,
  type ServiceConnectionGroup,
} from "@/lib/service-groups";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  summarizeBilling,
  summarizeBillingDetail,
  latestRecordedUse,
  insightStatusLabel,
} from "@/lib/service-insights";
import { ServiceUseSummary } from "./service-use-summary";
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
          { type, name: sourceLabel(key), avatarUrl: org?.avatar_url },
        ] as const;
      }),
    ).values(),
  ];
  const disabled = connections.filter((key) => !key.is_active).length;
  const connectionInsights = connections.map((key) =>
    insights.connections.get(key.id),
  );
  const billingSummary =
    insights.status !== "ready"
      ? insightStatusLabel(insights.status, "Billing")
      : summarizeBilling(
          connections
            .filter((key) => key.is_active)
            .map((key) => insights.connections.get(key.id)),
        );
  const activeInsights = connections
    .filter((key) => key.is_active)
    .map((key) => insights.connections.get(key.id));
  const billingDetail =
    insights.status === "ready"
      ? summarizeBillingDetail(activeInsights)
      : "Rates unavailable";
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
  const accessSummary = configuredKeys.length
    ? `${configuredKeys.slice(0, 2).join(" · ")}${configuredKeys.length > 2 ? ` +${configuredKeys.length - 2}` : ""}`
    : accessIncomplete
      ? "Key access incomplete"
      : "No matching keys";
  const openSummary = (view: "billing" | "requests" | "access") => {
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
      buttons.find((item) => item.dataset.connectionId === lastConnection) ??
      buttons[0];
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
        expanded ? "sm:col-span-2 xl:col-span-3" : "overflow-hidden",
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
            expanded ? "shadow-sm" : "min-h-80",
          )}
        >
          <div className="flex flex-1 flex-col gap-3 p-5">
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
              <p className="line-clamp-2 min-h-8 text-xs leading-4 text-muted-foreground">
                {group.description}
              </p>
            )}
            <div
              className={cn(
                "mt-auto text-xs",
                expanded ? "grid gap-4 md:grid-cols-3" : "space-y-3",
              )}
            >
              {!expanded && (
                <p
                  className="flex min-w-0 items-center gap-2 overflow-hidden text-muted-foreground"
                  title={sources.map((source) => source.name).join(" · ")}
                >
                  <span className="w-24 shrink-0">Sources</span>
                  {sources.map((source, index) => (
                    <span
                      key={index}
                      className="inline-flex min-w-0 items-center gap-1.5 text-foreground"
                    >
                      <ServiceOwnerAvatar {...source} />
                      <span className="truncate">{source.name}</span>
                    </span>
                  ))}
                </p>
              )}
              <button
                type="button"
                onClick={() => {
                  openSummary("billing");
                }}
                aria-expanded={expanded}
                aria-controls={contentId}
                aria-label={`Expand ${group.name} to compare billing`}
                className="grid w-full grid-cols-[6rem_minmax(0,1fr)] items-start gap-2 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-ring"
              >
                <span className="pt-0.5 text-muted-foreground">Billing</span>
                <span className="min-w-0">
                  <span className="flex min-h-5 min-w-0 items-center gap-1.5 font-medium">
                    <CreditCard className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate" title={billingSummary}>
                      {billingSummary}
                    </span>
                  </span>
                  <span
                    className="mt-1 block min-h-8 text-[11px] leading-4 text-muted-foreground line-clamp-2"
                    title={billingDetail}
                  >
                    {billingDetail}
                  </span>
                </span>
              </button>
              <ServiceUseSummary
                connections={connections}
                insights={insights}
                onClick={() => {
                  openSummary("requests");
                }}
                expanded={expanded}
                controls={contentId}
              />
              <button
                type="button"
                onClick={() => {
                  openSummary("access");
                }}
                aria-expanded={expanded}
                aria-controls={contentId}
                aria-label={`Expand ${group.name} to compare agent key scope`}
                className="grid w-full grid-cols-[6rem_minmax(0,1fr)] items-start gap-2 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-ring"
              >
                <span className="pt-0.5 text-muted-foreground">Agent keys</span>
                <span className="flex min-h-5 min-w-0 items-center gap-1.5">
                  <UsersRound className="size-3.5 shrink-0 text-muted-foreground" />
                  <span
                    className="truncate"
                    title={`Configured scope: ${accessSummary}`}
                  >
                    {accessSummary}
                  </span>
                </span>
              </button>
              {search.trim() && (
                <p
                  className="truncate text-[11px] text-muted-foreground"
                  title={connections.map((key) => key.label).join(" · ")}
                >
                  Matches: {connections.map((key) => key.label).join(" · ")}
                </p>
              )}
            </div>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border/70 px-4 py-3">
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
  readonly actions?: ReactNode;
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
        <div className="ml-auto">{actions}</div>
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
              ? "No personal services match this view. Choose All services to include organization and platform connections."
              : "No services match these filters. Clear filters to see all services."
            : "No connected services."}
        </p>
      )}
    </div>
  );
}
