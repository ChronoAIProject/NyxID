import { useRef, useState, type ReactNode, type Ref } from "react";
import { ArrowLeftRight, Layers, UserRound, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  ServiceFilterMultiselect,
  type ServiceFilterOption,
} from "./service-filter-multiselect";
import { ServiceOwnerAvatar } from "./service-owner-avatar";
import { ServiceSavedViews } from "./service-saved-views";
import { DataTableSearch } from "@/components/data-table/data-table-controls";
import { DEFAULT_SERVICE_FILTERS } from "@/schemas/service-view";
import { cn } from "@/lib/utils";
import type { ServiceConnectionGroup } from "@/lib/service-groups";
import type { useServiceView } from "@/hooks/use-service-view";
import type { KeyInfo } from "@/types/keys";
const SOURCE_LABELS = {
  personal: "Personal",
  org: "Organization",
  platform: "NyxID platform",
};

export function ServiceViewToolbar({
  view,
  keys,
  groups,
  children,
  actions,
  ref,
  stuck = false,
}: {
  readonly view: ReturnType<typeof useServiceView>;
  readonly keys: readonly KeyInfo[];
  readonly groups: readonly ServiceConnectionGroup[];
  readonly children?: ReactNode;
  /** Primary actions (Add, refresh) kept in the filter row so they stay
   *  reachable while the toolbar is stuck. */
  readonly actions?: ReactNode | ((compact: boolean) => ReactNode);
  readonly ref?: Ref<HTMLDivElement>;
  readonly stuck?: boolean;
}) {
  const { filters, setFilters } = view;
  const inputRef = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const organizations: ServiceFilterOption[] = [
    ...new Map(
      keys.flatMap((key) => {
        const source = key.credential_source;
        return source?.type === "org" ? [[source.org_id, source] as const] : [];
      }),
    ).values(),
  ]
    .sort((a, b) => a.org_name.localeCompare(b.org_name))
    .map((source) => ({
      id: source.org_id,
      label: source.org_name,
      icon: (
        <ServiceOwnerAvatar
          type="org"
          name={source.org_name}
          avatarUrl={source.avatar_url}
        />
      ),
    }));
  const services: ServiceFilterOption[] = groups.map((group) => ({
    id: group.id,
    label: group.name,
  }));
  const selections = [
    ...filters.organization_ids.map((id) => ({
      id,
      field: "organization_ids" as const,
      prefix: "Org",
      label:
        organizations.find((option) => option.id === id)?.label ??
        "Unavailable organization",
      icon: organizations.find((option) => option.id === id)?.icon,
    })),
    ...filters.service_group_ids.map((id) => ({
      id,
      field: "service_group_ids" as const,
      prefix: "Service",
      label:
        services.find((option) => option.id === id)?.label ??
        "Unavailable service",
      icon: undefined,
    })),
  ];
  const applied = [
    ...(filters.source === "org" || filters.source === "platform"
      ? [
          {
            key: "source" as const,
            label: "Source",
            value: SOURCE_LABELS[filters.source],
          },
        ]
      : []),
    ...(filters.state !== "all"
      ? [
          {
            key: "state" as const,
            label: "Service state",
            value: filters.state === "enabled" ? "Enabled" : "Disabled",
          },
        ]
      : []),
    ...(filters.service_type !== "all"
      ? [
          {
            key: "service_type" as const,
            label: "Type",
            value: filters.service_type.toUpperCase(),
          },
        ]
      : []),
    ...(!filters.show_auto_connected
      ? [
          {
            key: "show_auto_connected" as const,
            label: "Auto-connected",
            value: "Hidden",
          },
        ]
      : []),
  ];
  const editSearch = () => {
    setDraft(filters.search);
    inputRef.current?.focus();
  };
  const clear = () => {
    setDraft(null);
    setFilters({
      ...DEFAULT_SERVICE_FILTERS,
      source: filters.source === "personal" ? "personal" : "all",
    });
  };
  const sourceToggle = (
    <TooltipProvider delayDuration={100} disableHoverableContent>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="outline"
            size={stuck ? "icon" : "default"}
            className="shrink-0 rounded-full"
            aria-label={`Service view: ${filters.source === "personal" ? "Personal" : "All services"}`}
            onClick={() =>
              setFilters({
                ...filters,
                source: filters.source === "personal" ? "all" : "personal",
                organization_ids:
                  filters.source === "personal" ? filters.organization_ids : [],
              })
            }
          >
            {filters.source === "personal" ? (
              <UserRound className="size-3.5" aria-hidden="true" />
            ) : (
              <Layers className="size-3.5" aria-hidden="true" />
            )}
            {!stuck && (
              <>
                {filters.source === "personal" ? "Personal" : "All services"}
                <ArrowLeftRight
                  className="ml-1 size-3 text-muted-foreground"
                  aria-hidden="true"
                />
              </>
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent
          side="bottom"
          align="end"
          sideOffset={8}
          collisionPadding={12}
          className="max-w-[min(18rem,calc(100vw-2rem))] space-y-1 text-left leading-relaxed"
        >
          <p className="font-medium">
            {filters.source === "personal"
              ? "Switch to all services"
              : "Switch to personal services"}
          </p>
          <p className="text-muted-foreground">
            {filters.source === "personal"
              ? "Showing services with a personal connection, including their accessible organization and platform connections."
              : "Showing all accessible services. Switch to keep only services with a personal connection and their counterparts."}
          </p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );

  return (
    <div
      ref={ref}
      role="region"
      aria-label="Service filters"
      data-stuck={stuck}
      className="service-filter-toolbar pointer-events-none sticky top-0 z-20"
    >
      <div
        className={cn(
          "pointer-events-auto relative rounded-xl border border-border/60 bg-card transition-shadow duration-200 motion-reduce:transition-none",
          stuck
            ? "border-x-transparent border-t-transparent shadow-[0_4px_8px_-4px_rgb(0_0_0/0.4),0_16px_32px_-16px_rgb(0_0_0/0.6)] light:shadow-[0_4px_8px_-4px_rgb(0_0_0/0.12),0_16px_32px_-16px_rgb(0_0_0/0.2)]"
            : "shadow-sm",
        )}
      >
        {!stuck && (
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/50 px-3 py-2">
            <ServiceSavedViews
              view={view}
              onRestore={() => {
                setDraft(null);
                view.restoreDefault();
              }}
            />
            {sourceToggle}
          </div>
        )}
        <div
          className={cn(
            "service-filter-controls flex flex-col gap-2.5 border-b border-border/60 p-3",
            stuck && "border-b-0",
          )}
        >
          <div
            className={cn(
              "flex items-center gap-2 p-0.5",
              stuck
                ? "flex-nowrap overflow-x-auto overscroll-x-contain [&>form]:min-w-48 [&>form]:w-auto"
                : "flex-wrap",
            )}
          >
            <div
              className={cn(
                "flex gap-1.5",
                stuck ? "shrink-0 flex-nowrap" : "w-full flex-wrap sm:w-auto",
              )}
            >
              <ServiceFilterMultiselect
                label="Organization"
                className={cn(
                  "grid-cols-[auto_minmax(0,1fr)_auto] gap-1.5 px-2.5 sm:w-44 md:h-8",
                  stuck && "w-44 shrink-0",
                )}
                plural="organizations"
                options={organizations}
                selected={filters.organization_ids}
                onChange={(organization_ids) =>
                  setFilters({
                    ...filters,
                    organization_ids,
                    source:
                      organization_ids.length && filters.source === "personal"
                        ? "all"
                        : filters.source,
                  })
                }
              />
              <ServiceFilterMultiselect
                label="Service"
                className={cn(
                  "grid-cols-[auto_minmax(0,1fr)_auto] gap-1.5 px-2.5 sm:w-40 md:h-8",
                  stuck && "w-40 shrink-0",
                )}
                plural="services"
                options={services}
                selected={filters.service_group_ids}
                onChange={(service_group_ids) =>
                  setFilters({ ...filters, service_group_ids })
                }
              />
            </div>
            <DataTableSearch
              fields={[]}
              selectedField={null}
              inputRef={inputRef}
              value={draft ?? filters.search}
              ariaLabel="Search services and connections"
              maxLength={200}
              onFieldChange={() => undefined}
              onValueChange={setDraft}
              onApply={() => {
                setFilters({
                  ...filters,
                  search: (draft ?? filters.search).trim(),
                });
                setDraft(null);
              }}
              onCancel={() => setDraft("")}
            />
            {actions && (
              <div className="ml-auto flex shrink-0 items-center gap-2">
                {typeof actions === "function" ? actions(stuck) : actions}
              </div>
            )}
            {stuck && sourceToggle}
          </div>
          {Boolean(selections.length || filters.search || applied.length) && (
            <div
              className="service-filter-pills"
              aria-label="Active service filters"
            >
              {selections.map((selection) => (
                <div
                  key={`${selection.field}:${selection.id}`}
                  className="flex max-w-full items-center gap-1.5 rounded-md border border-border/80 bg-muted/25 pl-2 text-xs"
                >
                  {selection.icon}
                  <span
                    className="min-w-0 truncate"
                    title={`${selection.prefix}: ${selection.label}`}
                  >
                    <span className="text-muted-foreground">
                      {selection.prefix}:
                    </span>{" "}
                    {selection.label}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${selection.prefix}: ${selection.label}`}
                    className="flex h-full w-11 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring md:w-7"
                    onClick={() =>
                      setFilters({
                        ...filters,
                        [selection.field]: filters[selection.field].filter(
                          (id) => id !== selection.id,
                        ),
                      })
                    }
                  >
                    <X className="size-3" aria-hidden="true" />
                  </button>
                </div>
              ))}
              {filters.search && (
                <div className="flex max-w-full items-center rounded-md border border-border/80 bg-muted/25 text-xs">
                  <button
                    type="button"
                    className="min-w-0 truncate px-2"
                    onClick={editSearch}
                    aria-label="Edit search"
                    title={filters.search}
                  >
                    <span className="text-muted-foreground">Search:</span>{" "}
                    {filters.search}
                  </button>
                  <button
                    type="button"
                    className="flex h-full w-11 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring md:w-7"
                    aria-label="Remove search"
                    onClick={() => {
                      setDraft(null);
                      setFilters({ ...filters, search: "" });
                    }}
                  >
                    <X className="size-3" aria-hidden="true" />
                  </button>
                </div>
              )}
              {applied.map(({ key, label, value }) => (
                <div
                  key={key}
                  className="flex max-w-full items-center gap-1.5 rounded-md border border-border/80 bg-muted/25 pl-2 text-xs"
                >
                  <span className="truncate">
                    <span className="text-muted-foreground">{label}:</span>{" "}
                    {value}
                  </span>
                  <button
                    type="button"
                    className="flex h-full w-11 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring md:w-7"
                    aria-label={`Remove ${label} filter`}
                    onClick={() =>
                      setFilters({
                        ...filters,
                        [key]:
                          key === "source"
                            ? "all"
                            : DEFAULT_SERVICE_FILTERS[key],
                      })
                    }
                  >
                    <X className="size-3" aria-hidden="true" />
                  </button>
                </div>
              ))}
              <Button
                variant="ghost"
                size="sm"
                className="text-muted-foreground"
                onClick={clear}
              >
                Clear filters
              </Button>
            </div>
          )}
        </div>

        {!stuck && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-2">
            {children}
          </div>
        )}

        {!stuck && view.saveError && (
          <p role="alert" className="px-3 pb-2 text-xs text-destructive">
            {view.saveError}
          </p>
        )}
      </div>
    </div>
  );
}
