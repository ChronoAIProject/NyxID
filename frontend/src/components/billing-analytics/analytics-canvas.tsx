import { useId, useState } from "react";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  pointerWithin,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
} from "@dnd-kit/sortable";
import {
  ArrowDown,
  ArrowUp,
  Copy,
  Plus,
  Settings2,
  Ellipsis,
  Trash2,
} from "lucide-react";
import { useUsageAnalytics } from "@/hooks/use-usage-analytics";
import { usePanelVisibility } from "@/hooks/use-panel-visibility";
import {
  BREAKDOWN_LABELS,
  MEASURE_LABELS,
  filterError,
  formatAnalyticsValue,
  newPanel,
  duplicatePanel,
  panelSpan,
} from "@/lib/usage-analytics";
import type {
  AnalyticsFilters,
  AnalyticsPanel,
  AnalyticsResult,
  AnalyticsView,
} from "@/schemas/usage-analytics";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { cn } from "@/lib/utils";
import { ChartView } from "./chart-view";
import { AnalyticsSelect, PanelControls } from "./controls";
import { SortablePanel } from "./sortable-panel";
import "./operations.css";

export type AnalyticsSample = (
  filters: AnalyticsFilters,
  panel: AnalyticsPanel,
) => AnalyticsResult;
function AnalyticsPanelCard({
  panel,
  view,
  onChange,
  sample,
  children,
}: {
  panel: AnalyticsPanel;
  view: AnalyticsView;
  onChange: (view: AnalyticsView) => void;
  sample?: AnalyticsSample;
  children?: React.ReactNode;
}) {
  const { ref, visible, hasEntered } = usePanelVisibility();
  const query = useUsageAnalytics(view.filters, panel, sample, visible);
  const compact = view.layout === "operations";
  function drilldown(id: string) {
    const field =
      panel.breakdown === "service"
        ? "services"
        : panel.breakdown === "user"
          ? "actors"
          : "owners";
    onChange({ ...view, filters: { ...view.filters, [field]: [id] } });
  }
  return (
    <Card
      ref={ref}
      role="article"
      className={cn(
        "analytics-panel min-w-0 overflow-visible p-4",
        compact && "analytics-panel-compact",
      )}
      aria-label={panel.title}
    >
      <div className="mb-4 flex shrink-0 items-start justify-between gap-3">
        <div className="min-w-0">
          <h3
            className="truncate font-display text-[15px] font-semibold leading-none tracking-tight"
            title={panel.title}
          >
            {panel.title || "Untitled panel"}
          </h3>
          <p className="mt-1 text-[12px] text-muted-foreground">
            {MEASURE_LABELS[panel.measure]} ·{" "}
            {panel.top === 0
              ? "All selected usage"
              : BREAKDOWN_LABELS[panel.breakdown]}
          </p>
        </div>
        {children}
      </div>
      {filterError(view.filters) ? (
        <div
          className={cn(
            "flex items-center justify-center text-[12px] text-muted-foreground",
            compact ? "h-[260px]" : "h-[330px]",
          )}
        >
          Choose a valid time range to query usage.
        </div>
      ) : !hasEntered || query.isPending ? (
        <Skeleton className={compact ? "h-[260px]" : "h-[330px]"} />
      ) : query.isError ? (
        <div
          className={cn(
            "flex items-center",
            compact ? "min-h-[260px]" : "min-h-[330px]",
          )}
        >
          <ErrorBanner
            message={
              query.error.message ||
              "Could not load analytics. Try a narrower range."
            }
            onRetry={() => void query.refetch()}
          />
        </div>
      ) : (
        <ChartView
          data={query.data}
          panel={panel}
          onSelect={
            panel.breakdown === "credential_class" ? undefined : drilldown
          }
          compact={compact}
        />
      )}
    </Card>
  );
}
function Summary({
  view,
  sample,
}: {
  view: AnalyticsView;
  sample?: AnalyticsSample;
}) {
  const query = useUsageAnalytics(
    view.filters,
    {
      ...view.panels[0]!,
      measure: "cost",
      metric: "tokens",
      breakdown: "service",
      top: 5,
    },
    sample,
  );
  if (filterError(view.filters)) return null;
  if (query.isPending)
    return (
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {[0, 1, 2, 3].map((i) => (
          <Skeleton key={i} className="h-28" />
        ))}
      </div>
    );
  if (!query.data || query.isError) return null;
  const data = query.data;
  const stats = [
    {
      label: "Gross cost",
      value: formatAnalyticsValue(data.total, "microcredits", true),
      suffix: "credits",
      note: data.totals.unknown_cost_events
        ? "Some costs are unavailable"
        : "Before allowances and grants",
    },
    {
      label: "Requests",
      value: formatAnalyticsValue(data.totals.requests, "requests", true),
      suffix: "requests",
      note: "One count per metered request",
    },
    {
      label: "Total tokens",
      value: formatAnalyticsValue(data.totals.total_tokens, "tokens", true),
      suffix: "tokens",
      note: "Input + output tokens",
    },
    {
      label: "Active users",
      value: data.totals.unique_users.toLocaleString(),
      suffix: "users",
      note: `${data.totals.unique_services.toLocaleString()} services in this selection`,
    },
  ];
  return (
    <div
      className={cn(
        "grid grid-cols-2 gap-3 lg:grid-cols-4",
        view.layout === "operations" && "analytics-summary",
      )}
    >
      {stats.map((stat) => (
        <Card key={stat.label} className="min-w-0 px-4 py-4">
          <p className="text-[10px] font-medium uppercase tracking-[1.5px] text-muted-foreground">
            {stat.label}
          </p>
          <div className="mt-3 flex flex-wrap items-baseline gap-x-2">
            <span className="font-display text-[28px] font-medium leading-none tracking-tight tabular-nums">
              {stat.value}
            </span>
            <span className="text-[10px] text-muted-foreground">
              {stat.suffix}
            </span>
          </div>
          <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">
            {stat.note}
          </p>
        </Card>
      ))}
    </div>
  );
}
export function AnalyticsCanvas({
  view,
  onChange,
  sample,
  disabled = false,
}: {
  view: AnalyticsView;
  onChange: (view: AnalyticsView) => void;
  sample?: AnalyticsSample;
  disabled?: boolean;
}) {
  const operations = view.layout === "operations";
  const dragId = useId();
  const [activeId, setActiveId] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const activePanel = view.panels.find((panel) => panel.id === activeId);
  function updatePanel(panel: AnalyticsPanel) {
    onChange({
      ...view,
      panels: view.panels.map((current) =>
        current.id === panel.id ? panel : current,
      ),
    });
  }
  function move(index: number, direction: number) {
    reorder(index, index + direction);
  }
  function reorder(from: number, to: number) {
    if (
      disabled ||
      from < 0 ||
      to < 0 ||
      from === to ||
      to >= view.panels.length
    )
      return;
    onChange({ ...view, panels: arrayMove(view.panels, from, to) });
    setAnnouncement(
      `${view.panels[from]!.title} moved to position ${to + 1} of ${view.panels.length}.`,
    );
  }
  if (view.layout === "explorer") {
    const panel = view.panels[0]!;
    return (
      <div className="grid items-start gap-4 lg:grid-cols-[260px_minmax(0,1fr)]">
        <aside className="rounded-xl border border-border bg-card p-4 text-card-foreground shadow-sm">
          <div className="mb-5 border-b border-border/50 pb-4">
            <h3 className="text-[15px] font-semibold">Build an insight</h3>
            <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
              Choose what to measure and how to see it.
            </p>
          </div>
          <PanelControls panel={panel} onChange={updatePanel} />
          <Button
            className="mt-5 w-full"
            onClick={() =>
              onChange({
                ...view,
                name: "My operations",
                layout: "operations",
                panels: [
                  panel,
                  newPanel({
                    title: "Request traffic",
                    measure: "requests",
                    chart: "line",
                  }),
                ],
              })
            }
          >
            <Plus className="size-3" />
            Use in Operations
          </Button>
        </aside>
        <AnalyticsPanelCard
          panel={panel}
          view={view}
          onChange={onChange}
          sample={sample}
        />
      </div>
    );
  }
  return (
    <div
      className={cn(
        "analytics-board space-y-5",
        operations && "analytics-operations",
      )}
    >
      <span className="sr-only" aria-label="Panel order" aria-live="polite">
        {announcement}
      </span>
      <Summary view={view} sample={sample} />
      {operations && (
        <div className="flex items-center justify-between gap-3">
          <p className="text-[10px] font-semibold uppercase tracking-[1.5px] text-muted-foreground">
            Usage monitors{" "}
            <span className="ml-2 font-mono font-normal">
              {view.panels.length} panels
            </span>
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              onChange({ ...view, panels: [...view.panels, newPanel()] })
            }
          >
            <Plus className="size-3" />
            Add panel
          </Button>
        </div>
      )}
      <DndContext
        id={dragId}
        sensors={sensors}
        collisionDetection={(args) => {
          const targets = pointerWithin(args);
          return targets.length ? targets : closestCenter(args);
        }}
        onDragStart={({ active }) => setActiveId(String(active.id))}
        onDragCancel={() => setActiveId(null)}
        onDragEnd={({ active, over }) => {
          setActiveId(null);
          if (over)
            reorder(
              view.panels.findIndex((p) => p.id === active.id),
              view.panels.findIndex((p) => p.id === over.id),
            );
        }}
        accessibility={{
          screenReaderInstructions: {
            draggable:
              "Press Space to pick up a panel, arrow keys to move it, Space to drop, or Escape to cancel.",
          },
          announcements: {
            onDragStart: ({ active }) =>
              `Picked up ${view.panels.find((p) => p.id === active.id)?.title ?? "panel"}.`,
            onDragOver: ({ over }) =>
              over
                ? `Over position ${view.panels.findIndex((p) => p.id === over.id) + 1} of ${view.panels.length}.`
                : "Outside the panel grid.",
            onDragEnd: ({ over }) =>
              over
                ? `Dropped at position ${view.panels.findIndex((p) => p.id === over.id) + 1}.`
                : "Move cancelled.",
            onDragCancel: () => "Move cancelled.",
          },
        }}
      >
        <SortableContext
          items={view.panels.map((p) => p.id)}
          strategy={() => null}
        >
          <div
            className={cn(
              "grid",
              operations
                ? "analytics-operations-grid items-stretch"
                : "analytics-overview-grid items-start gap-4",
            )}
          >
            {view.panels.map((panel, index) => (
              <SortablePanel key={panel.id} panel={panel} disabled={disabled}>
                {(handle) => (
                  <AnalyticsPanelCard
                    panel={panel}
                    view={view}
                    onChange={onChange}
                    sample={sample}
                  >
                    <div className="flex shrink-0 items-center gap-0.5">
                      {handle}
                      <Popover>
                        <PopoverTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-7 shrink-0"
                            aria-label={`Configure ${panel.title}`}
                          >
                            {operations ? (
                              <Ellipsis className="size-3.5" />
                            ) : (
                              <Settings2 className="size-3.5" />
                            )}
                          </Button>
                        </PopoverTrigger>
                        <PopoverContent
                          align="end"
                          collisionPadding={12}
                          className="max-h-[min(80vh,var(--radix-popover-content-available-height))] w-72 space-y-4 overflow-y-auto p-4"
                        >
                          <PanelControls panel={panel} onChange={updatePanel} />
                          <AnalyticsSelect
                            label="Panel width"
                            value={String(panelSpan(panel))}
                            onChange={(value) =>
                              updatePanel({
                                ...panel,
                                span: Number(value) as 1 | 2 | 3,
                                wide: value === "3",
                              })
                            }
                            options={[
                              { value: "1", label: "1 column" },
                              { value: "2", label: "2 columns" },
                              { value: "3", label: "3 columns · full width" },
                            ]}
                          />
                          <AnalyticsSelect
                            label="Chart height"
                            value={
                              panel.height ??
                              (operations ? "compact" : "standard")
                            }
                            onChange={(height) =>
                              updatePanel({
                                ...panel,
                                height: height as AnalyticsPanel["height"],
                              })
                            }
                            options={[
                              { value: "compact", label: "Compact" },
                              { value: "standard", label: "Standard" },
                              { value: "tall", label: "Tall" },
                            ]}
                          />
                          <div className="flex flex-wrap gap-1 border-t border-border pt-3">
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() =>
                                onChange({
                                  ...view,
                                  panels: [
                                    ...view.panels,
                                    duplicatePanel(panel),
                                  ],
                                })
                              }
                            >
                              <Copy className="size-3" />
                              Duplicate
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              aria-label={`Move ${panel.title} up`}
                              disabled={index === 0}
                              onClick={() => move(index, -1)}
                            >
                              <ArrowUp className="size-3" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              aria-label={`Move ${panel.title} down`}
                              disabled={index === view.panels.length - 1}
                              onClick={() => move(index, 1)}
                            >
                              <ArrowDown className="size-3" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              aria-label={`Remove ${panel.title}`}
                              disabled={view.panels.length === 1}
                              onClick={() =>
                                onChange({
                                  ...view,
                                  panels: view.panels.filter(
                                    (current) => current.id !== panel.id,
                                  ),
                                })
                              }
                            >
                              <Trash2 className="size-3 text-destructive" />
                            </Button>
                          </div>
                        </PopoverContent>
                      </Popover>
                    </div>
                  </AnalyticsPanelCard>
                )}
              </SortablePanel>
            ))}
            <Button
              variant="outline"
              className="h-24 border-dashed text-muted-foreground"
              onClick={() =>
                onChange({ ...view, panels: [...view.panels, newPanel()] })
              }
            >
              <Plus className="size-4" />
              Add a panel
            </Button>
          </div>
        </SortableContext>
        <DragOverlay dropAnimation={null}>
          {activePanel && (
            <Card className="border-primary p-4 shadow-xl">
              <p className="text-[15px] font-semibold">{activePanel.title}</p>
              <p className="mt-1 text-[12px] text-muted-foreground">
                Move to a new position
              </p>
            </Card>
          )}
        </DragOverlay>
      </DndContext>
    </div>
  );
}
