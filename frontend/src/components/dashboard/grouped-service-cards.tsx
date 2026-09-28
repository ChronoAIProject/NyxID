import { Link } from "@tanstack/react-router";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { ChevronRight } from "lucide-react";
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
import type { CatalogEntry, KeyInfo } from "@/types/keys";

function GroupCard({
  group,
  expanded,
  onToggle,
  search,
  connections,
  renderConnectionActions,
  filtersRef,
}: {
  readonly group: ServiceConnectionGroup;
  readonly expanded: boolean;
  readonly onToggle: (card: HTMLElement | null) => void;
  readonly search: string;
  readonly connections: readonly KeyInfo[];
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
    if (filtersRef.current) observer.observe(filtersRef.current);
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
  const matches = search
    ? connections.filter((key) =>
        [key.label, key.slug, sourceLabel(key)].some((value) =>
          value.toLowerCase().includes(search),
        ),
      )
    : [];

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
            expanded ? "shadow-sm" : "min-h-64",
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
            {!expanded && (
              <div className="mt-auto space-y-2 text-xs">
                <p
                  className="flex min-w-0 items-center gap-2 overflow-hidden text-muted-foreground"
                  title={sources.map((source) => source.name).join(" · ")}
                >
                  <span className="mr-2">Sources</span>
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
                {matches.length > 0 && (
                  <p
                    className="truncate text-muted-foreground"
                    title={matches.map((key) => key.label).join(", ")}
                  >
                    Matches: {matches.map((key) => key.label).join(", ")}
                  </p>
                )}
              </div>
            )}
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
    const scroller = toolbar.closest("main");
    const updateShadow = () => {
      const toolbarTop = toolbar.getBoundingClientRect().top;
      setFiltersStuck(container.getBoundingClientRect().top < toolbarTop - 1);
    };
    const measure = () => {
      const bounds = toolbar.getBoundingClientRect();
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
    observer.observe(toolbar);
    if (scroller) observer.observe(scroller);
    scroller?.addEventListener("scroll", updateShadow, { passive: true });
    return () => {
      observer.disconnect();
      scroller?.removeEventListener("scroll", updateShadow);
    };
  }, [view.accountId]);
  const { filters, expanded } = view;
  const needle = filters.search.trim().toLowerCase();
  const groups = groupServiceConnections(keys, catalog);
  const visible = groups
    .map((group) => ({ group, matches: matchingConnections(group, filters) }))
    .filter(({ matches }) => matches.length > 0);
  const matchingKeys = visible.flatMap(({ matches }) => matches);

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
                search={needle}
                connections={matches}
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
