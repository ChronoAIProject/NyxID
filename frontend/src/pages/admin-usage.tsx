import { lazy, Suspense, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import {
  ChartNoAxesCombined,
  Check,
  Copy,
  LayoutDashboard,
  RefreshCw,
  Save,
  Search,
  Trash2,
} from "lucide-react";
import { useAuthStore } from "@/stores/auth-store";
import {
  useAutosavedWorkspace,
  useUsageWorkspace,
} from "@/hooks/use-usage-workspace";
import { newView, TEMPLATE_COPY } from "@/lib/usage-analytics";
import {
  LAYOUTS,
  type AnalyticsLayout,
  type WorkspaceResponse,
} from "@/schemas/usage-analytics";
import {
  AnalyticsCanvas,
  type AnalyticsSample,
} from "@/components/billing-analytics/analytics-canvas";
import {
  FilterBar,
  type SampleOptions,
} from "@/components/billing-analytics/controls";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { AdminUsageList } from "@/components/billing-analytics/usage-list";
import { PageHeader } from "@/components/shared/page-header";
import { cn } from "@/lib/utils";

const SamplePage =
  import.meta.env.DEV && import.meta.env.MODE === "test"
    ? lazy(() =>
        import("@/components/billing-analytics/samples").then((module) => ({
          default: module.AnalyticsSamples,
        })),
      )
    : null;
export function AdminUsagePage() {
  const search = useSearch({ from: "/dashboard/admin/usage" });
  const { sample, tab = "dashboard" } = search;
  const navigate = useNavigate();
  if (SamplePage && sample)
    return (
      <Suspense fallback={<Skeleton className="h-96" />}>
        <SamplePage layout={sample} />
      </Suspense>
    );
  return (
    <LiveAnalytics
      tab={tab}
      onTabChange={(next) =>
        void navigate({ to: "/admin/usage", search: { ...search, tab: next } })
      }
    />
  );
}
function LiveAnalytics({
  tab,
  onTabChange,
}: {
  tab: "dashboard" | "list";
  onTabChange: (tab: "dashboard" | "list") => void;
}) {
  const user = useAuthStore((state) => state.user);
  const query = useUsageWorkspace(user?.id ?? "");
  if (query.isPending || query.isFetching)
    return (
      <div className="space-y-4">
        <Skeleton className="h-20" />
        <Skeleton className="h-96" />
      </div>
    );
  if (query.isError)
    return (
      <ErrorBanner
        message="Could not load your analytics workspace."
        onRetry={() => void query.refetch()}
      />
    );
  return (
    <AnalyticsWorkspace
      key={user?.id}
      initial={query.data}
      userId={user?.id ?? ""}
      editable={user?.role === "admin" || user?.is_admin === true}
      tab={tab}
      onTabChange={onTabChange}
    />
  );
}
export function AnalyticsWorkspace({
  initial,
  userId,
  editable,
  sample,
  options,
  initialLayout,
  tab,
  onTabChange,
}: {
  initial: WorkspaceResponse;
  userId: string;
  editable: boolean;
  sample?: AnalyticsSample;
  options?: SampleOptions;
  initialLayout?: AnalyticsLayout;
  tab?: "dashboard" | "list";
  onTabChange?: (tab: "dashboard" | "list") => void;
}) {
  const [starting] = useState<WorkspaceResponse>(() =>
    initialLayout && !initial.config
      ? {
          ...initial,
          config: {
            version: 1,
            draft: newView(initialLayout),
            saved_views: [],
          },
        }
      : initial,
  );
  const workspace = useAutosavedWorkspace(
    starting,
    userId,
    editable,
    Boolean(sample),
  );
  const { config, setConfig } = workspace;
  const view = sample
    ? config.draft
    : { ...config.draft, layout: "operations" as const };
  const [localTab, setLocalTab] = useState<"dashboard" | "list">("dashboard");
  const activeTab = tab ?? localTab;
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [viewsOpen, setViewsOpen] = useState(false);
  const saved = config.saved_views.find((saved) => saved.id === view.id);
  const savedChanged = JSON.stringify(saved) !== JSON.stringify(view);
  const status = !editable
    ? "Read-only account · settings not saved"
    : workspace.pendingRecovery
      ? "Recovered draft available"
      : workspace.error
        ? "Save failed"
        : !workspace.valid
          ? "Complete settings to save"
          : workspace.saving
            ? "Saving…"
            : workspace.dirty
              ? "Unsaved changes…"
              : sample
                ? "Saved in this browser"
                : "All changes saved";
  const nameValid =
    name.trim().length > 0 &&
    new TextEncoder().encode(name.trim()).length <= 100;
  return (
    <div className="space-y-5 pb-6">
      <PageHeader
        title="Usage"
        description="Understand what you use, where credits go, and who drives spend."
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              onClick={() =>
                void Promise.all([
                  client.invalidateQueries({
                    queryKey: [
                      "usage-analytics",
                      useAuthStore.getState().user?.id,
                    ],
                  }),
                  client.invalidateQueries({ queryKey: ["admin", "usage"] }),
                ])
              }
            >
              <RefreshCw className="size-3" />
              Refresh
            </Button>
          </div>
        }
      />
      {import.meta.env.DEV &&
        import.meta.env.VITE_USAGE_SEEDED === "true" &&
        !sample && (
          <p className="text-[12px] text-muted-foreground" role="note">
            <strong className="font-medium text-foreground">
              Local seed data.
            </strong>{" "}
            Sample usage for reviewing analytics. No real charges.
          </p>
        )}
      {sample && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-info/25 bg-info/5 px-4 py-2.5 text-[11px] text-info">
          <span>
            <strong>Automated test fixture.</strong> Synthetic data for browser
            tests only.
          </span>
          <span>Interactive filters · charts · saved views</span>
        </div>
      )}
      <Tabs
        value={activeTab}
        onValueChange={(value) => {
          const next = value === "list" ? "list" : "dashboard";
          setLocalTab(next);
          onTabChange?.(next);
        }}
      >
        <TabsList aria-label="Usage view">
          <TabsTrigger value="dashboard">Dashboard</TabsTrigger>
          <TabsTrigger value="list">List</TabsTrigger>
        </TabsList>
        {sample && (
          <div
            className="mt-5 flex flex-wrap gap-2"
            aria-label="Analytics layout"
          >
            {LAYOUTS.map((layout) => {
              const Icon =
                layout === "overview"
                  ? ChartNoAxesCombined
                  : layout === "operations"
                    ? LayoutDashboard
                    : Search;
              return (
                <Button
                  type="button"
                  variant="outline"
                  key={layout}
                  aria-pressed={view.layout === layout}
                  disabled={Boolean(workspace.pendingRecovery)}
                  title={TEMPLATE_COPY[layout].description}
                  onClick={() => {
                    if (view.layout !== layout)
                      setConfig({
                        ...config,
                        draft: newView(layout, view.filters),
                      });
                  }}
                  className={cn(
                    "min-w-0",
                    view.layout === layout
                      ? "border-nyx-secondary-400/40 bg-nyx-secondary-400/10 text-foreground"
                      : "bg-card",
                  )}
                >
                  <div className="flex items-center gap-2">
                    <Icon
                      className={cn(
                        "size-4",
                        view.layout === layout
                          ? "text-nyx-secondary-400"
                          : "text-muted-foreground",
                      )}
                    />
                    <span className="text-[13px] font-semibold">
                      {TEMPLATE_COPY[layout].name}
                    </span>
                    <span className="sr-only">
                      {TEMPLATE_COPY[layout].reference}
                    </span>
                    {TEMPLATE_COPY[layout].recommended && (
                      <span className="rounded-md border border-nyx-secondary-400/30 bg-nyx-secondary-400/10 px-1.5 py-0.5 text-[10px] font-medium text-nyx-secondary-400">
                        Recommended
                      </span>
                    )}
                  </div>
                </Button>
              );
            })}
          </div>
        )}
        <div className="mt-5 flex flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-3">
            <h2 className="truncate text-[15px] font-semibold">
              {activeTab === "dashboard" ? view.name : "Usage records"}
            </h2>
            <span
              role="status"
              aria-label="Workspace save status"
              className={cn(
                "flex items-center gap-1.5 text-[10px]",
                workspace.error ? "text-warning" : "text-muted-foreground",
              )}
            >
              {!workspace.dirty &&
                !workspace.error &&
                !workspace.pendingRecovery && (
                  <Check className="size-3 text-success" />
                )}
              {status}
            </span>
          </div>
          {activeTab === "dashboard" && (
            <div className="flex flex-wrap gap-2">
              {saved && (
                <Button
                  variant="outline"
                  disabled={
                    !editable ||
                    !savedChanged ||
                    !workspace.valid ||
                    Boolean(workspace.pendingRecovery)
                  }
                  onClick={() =>
                    setConfig({
                      ...config,
                      saved_views: config.saved_views.map((item) =>
                        item.id === view.id ? structuredClone(view) : item,
                      ),
                    })
                  }
                >
                  <Save className="size-3" />
                  Update saved view
                </Button>
              )}
              <Popover open={viewsOpen} onOpenChange={setViewsOpen}>
                <PopoverTrigger asChild>
                  <Button
                    variant="outline"
                    disabled={Boolean(workspace.pendingRecovery)}
                  >
                    <Copy className="size-3" />
                    Saved views
                    <span className="ml-1 text-muted-foreground">
                      {config.saved_views.length}
                    </span>
                  </Button>
                </PopoverTrigger>
                <PopoverContent align="end" className="w-80 space-y-4 p-4">
                  <div>
                    <h3 className="text-[13px] font-semibold">
                      Your saved views
                    </h3>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      Private to your account. Exploring never overwrites a
                      named view.
                    </p>
                  </div>
                  <div className="max-h-64 space-y-1 overflow-auto">
                    {config.saved_views.length === 0 && (
                      <p className="py-3 text-[12px] text-muted-foreground">
                        Save a starting point to return to later.
                      </p>
                    )}
                    {config.saved_views.map((item) => (
                      <div key={item.id} className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          className="min-w-0 flex-1 justify-start"
                          onClick={() => {
                            setConfig({
                              ...config,
                              draft: structuredClone(item),
                            });
                            setViewsOpen(false);
                          }}
                        >
                          <span className="truncate">{item.name}</span>
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Delete saved view ${item.name}`}
                          disabled={!editable}
                          onClick={() =>
                            setConfig({
                              ...config,
                              saved_views: config.saved_views.filter(
                                (current) => current.id !== item.id,
                              ),
                            })
                          }
                        >
                          <Trash2 className="size-3 text-muted-foreground" />
                        </Button>
                      </div>
                    ))}
                  </div>
                  <div className="space-y-2 border-t border-border pt-3">
                    <Input
                      aria-label="New saved view name"
                      placeholder="e.g. Weekly platform spend"
                      value={name}
                      onChange={(e) => setName(e.target.value)}
                      maxLength={100}
                    />
                    <Button
                      className="w-full"
                      disabled={
                        !editable ||
                        !nameValid ||
                        !workspace.valid ||
                        config.saved_views.length >= 20
                      }
                      onClick={() => {
                        const copy = {
                          ...structuredClone(view),
                          id: crypto.randomUUID(),
                          name: name.trim(),
                        };
                        setConfig({
                          ...config,
                          draft: copy,
                          saved_views: [...config.saved_views, copy],
                        });
                        setName("");
                        setViewsOpen(false);
                      }}
                    >
                      <Save className="size-3" />
                      Save as new view
                    </Button>
                  </div>
                </PopoverContent>
              </Popover>
            </div>
          )}
        </div>
        {workspace.pendingRecovery && (
          <div
            role="status"
            aria-label="Recovered workspace"
            className="mt-3 space-y-3 rounded-lg border border-border bg-card p-3 text-[12px]"
          >
            <p>
              Showing your saved view. An older or incomplete draft is still
              available in this browser. Choose which version to use before
              editing further.
            </p>
            <div className="flex flex-wrap gap-2">
              <Button size="sm" onClick={() => void workspace.reload()}>
                Use saved view
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void workspace.reload(true)}
              >
                Restore recovered draft
              </Button>
            </div>
          </div>
        )}
        {workspace.error && (
          <div
            role="alert"
            className="flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/5 p-3 text-[12px]"
          >
            <p className="mr-auto">{workspace.error}</p>
            <Button size="sm" onClick={workspace.retry}>
              Retry
            </Button>
            <Button size="sm" onClick={() => void workspace.reload()}>
              Reload saved
            </Button>
            <Button size="sm" onClick={() => void workspace.reload(true)}>
              Keep my changes
            </Button>
          </div>
        )}
        {workspace.storageError && (
          <p role="alert" className="text-[11px] text-warning">
            Browser recovery storage is unavailable. Keep this page open until
            changes are saved.
          </p>
        )}
        {workspace.validationError && (
          <p role="alert" className="text-[11px] text-warning">
            {workspace.validationError}
          </p>
        )}
        <fieldset
          disabled={Boolean(workspace.pendingRecovery)}
          className="min-w-0"
        >
          <div className="mt-5">
            <FilterBar
              filters={view.filters}
              sample={options}
              onChange={(filters) =>
                setConfig({ ...config, draft: { ...view, filters } })
              }
            />
          </div>
          <TabsContent value="dashboard" className="mt-6">
            <AnalyticsCanvas
              view={view}
              disabled={Boolean(workspace.pendingRecovery)}
              onChange={(draft) => setConfig({ ...config, draft })}
              sample={sample}
            />
          </TabsContent>
          <TabsContent value="list" className="mt-6">
            <AdminUsageList filters={view.filters} />
          </TabsContent>
        </fieldset>
      </Tabs>
      <p className="text-[10px] leading-relaxed text-muted-foreground">
        Usage windows are UTC and end-exclusive. Gross cost includes wallet,
        grant, and allowance funding. Billing-account filters select who paid;
        acting-user filters select who made the request.
      </p>
    </div>
  );
}
